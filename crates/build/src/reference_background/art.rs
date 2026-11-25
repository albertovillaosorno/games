// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
//
// Boundary-Contract:
// - Owns:
//   - Publication of selected reconstructed scene layers as canonical art.
// - Must-Not:
//   - Infer actor identity, execute ROMs or embed proprietary art.
// - Allows:
//   - Inputs: Verified graphics bundle, private observation, object groups.
//   - Outputs: Bounded canonical RGBA8 background and isolated sprites.
//   - Side effects: Atomic local bundle publication.
// - Split-When:
//   - Additional raster formats need distinct conversion strategies.
// - Merge-When:
//   - Scene diagnostics duplicate observation or pixel reconstruction.
// - Summary:
//   - Extract independent artwork from one verified original observation.
// - Description:
//   - Validates the complete scene before isolating caller-selected objects.
// - Usage:
//   - Supply exact scene files and named, disjoint object index groups.
// - Defaults:
//   - Reject ambiguous, invisible or unsupported sprite selections.

//! Bounded scene art publication for locally supplied reference observations.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::path::PathBuf;

use games_assets::{
    AssetFormat, AssetManifest, AssetRecord, ContentHash, ManifestLimits,
};
use games_importer::mame::Machine;
use games_importer::mame::background::Pixel;
use games_platform_storage::bundle::CanonicalBundle;
use games_platform_storage::staging::CanonicalStaging;

use super::{
    Arguments, GraphicsPayloads, Observation, SceneAssets, SpriteObservation,
    count_matches,
};
use crate::{USAGE, publication, text};

const RECORD_BYTES: usize = 16;
const TRANSPARENT: u16 = u16::MAX;

#[derive(Debug)]
struct Group {
    indices: BTreeSet<usize>,
    key: String,
}

/// Scene publication arguments; groups must be disjoint active objects.
#[derive(Debug)]
pub struct Art {
    groups: Vec<Group>,
    output: PathBuf,
    scene: Arguments,
}

impl Art {
    /// Parse the output parent, scene paths and named object groups.
    ///
    /// # Errors
    /// Rejects missing paths, invalid indices and overlapping selections.
    pub fn parse(mut args: Vec<OsString>) -> Result<Self, String> {
        if !(11..=42).contains(&args.len()) {
            return Err(String::from(USAGE));
        }
        let groups = args.split_off(10);
        let output = PathBuf::from(args.remove(0));
        let mut used = BTreeSet::new();
        let mut parsed = Vec::new();
        for value in groups {
            let (key, raw_indices) = text(&value)?
                .split_once(':')
                .ok_or_else(|| String::from("expected KEY:INDEX,INDEX"))?;
            let indices = parse_indices(raw_indices, &mut used)?;
            parsed.push(Group {
                indices,
                key: key.to_owned(),
            });
        }
        Ok(Self {
            groups: parsed,
            output,
            scene: Arguments::parse_scene(args)?,
        })
    }

