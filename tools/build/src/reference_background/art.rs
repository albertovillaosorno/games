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
use std::ffi::{OsStr, OsString};
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};

use games_assets::{
    AssetFormat, AssetManifest, AssetRecord, ContentHash, ManifestLimits,
};
use games_platform::ir::importer::reference::{Machine, Pixel};
use games_platform::ir::storage::bundle::CanonicalBundle;
use games_platform::ir::storage::staging::CanonicalStaging;
use games_platform::ir::toolchain;

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
    output: OsString,
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
        let output = args.remove(0);
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
        let mut placements = Vec::with_capacity(self.groups.len());
        for group in &self.groups {
            let Crop {
                image: actor,
                origin: (left, top),
            } = isolate((machine, assets), observation, &sprites, group)?;
            placements.push((&group.key, left, top));
            actor.stage(&group.key, &mut records, &mut stage)?;
        }
        let manifest = AssetManifest::new(
            bundle.manifest().source().clone(),
            records,
            limits,
        )
        .map_err(|error| format!("manifest: {error:?}"))?;
        publication::finish(stage, &manifest, limits)?;
        for (key, left, top) in placements {
            toolchain::write_line(&format!("Crop origin {key}: {left},{top}"))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Verify a scene and publish the selected layers independently.
    ///
    /// # Errors
    /// Rejects differences from the observed frame and any invalid art.
    pub fn run(&self, machine: Machine) -> Result<(), String> {
        publication::require_asset_parent(
            &self.output,
            &machine.identity().title,
        )?;
        let bundle = graphics_bundle(&self.scene, machine)?;
        let payloads = GraphicsPayloads::load(&bundle, true)?;
        let assets = payloads.bind(&bundle)?;
        let mut pens =
            vec![Pixel::default(); machine.background_raster().pixels()];
        let mut observation =
            verified_scene(&self.scene, machine, &assets, &mut pens)?;
        self.publish((machine, &assets), &mut observation, &bundle)
    }
}

/// Publish one cropped background from a completely verified scene.
#[derive(Debug)]
pub struct BackgroundPublication {
    output: OsString,
    scene: Arguments,
    top: usize,
}

impl BackgroundPublication {
    /// Parse the output, complete scene and top exclusion.
    /// # Errors
    /// Rejects incomplete observations and malformed exclusion arguments.
    pub fn parse(mut args: Vec<OsString>) -> Result<Self, String> {
        if args.len() != 11 {
            return Err(String::from(USAGE));
        }
        let raw_top = args.pop().ok_or_else(|| String::from(USAGE))?;
        let top = text(&raw_top)?
            .parse()
            .map_err(|_error| String::from("invalid top exclusion"))?;
        let output = args.remove(0);
        Ok(Self {
            output,
            scene: Arguments::parse_scene(args)?,
            top,
        })
    }

    /// Publish the sprite-free background only after exact comparison.
    /// # Errors
    /// Rejects misaligned observations, invalid crops or publication failure.
    pub fn run(&self, machine: Machine) -> Result<(), String> {
        publication::require_asset_parent(
            &self.output,
            &machine.identity().title,
        )?;
        let bundle = graphics_bundle(&self.scene, machine)?;
        let payloads = GraphicsPayloads::load(&bundle, true)?;
        let assets = payloads.bind(&bundle)?;
        let mut pens =
            vec![Pixel::default(); machine.background_raster().pixels()];
        let observation =
            verified_scene(&self.scene, machine, &assets, &mut pens)?;
        let image = background_image(
            (machine, &assets),
            observation,
            &mut pens,
            self.top,
        )?;
        publish_image(
            &self.output,
            &bundle,
            image,
            ("scene/background", limits()?),
        )
    }
}

/// Bounded adjacent scene observations for a continuous background strip.
#[derive(Debug)]
pub struct Strip {
    output: OsString,
    scenes: Vec<Arguments>,
    side: usize,
    top: usize,
}