    fn publish(
        &self,
        context: (Machine, &SceneAssets<'_>),
        observation: &mut Observation,
        bundle: &CanonicalBundle,
    ) -> Result<(), String> {
        let (machine, assets) = context;
        let mut pens =
            vec![Pixel::default(); machine.background_raster().pixels()];
        let limits = limits()?;
        let mut stage = CanonicalStaging::new(
            &self.output,
            publication::staging_budget(limits)?,
        )
        .map_err(|error| format!("stage: {error:?}"))?;
        let mut records = Vec::new();
        let sprites = observation
            .sprites
            .take()
            .ok_or_else(|| String::from("missing object observation"))?;
        observation.reconstruct(machine, assets, &mut pens)?;
        let (width, height) = machine.background_raster().extent();
        let background = Image::from_pens(
            &pens,
            (width, height),
            &observation.colors,
            false,
        )?;
        background.stage("scene/background", &mut records, &mut stage)?;
        for group in &self.groups {
            let actor =
                isolate((machine, assets), observation, &sprites, group)?;
            actor.stage(&group.key, &mut records, &mut stage)?;
        }
        let manifest = AssetManifest::new(
            bundle.manifest().source().clone(),
            records,
            limits,
        )
        .map_err(|error| format!("manifest: {error:?}"))?;
        publication::finish(stage, &manifest, limits)
    }

    /// Verify a scene and publish the selected layers independently.
    ///
    /// # Errors
    /// Rejects differences from the observed frame and any invalid art.
    pub fn run(&self, machine: Machine) -> Result<(), String> {
        let bundle = CanonicalBundle::open(
            &self.scene.parent,
            self.scene.digest,
            machine.graphics_limits(),
        )
        .map_err(|error| format!("graphics bundle: {error:?}"))?;
        machine
            .validate_manifest(bundle.manifest())
            .map_err(|error| format!("profile manifest: {error:?}"))?;
        let payloads = GraphicsPayloads::load(&bundle, true)?;
        let assets = payloads.bind(&bundle)?;
        let (mut observation, frame) = Observation::load(&self.scene, machine)?;
        let mut pens =
            vec![Pixel::default(); machine.background_raster().pixels()];
        observation.reconstruct(machine, &assets, &mut pens)?;
        if count_matches(&pens, &observation.colors, &frame)? != pens.len() {
            return Err(String::from("reference scene does not match exactly"));
        }
        self.publish((machine, &assets), &mut observation, &bundle)
    }
}

#[derive(Debug)]
struct Image {
    height: usize,
    pixels: Vec<[u8; 4]>,
    width: usize,
}

impl Image {
    fn crop(&self) -> Result<Self, String> {
        let bounds = self
            .pixels
            .chunks_exact(self.width)
            .enumerate()
            .flat_map(|(row, pixels)| {
                pixels
                    .iter()
                    .enumerate()
                    .filter(|(_, [_, _, _, alpha])| *alpha != 0)
                    .map(move |(column, _)| (column, row))
            })
            .fold(None, |bounds, (column, row)| {
                Some(bounds.map_or(
                    (column, row, column, row),
                    |(left, top, right, bottom): (
                        usize,
                        usize,
                        usize,
                        usize,
                    )| {
                        (
                            left.min(column),
                            top.min(row),
                            right.max(column),
                            bottom.max(row),
                        )
                    },
                ))
            });
        let (left, top, right, bottom) = bounds.ok_or_else(|| {
            String::from("selected objects have no visible art")
        })?;
        let width = right.saturating_sub(left).saturating_add(1);
        let height = bottom.saturating_sub(top).saturating_add(1);
        let mut pixels = Vec::new();
        for row in self.pixels.chunks_exact(self.width).skip(top).take(height) {
            let slice = row
                .get(left..=right)
                .ok_or_else(|| String::from("crop outside raster"))?;
            pixels.extend_from_slice(slice);
        }
        Ok(Self { height, pixels, width })
    }