impl Strip {
    /// Parse output, bundle, HUD rows, black side margins and scene groups.
    /// # Errors
    /// Rejects invalid margins or incomplete/excessive observations.
    pub fn parse(args: &[OsString]) -> Result<Self, String> {
        let (header, paths) = args
            .split_first_chunk::<5>()
            .ok_or_else(|| String::from(USAGE))?;
        let [output, parent, digest, top_arg, side_arg] = header;
        let top = text(top_arg)?
            .parse::<usize>()
            .map_err(|_error| String::from("invalid top exclusion"))?;
        let side = text(side_arg)?
            .parse::<usize>()
            .map_err(|_error| String::from("invalid side exclusion"))?;
        let (groups, remainder) = paths.as_chunks::<7>();
        if !remainder.is_empty() || !(2..=8).contains(&groups.len()) {
            return Err(String::from("expected 2 to 8 complete scenes"));
        }
        let scenes = groups
            .iter()
            .map(|group| {
                Arguments::parse_scene(
                    [parent.clone(), digest.clone()]
                        .into_iter()
                        .chain(group.iter().cloned())
                        .collect(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            output: output.clone(),
            scenes,
            side,
            top,
        })
    }

    /// Verify every visible scene and combine exact overlapping backgrounds.
    /// # Errors
    /// Rejects inconsistent scroll, differing overlap or invalid assets.
    pub fn run(&self, machine: Machine) -> Result<(), String> {
        publication::require_asset_parent(
            &self.output,
            &machine.identity().title,
        )?;
        let first = self.scenes.first().ok_or_else(|| String::from(USAGE))?;
        let bundle = graphics_bundle(first, machine)?;
        let payloads = GraphicsPayloads::load(&bundle, true)?;
        let assets = payloads.bind(&bundle)?;
        let raster = machine.background_raster();
        let mut pens = vec![Pixel::default(); raster.pixels()];
        let mut previous: Option<Observation> = None;
        let mut strip: Option<Image> = None;
        for paths in &self.scenes {
            let mut observation =
                verified_scene(paths, machine, &assets, &mut pens)?;
            let _objects = observation.sprites.take();
            observation.reconstruct(machine, &assets, &mut pens)?;
            let image = Image::from_pens(
                &pens,
                raster.extent(),
                &observation.colors,
                false,
            )?
            .without_top(self.top)?
            .without_sides(self.side)?;
            strip = Some(match (strip, &previous) {
                (Some(left), Some(prior)) => {
                    let shift = prior.rightward_shift(&observation, machine)?;
                    left.join_right(&image, shift)?
                },
                (None, None) => image,
                _ => return Err(String::from("incomplete strip state")),
            });
            previous = Some(observation);
        }
        let completed = strip.ok_or_else(|| String::from("empty strip"))?;
        publish_image(
            &self.output,
            &bundle,
            completed,
            ("scene/world-background", strip_limits()?),
        )
    }
}

#[derive(Debug)]
struct Image {
    height: usize,
    pixels: Vec<[u8; 4]>,
    width: usize,
}

#[derive(Debug)]
struct Crop {
    image: Image,
    origin: (usize, usize),
}

impl Image {
    fn crop(&self) -> Result<Crop, String> {
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
        Ok(Crop {
            image: Self { height, pixels, width },
            origin: (left, top),
        })
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
                if transparent {
                    pen.isolated_rgba(colors, TRANSPARENT)
                } else {
                    pen.rgba(colors)
                }
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| String::from("pen outside palette"))?;
        Ok(Self { height, pixels, width })
    }

    fn join_right(self, right: &Self, shift: usize) -> Result<Self, String> {
        let invalid = || String::from("invalid background overlap");
        if self.height != right.height
            || right.width > self.width
            || shift == 0
            || shift >= right.width
        {
            return Err(invalid());
        }
        let overlap = right.width.saturating_sub(shift);
        let width = self.width.checked_add(shift).ok_or_else(invalid)?;
        let pixels = width.checked_mul(self.height).ok_or_else(invalid)?;
        let bytes = pixels.checked_mul(4).ok_or_else(invalid)?;
        let limit = strip_limits()?.payload_bytes.get();
        if u64::try_from(bytes).map_or(true, |len| len > limit) {
            return Err(String::from("background strip exceeds byte budget"));
        }
        let mut combined = Vec::new();
        combined
            .try_reserve_exact(pixels)
            .map_err(|_error| String::from("strip allocation failed"))?;
        for (left_row, right_row) in self
            .pixels
            .chunks_exact(self.width)
            .zip(right.pixels.chunks_exact(right.width))
        {
            let tail = left_row.get(self.width.saturating_sub(overlap)..);
            let head = right_row.get(..overlap);
            if tail != head || tail.is_none() {
                return Err(String::from("background overlap differs"));
            }
            combined.extend_from_slice(left_row);
            combined.extend_from_slice(
                right_row.get(overlap..).ok_or_else(invalid)?,
            );
        }
        if combined.len() != pixels {
            return Err(invalid());
        }
        Ok(Self {
            height: self.height,
            pixels: combined,
            width,
        })
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

    fn without_sides(self, side: usize) -> Result<Self, String> {
        if side == 0 {
            return Ok(self);
        }
        let invalid = || String::from("invalid side exclusion");
        let end = self.width.checked_sub(side).ok_or_else(invalid)?;
        let width = end
            .checked_sub(side)
            .filter(|w| *w > 0)
            .ok_or_else(invalid)?;
        let mut pixels = Vec::new();
        for row in self.pixels.chunks_exact(self.width) {
            let left = row.get(..side).ok_or_else(invalid)?;
            let right = row.get(end..).ok_or_else(invalid)?;
            if left
                .iter()
                .chain(right)
                .any(|pixel| *pixel != [0, 0, 0, 255])
            {
                return Err(String::from("side margins are not opaque black"));
            }
            pixels.extend_from_slice(row.get(side..end).ok_or_else(invalid)?);
        }
        Ok(Self {
            height: self.height,
            pixels,
            width,
        })
    }

    fn without_top(mut self, top: usize) -> Result<Self, String> {
        if top >= self.height {
            return Err(String::from("top exclusion removes entire scene"));
        }
        let skip = top.saturating_mul(self.width);
        drop(self.pixels.drain(..skip));
        self.height = self.height.saturating_sub(top);
        Ok(self)
    }
}

pub(super) fn report_background(
    context: (Machine, &SceneAssets<'_>),
    observation: Observation,
    pens: &mut [Pixel],
    top: usize,
) -> Result<(), String> {
    let image = background_image(context, observation, pens, top)?;
    toolchain::write_line(&format!(
        "Verified background: {}x{} RGBA8 {}",
        image.width,
        image.height,
        ContentHash::calculate(image.pixels.as_flattened()),
    ))
    .map_err(|error| error.to_string())
}

fn background_image(
    (machine, assets): (Machine, &SceneAssets<'_>),
    mut observation: Observation,
    pens: &mut [Pixel],
    top: usize,
) -> Result<Image, String> {
    let _sprites = observation.sprites.take();
    observation.reconstruct(machine, assets, pens)?;
    Image::from_pens(
        pens,
        machine.background_raster().extent(),
        &observation.colors,
        false,
    )?
    .without_top(top)
}

fn graphics_bundle(
    paths: &Arguments,
    machine: Machine,
) -> Result<CanonicalBundle, String> {
    let bundle = CanonicalBundle::open(
        &paths.parent,
        paths.digest,
        machine.graphics_limits(),
    )
    .map_err(|error| format!("graphics bundle: {error:?}"))?;
    machine
        .validate_manifest(bundle.manifest())
        .map_err(|error| format!("profile manifest: {error:?}"))?;
    Ok(bundle)
}

fn isolate(
    context: (Machine, &SceneAssets<'_>),
    observation: &Observation,
    sprites: &SpriteObservation,
    group: &Group,
) -> Result<Crop, String> {
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

fn strip_limits() -> Result<ManifestLimits, String> {
    let bytes = NonZeroU64::new(2_097_152)
        .ok_or_else(|| String::from("zero strip limit"))?;
    Ok(ManifestLimits {
        payload_bytes: bytes,
        records: NonZeroUsize::MIN,
        total_bytes: bytes,
        ..limits()?
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

fn publish_image(
    output: &OsStr,
    bundle: &CanonicalBundle,
    image: Image,
    (key, limits): (&str, ManifestLimits),
) -> Result<(), String> {
    let mut stage =
        CanonicalStaging::new(output, publication::staging_budget(limits)?)
            .map_err(|error| format!("stage: {error:?}"))?;
    let mut records = Vec::new();
    image.stage(key, &mut records, &mut stage)?;
    let manifest =
        AssetManifest::new(bundle.manifest().source().clone(), records, limits)
            .map_err(|error| format!("manifest: {error:?}"))?;
    publication::finish(stage, &manifest, limits)
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

fn verified_scene(
    paths: &Arguments,
    machine: Machine,
    assets: &SceneAssets<'_>,
    pens: &mut [Pixel],
) -> Result<Observation, String> {
    let (observation, frame) = Observation::load(paths, machine)?;
    observation.reconstruct(machine, assets, pens)?;
    if count_matches(pens, &observation.colors, &frame)? != pens.len() {
        return Err(String::from("reference scene does not match exactly"));
    }
    Ok(observation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_publication_requires_complete_scene_and_crop() {
        let digest = "0".repeat(64);
        let mut args = [
            "output",
            "parent",
            &digest,
            "ram",
            "registers",
            "scroll",
            "palette",
            "frame",
            "objects",
            "controls",
        ]
        .map(OsString::from)
        .to_vec();
        assert!(BackgroundPublication::parse(args.clone()).is_err());
        args.push(OsString::from("28"));
        assert_eq!(
            BackgroundPublication::parse(args.clone()).map(|value| value.top),
            Ok(28)
        );
        args.push(OsString::from("unexpected"));
        assert!(BackgroundPublication::parse(args.clone()).is_err());
        let _extra = args.pop();
        let _top = args.pop();
        args.push(OsString::from("-1"));
        assert!(BackgroundPublication::parse(args).is_err());
    }

    #[test]
    fn background_digest_excludes_only_explicit_top_rows() -> Result<(), String>
    {
        let colors = [[1, 2, 3, 255], [4, 5, 6, 255]];
        let image = |hud| {
            Image::from_pens(
                &[hud, hud, Pixel::from_pen(0), Pixel::from_pen(1)],
                (2, 2),
                &colors,
                false,
            )
        };
        let first = image(Pixel::from_pen(0))?.without_top(1)?;
        let second = image(Pixel::from_pen(1))?.without_top(1)?;
        let expected = ContentHash::calculate(colors.as_flattened());
        ((first.width, first.height) == (2, 1)
            && ContentHash::calculate(first.pixels.as_flattened()) == expected
            && ContentHash::calculate(second.pixels.as_flattened()) == expected
            && image(Pixel::from_pen(0))?.without_top(2).is_err())
        .then_some(())
        .ok_or_else(|| String::from("incorrect background digest crop"))
    }

    #[test]
    fn strip_requires_exact_overlap_and_retains_unique_pixels()
    -> Result<(), String> {
        let image = |pixels: &[u8], width, height| Image {
            height,
            pixels: pixels.iter().map(|&value| [value, 0, 0, 255]).collect(),
            width,
        };
        let left = || image(&[0, 1, 2, 3, 4, 5, 6, 7, 8], 3, 3);
        let right = || image(&[9, 9, 9, 4, 5, 9, 7, 8, 10], 3, 3);
        let result = left()
            .without_top(1)?
            .join_right(&right().without_top(1)?, 1)?;
        let expected = image(&[3, 4, 5, 9, 6, 7, 8, 10], 4, 2);
        if (result.width, result.height) != (4, 2)
            || result.pixels != expected.pixels
            || left().join_right(&right(), 1).is_ok()
            || left().join_right(&right(), 0).is_ok()
            || left().join_right(&right(), 3).is_ok()
            || left().without_top(3).is_ok()
        {
            return Err(String::from("invalid strip overlap behavior"));
        }
        let next = image(&[5, 9, 11, 8, 10, 12], 3, 2);
        let extended = result.join_right(&next, 1)?;
        let final_expected = image(&[3, 4, 5, 9, 11, 6, 7, 8, 10, 12], 5, 2);
        if extended.pixels != final_expected.pixels {
            return Err(String::from("multi-scene overlap differs"));
        }
        Ok(())
    }

    #[test]
    fn strip_budget_admits_longer_art_without_relaxing_actor_limits()
    -> Result<(), String> {
        let image = |width: usize| Image {
            height: 196,
            pixels: vec![[0, 0, 0, 255]; width * 196],
            width,
        };
        let right = image(288);
        let combined = image(1290).join_right(&right, 188)?;
        if combined.width != 1478
            || combined.pixels.as_flattened().len() <= 1_048_576
            || image(2600).join_right(&right, 100).is_ok()
            || limits()?.payload_bytes.get() != 1_048_576
            || strip_limits()?.records.get() != 1
        {
            return Err(String::from("incorrect background strip budget"));
        }
        Ok(())
    }

    #[test]
    fn strip_excludes_only_opaque_black_side_margins() -> Result<(), String> {
        let image = |edge| Image {
            height: 1,
            pixels: vec![edge, [1, 2, 3, 255], [0, 0, 0, 255]],
            width: 3,
        };
        let black = [0, 0, 0, 255];
        let inner = image(black).without_sides(1)?;
        if (inner.width, inner.height) != (1, 1)
            || inner.pixels != [[1, 2, 3, 255]]
            || image([0, 0, 0, 0]).without_sides(1).is_ok()
            || image([1, 0, 0, 255]).without_sides(1).is_ok()
            || image(black).without_sides(2).is_ok()
            || image(black).without_sides(usize::MAX).is_ok()
        {
            return Err(String::from("invalid side margin behavior"));
        }
        Ok(())
    }

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
        let Crop { image: cropped, origin } = Image {
            height: 4,
            pixels,
            width: 4,
        }
        .crop()?;
        (origin == (1, 1)
            && (cropped.width, cropped.height) == (2, 2)
            && cropped.pixels
                == [[1, 2, 3, 255], [0; 4], [0; 4], [4, 5, 6, 255]])
        .then_some(())
        .ok_or_else(|| String::from("wrong crop"))
    }
}