    fn from_pens(
        pens: &[Pixel],
        extent: (usize, usize),
        colors: &[[u8; 4]],
        transparent: bool,
    ) -> Result<Self, String> {
        let (width, height) = extent;
        let pixels = pens
            .iter()
            .map(|pen| {
                if transparent && pen.pen() == TRANSPARENT {
                    Some([0; 4])
                } else {
                    pen.rgba(colors)
                }
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| String::from("pen outside palette"))?;
        Ok(Self { height, pixels, width })
    }

    fn stage(
        self,
        key: &str,
        records: &mut Vec<AssetRecord>,
        stage: &mut CanonicalStaging,
    ) -> Result<(), String> {
        let dimension = |value| {
            u32::try_from(value)
                .ok()
                .and_then(NonZeroU32::new)
                .ok_or_else(|| String::from("invalid art dimensions"))
        };
        let bytes = self.pixels.as_flattened();
        let digest = ContentHash::calculate(bytes);
        if !records.iter().any(|record| record.digest == digest) {
            stage
                .write_payload(digest, bytes)
                .map_err(|error| format!("art payload: {error:?}"))?;
        }
        records.push(AssetRecord {
            digest,
            format: AssetFormat::Rgba8 {
                height: dimension(self.height)?,
                width: dimension(self.width)?,
            },
            key: key.to_owned(),
            size: u64::try_from(bytes.len())
                .map_err(|error| error.to_string())?,
        });
        Ok(())
    }
}

fn isolate(
    context: (Machine, &SceneAssets<'_>),
    observation: &Observation,
    sprites: &SpriteObservation,
    group: &Group,
) -> Result<Image, String> {
    let (machine, assets) = context;
    let ram = select(&sprites.ram, &group.indices)?;
    let (_, mixer_raw) = observation
        .registers
        .split_first_chunk::<6>()
        .ok_or_else(|| String::from("missing controls"))?;
    let mixer = machine
        .decode_video_priority(mixer_raw)
        .map_err(|error| format!("mixer: {error:?}"))?;
    let tiles = assets
        .sprites
        .as_ref()
        .ok_or_else(|| String::from("missing sprite tiles"))?;
    let raster = machine.background_raster();
    let mut pixels = vec![Pixel::from_pen(TRANSPARENT); raster.pixels()];
    SpriteObservation {
        controls: sprites.controls.clone(),
        ram,
    }
    .overlay(machine, (tiles, mixer), &mut pixels)?;
    let (width, height) = raster.extent();
    Image::from_pens(&pixels, (width, height), &observation.colors, true)?
        .crop()
}

fn limits() -> Result<ManifestLimits, String> {
    let size = |value| {
        NonZeroUsize::new(value).ok_or_else(|| String::from("zero limit"))
    };
    let bytes = |value| {
        NonZeroU64::new(value).ok_or_else(|| String::from("zero limit"))
    };
    Ok(ManifestLimits {
        manifest_bytes: size(32_768)?,
        payload_bytes: bytes(1_048_576)?,
        records: size(33)?,
        total_bytes: bytes(33_554_432)?,
    })
}

fn parse_indices(
    value: &str,
    used: &mut BTreeSet<usize>,
) -> Result<BTreeSet<usize>, String> {
    let mut selected = BTreeSet::new();
    for token in value.split(',') {
        let index =
            token.parse::<usize>().map_err(|error| error.to_string())?;
        if index >= 256 || !used.insert(index) {
            return Err(String::from("invalid or overlapping object index"));
        }
        let _inserted = selected.insert(index);
    }
    Ok(selected)
}

fn select(ram: &[u8], indices: &BTreeSet<usize>) -> Result<Vec<u8>, String> {
    let mut selected = vec![0; ram.len()];
    for index in indices {
        let start = index
            .checked_mul(RECORD_BYTES)
            .ok_or_else(|| String::from("object index overflow"))?;
        let end = start.saturating_add(RECORD_BYTES);
        let object = ram
            .get(start..end)
            .ok_or_else(|| String::from("missing object record"))?;
        if object.get(1).is_none_or(|flags| flags & 128 == 0) {
            return Err(String::from("selected object is inactive"));
        }
        selected
            .get_mut(start..end)
            .ok_or_else(|| String::from("missing output record"))?
            .copy_from_slice(object);
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_only_active_named_objects() -> Result<(), String> {
        let mut used = BTreeSet::new();
        let group = parse_indices("0,2", &mut used)?;
        if parse_indices("2", &mut used).is_ok() {
            return Err(String::from("overlap admitted"));
        }
        let mut source = vec![128; 48];
        let result = select(&source, &group)?;
        if result.get(16..32) != Some([0; 16].as_slice())
            || result.get(32..48) != Some([128; 16].as_slice())
        {
            return Err(String::from("wrong object selection"));
        }
        *source.get_mut(33).ok_or_else(|| String::from("fixture"))? = 0;
        select(&source, &group)
            .is_err()
            .then_some(())
            .ok_or_else(|| String::from("inactive object admitted"))
    }

    #[test]
    fn crop_preserves_visible_pixels_and_alpha() -> Result<(), String> {
        let mut pixels = vec![[0; 4]; 16];
        *pixels.get_mut(5).ok_or_else(|| String::from("fixture"))? =
            [1, 2, 3, 255];
        *pixels.get_mut(10).ok_or_else(|| String::from("fixture"))? =
            [4, 5, 6, 255];
        let cropped = Image {
            height: 4,
            pixels,
            width: 4,
        }
        .crop()?;
        ((cropped.width, cropped.height) == (2, 2)
            && cropped.pixels
                == [[1, 2, 3, 255], [0; 4], [0; 4], [4, 5, 6, 255]])
        .then_some(())
        .ok_or_else(|| String::from("wrong crop"))
    }
}
