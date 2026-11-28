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

#[cfg(feature = "native-preview")]
use games::presentation::{Dimensions, Viewport};
use games_assets::{
    AssetFormat, AssetManifest, AssetRecord, ContentHash, ManifestLimits,
};
#[cfg(feature = "native-preview")]
use games_platform::ir::frame::{FrameLayout, RenderError};
#[cfg(feature = "native-preview")]
use games_platform::ir::graphics::NativeWindow;
#[cfg(feature = "native-preview")]
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::importer::reference::{
    Background, Machine, Pixel, Raster,
};
#[cfg(feature = "native-preview")]
use games_platform::ir::scene::{SceneLimits, SpriteDraw};
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
const COMPLETE_OBJECTS: &str = "--complete-objects";
const DEFAULT_STRIP_SCENES: usize = 8;

#[derive(Debug)]
struct Group {
    complete: bool,
    indices: BTreeSet<usize>,
    key: String,
}

/// Report registered actor crops without publishing any original art.
#[derive(Debug)]
pub struct ActorDigests {
    groups: Vec<Group>,
    scene: Arguments,
}

impl ActorDigests {
    /// Parse complete scene paths and named object groups.
    ///
    /// # Errors
    /// Rejects incomplete observations and invalid or overlapping selections.
    pub fn parse(mut args: Vec<OsString>) -> Result<Self, String> {
        if !(10..=42).contains(&args.len()) {
            return Err(String::from(USAGE));
        }
        let groups = parse_selection(args.split_off(9))?;
        Ok(Self {
            groups,
            scene: Arguments::parse_scene(args)?,
        })
    }

    /// Verify the observed scene and report selected actor crop identities.
    ///
    /// # Errors
    /// Rejects misaligned observations and invalid or invisible selections.
    pub fn run(&self, machine: Machine) -> Result<(), String> {
        let bundle = graphics_bundle(&self.scene, machine)?;
        let payloads = GraphicsPayloads::load(&bundle, true)?;
        let assets = payloads.bind(&bundle)?;
        let mut pens =
            vec![Pixel::default(); machine.background_raster().pixels()];
        let observation =
            verified_scene(&self.scene, machine, &assets, &mut pens)?;
        let sprites = observation
            .sprites
            .as_ref()
            .ok_or_else(|| String::from("missing object observation"))?;
        let mut lines = Vec::with_capacity(self.groups.len());
        for group in &self.groups {
            let crop =
                isolate((machine, &assets), &observation, sprites, group)?;
            let digest =
                ContentHash::calculate(crop.image.pixels.as_flattened());
            let (left, top) = crop.origin;
            let (width, height) = (crop.image.width, crop.image.height);
            lines.push(format!(
                "{}: {width}x{height} at {left},{top} RGBA8 {digest}",
                group.key
            ));
        }
        for line in lines {
            toolchain::write_line(&line).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
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
        if !(11..=43).contains(&args.len()) {
            return Err(String::from(USAGE));
        }
        let groups = parse_selection(args.split_off(10))?;
        let output = args.remove(0);
        Ok(Self {
            groups,
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
    selection: BackgroundSelection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BackgroundSelection {
    pub(super) omitted_layer: Option<usize>,
    pub(super) top: usize,
}

impl BackgroundSelection {
    pub(super) fn parse(args: &mut Vec<OsString>) -> Result<Self, String> {
        let omitted_layer = match args.as_slice() {
            [.., flag, index] if flag == "--omit-layer" => {
                let layer = text(index)?
                    .parse::<usize>()
                    .map_err(|_error| String::from("invalid omitted layer"))?;
                if layer >= 3 {
                    return Err(String::from("invalid omitted layer"));
                }
                args.truncate(args.len().saturating_sub(2));
                Some(layer)
            },
            _ => None,
        };
        let raw_top = args.pop().ok_or_else(|| String::from(USAGE))?;
        let top = text(&raw_top)?
            .parse()
            .map_err(|_error| String::from("invalid top exclusion"))?;
        Ok(Self { omitted_layer, top })
    }
}

impl BackgroundPublication {
    /// Parse the output, complete scene and top exclusion.
    /// # Errors
    /// Rejects incomplete observations and malformed exclusion arguments.
    pub fn parse(mut args: Vec<OsString>) -> Result<Self, String> {
        let selection = BackgroundSelection::parse(&mut args)?;
        if args.len() != 10 {
            return Err(String::from(USAGE));
        }
        let output = args.remove(0);
        Ok(Self {
            output,
            scene: Arguments::parse_scene(args)?,
            selection,
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
            self.selection,
        )?;
        publish_image(
            &self.output,
            &bundle,
            &image,
            ("scene/background", limits()?),
        )
    }
}

/// Publish independent registered maps from one exact scene observation.
#[derive(Debug)]
pub struct LayerPublication {
    output: OsString,
    scene: Arguments,
}

impl LayerPublication {
    /// Parse an explicit output parent and one complete observation.
    ///
    /// # Errors
    /// Rejects incomplete or excessive scene arguments.
    pub fn parse(mut args: Vec<OsString>) -> Result<Self, String> {
        if args.len() != 10 {
            return Err(String::from(USAGE));
        }
        let output = args.remove(0);
        Ok(Self {
            output,
            scene: Arguments::parse_scene(args)?,
        })
    }

    /// Publish only after full-scene and isolated composition verification.
    ///
    /// # Errors
    /// Rejects misaligned observations, unsupported layers and unsafe output.
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
        let layers =
            registered_layers((machine, &assets), &mut observation, &mut pens)?;
        layers.publish(&self.output, &bundle, &observation.colors)
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
    /// Parse output, bundle, crop, optional scene budget and scene groups.
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
        let (limit, scene_paths) = match paths {
            [flag, value, rest @ ..] if flag == "--scene-limit" => {
                let limit = text(value)?
                    .parse::<usize>()
                    .map_err(|_error| String::from("invalid scene limit"))?;
                (limit, rest)
            },
            _ => (DEFAULT_STRIP_SCENES, paths),
        };
        let (groups, remainder) = scene_paths.as_chunks::<7>();
        if !remainder.is_empty() || !(2..=limit).contains(&groups.len()) {
            return Err(format!("expected 2 to {limit} complete scenes"));
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
            &completed,
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
    origin: (i32, i32),
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
            origin: (
                i32::try_from(left).map_err(|error| error.to_string())?,
                i32::try_from(top).map_err(|error| error.to_string())?,
            ),
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
        &self,
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

#[derive(Debug)]
struct RegisteredLayer {
    image: Image,
    index: usize,
    origin: (usize, usize),
}

#[derive(Debug)]
pub(super) struct RegisteredLayers {
    backdrop: u16,
    layers: Vec<RegisteredLayer>,
}

impl RegisteredLayers {
    fn backdrop_rgba(&self, colors: &[[u8; 4]]) -> Result<[u8; 4], String> {
        Pixel::from_pen(self.backdrop)
            .rgba(colors)
            .ok_or_else(|| String::from("backdrop pen outside palette"))
    }

    #[cfg(feature = "native-preview")]
    pub(super) fn native_check(
        &self,
        raster: Raster,
        pens: &[Pixel],
        colors: &[[u8; 4]],
    ) -> Result<(), String> {
        let expected = Image::from_pens(pens, raster.extent(), colors, false)?;
        let layout =
            super::native_layout(raster, expected.pixels.as_flattened().len())?;
        let limits = self.native_limits()?;
        let [red, green, blue, _alpha] = self.backdrop_rgba(colors)?;
        let mut window = NativeWindow::open(
            "Independent layer verification",
            layout.width(),
            layout.height(),
            true,
        )
        .map_err(|error| format!("native window: {error:?}"))?;
        let renderer = window.renderer_name();
        window
            .with_scene(layout, limits, |scene| {
                let mut commands = Vec::with_capacity(limits.sprites.get());
                for layer in &self.layers {
                    commands.extend(layer.commands(scene, layout)?);
                }
                if !scene.present([red, green, blue], &commands)?
                    || scene.readback()? != expected.pixels.as_flattened()
                {
                    return Err(RenderError::Backend(String::from(
                        "independent layer readback differs",
                    )));
                }
                Ok(())
            })
            .map_err(|error| format!("native scene: {error:?}"))?;
        toolchain::write_line(&format!(
            "Native independent layer readback matches ({renderer})"
        ))
        .map_err(|error| error.to_string())
    }

    #[cfg(feature = "native-preview")]
    fn native_limits(&self) -> Result<SceneLimits, String> {
        let size = |value| {
            NonZeroUsize::new(value)
                .ok_or_else(|| String::from("empty independent layer budget"))
        };
        let bytes = self.layers.iter().fold(0usize, |sum, layer| {
            sum.saturating_add(layer.image.pixels.as_flattened().len())
        });
        Ok(SceneLimits {
            atlas_bytes: size(bytes)?,
            atlases: size(self.layers.len())?,
            sprites: size(self.layers.len().saturating_mul(4))?,
        })
    }

    fn publish(
        &self,
        output: &OsStr,
        bundle: &CanonicalBundle,
        colors: &[[u8; 4]],
    ) -> Result<(), String> {
        let limits = layer_limits()?;
        let mut stage =
            CanonicalStaging::new(output, publication::staging_budget(limits)?)
                .map_err(|error| format!("stage: {error:?}"))?;
        let records = self.stage(colors, &mut stage)?;
        let manifest = AssetManifest::new(
            bundle.manifest().source().clone(),
            records,
            limits,
        )
        .map_err(|error| format!("manifest: {error:?}"))?;
        publication::finish(stage, &manifest, limits)?;
        self.report()
    }

    pub(super) fn report(&self) -> Result<(), String> {
        toolchain::write_line(&format!(
            "Backdrop: palette pen {}",
            self.backdrop
        ))
        .map_err(|error| error.to_string())?;
        for layer in &self.layers {
            let (horizontal, vertical) = layer.origin;
            let digest =
                ContentHash::calculate(layer.image.pixels.as_flattened());
            toolchain::write_line(&format!(
                "Layer {}: origin {horizontal},{vertical} RGBA8 {digest}",
                layer.index,
            ))
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn stage(
        &self,
        colors: &[[u8; 4]],
        stage: &mut CanonicalStaging,
    ) -> Result<Vec<AssetRecord>, String> {
        let backdrop = Image {
            height: 1,
            pixels: vec![self.backdrop_rgba(colors)?],
            width: 1,
        };
        let mut records = Vec::with_capacity(4);
        backdrop.stage("scene/backdrop", &mut records, stage)?;
        for layer in &self.layers {
            let key = format!("scene/layer-{}", layer.index);
            layer.image.stage(&key, &mut records, stage)?;
        }
        Ok(records)
    }
}

#[cfg(feature = "native-preview")]
impl RegisteredLayer {
    fn commands(
        &self,
        scene: &mut SceneSession<'_, '_>,
        frame: FrameLayout,
    ) -> Result<impl Iterator<Item = SpriteDraw>, RenderError> {
        let dim = |value| {
            u32::try_from(value)
                .ok()
                .and_then(NonZeroU32::new)
                .ok_or(RenderError::InvalidDimensions)
        };
        let bytes = self.image.pixels.as_flattened();
        let budget =
            NonZeroUsize::new(bytes.len()).ok_or(RenderError::InvalidLength)?;
        let atlas_layout = FrameLayout::new(
            dim(self.image.width)?,
            dim(self.image.height)?,
            budget,
        )?;
        let atlas = scene.load_atlas(atlas_layout, bytes)?;
        let view = Viewport {
            height: frame.height().get(),
            width: frame.width().get(),
            x: u32::try_from(self.origin.0)
                .map_err(|_error| RenderError::InvalidDimensions)?,
            y: u32::try_from(self.origin.1)
                .map_err(|_error| RenderError::InvalidDimensions)?,
        };
        let period =
            Dimensions::new(atlas_layout.width(), atlas_layout.height());
        SpriteDraw::wrapped(atlas, period, view)
    }
}

/// Register independent maps and verify their sprite-free composition.
pub(super) fn registered_layers(
    context: (Machine, &SceneAssets<'_>),
    observation: &mut Observation,
    pens: &mut [Pixel],
) -> Result<RegisteredLayers, String> {
    register_layers(context, observation, pens, None)
}

// Compare the full layer composition before exposing any selected subset.
fn register_layers(
    (machine, assets): (Machine, &SceneAssets<'_>),
    observation: &mut Observation,
    pens: &mut [Pixel],
    omitted_layer: Option<usize>,
) -> Result<RegisteredLayers, String> {
    let _sprites = observation.sprites.take();
    observation.reconstruct(machine, assets, pens)?;
    let (state, mixer) = observation.tile_state(machine)?;
    let raster = machine.background_raster();
    let (width, height) = Raster::LAYER_EXTENT;
    let mut map = vec![Pixel::default(); width.saturating_mul(height)];
    let backdrop = u16::from(mixer.backdrop.palette_bank) << 4u32;
    let mut composed = vec![backdrop; pens.len()];
    let mut selected = omitted_layer.map(|_| vec![backdrop; pens.len()]);
    let mut layers = Vec::with_capacity(3);
    let background = Background {
        scroll_control: observation.scroll,
        state,
        tiles: &assets.tiles,
    };
    for index in mixer.ordered_layers() {
        let origin = raster
            .isolate_layer(background, index, &mut map)
            .map_err(|error| format!("isolated layer: {error:?}"))?;
        overlay_layer(&map, origin, raster.extent(), &mut composed)?;
        if Some(index) != omitted_layer
            && let Some(subset) = &mut selected
        {
            overlay_layer(&map, origin, raster.extent(), subset)?;
        }
        let image =
            Image::from_pens(&map, (width, height), &observation.colors, true)?;
        layers.push(RegisteredLayer { image, index, origin });
    }
    if !pens.iter().copied().map(Pixel::pen).eq(composed) {
        return Err(String::from("isolated layer composition differs"));
    }
    if let Some(subset) = selected {
        for (pixel, pen) in pens.iter_mut().zip(subset) {
            *pixel = Pixel::from_pen(pen);
        }
    }
    Ok(RegisteredLayers { backdrop, layers })
}

fn overlay_layer(
    map: &[Pixel],
    origin: (usize, usize),
    extent: (usize, usize),
    output: &mut [u16],
) -> Result<(), String> {
    let (width, height) = Raster::LAYER_EXTENT;
    let (view_width, view_height) = extent;
    if view_width == 0
        || view_height == 0
        || output.len() != view_width.saturating_mul(view_height)
        || map.len() != width.saturating_mul(height)
        || origin.0 >= width
        || origin.1 >= height
    {
        return Err(String::from("invalid isolated layer registration"));
    }
    for (row, pixels) in output.chunks_exact_mut(view_width).enumerate() {
        let y = origin.1.wrapping_add(row) & height.saturating_sub(1);
        let start = y.saturating_mul(width);
        let source = map
            .get(start..start.saturating_add(width))
            .ok_or_else(|| String::from("missing isolated layer row"))?;
        overlay_layer_row(source, origin.0, pixels)?;
    }
    Ok(())
}

fn overlay_layer_row(
    row: &[Pixel],
    x: usize,
    output: &mut [u16],
) -> Result<(), String> {
    let (width, _) = Raster::LAYER_EXTENT;
    for (column, pen) in output.iter_mut().enumerate() {
        let source = row
            .get(x.wrapping_add(column) & width.saturating_sub(1))
            .ok_or_else(|| String::from("missing isolated layer pixel"))?
            .pen();
        if source != TRANSPARENT {
            *pen = source;
        }
    }
    Ok(())
}

pub(super) fn report_background(
    context: (Machine, &SceneAssets<'_>),
    observation: Observation,
    pens: &mut [Pixel],
    selection: BackgroundSelection,
) -> Result<(), String> {
    let image = background_image(context, observation, pens, selection)?;
    if let Some(layer) = selection.omitted_layer {
        toolchain::write_line(&format!("Omitted registered layer: {layer}"))
            .map_err(|error| error.to_string())?;
    }
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
    selection: BackgroundSelection,
) -> Result<Image, String> {
    if selection.omitted_layer.is_some() {
        let _layers = register_layers(
            (machine, assets),
            &mut observation,
            pens,
            selection.omitted_layer,
        )?;
    } else {
        let _sprites = observation.sprites.take();
        observation.reconstruct(machine, assets, pens)?;
    }
    Image::from_pens(
        pens,
        machine.background_raster().extent(),
        &observation.colors,
        false,
    )?
    .without_top(selection.top)
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
    if group.complete {
        let budget = NonZeroUsize::new(raster.pixels())
            .ok_or_else(|| String::from("empty object pixel budget"))?;
        let state = machine
            .sprite_snapshot(&ram, &sprites.controls, mixer)
            .map_err(|error| format!("sprite state: {error:?}"))?;
        let region = state
            .isolate(raster, tiles, budget)
            .map_err(|error| format!("object isolation: {error:?}"))?;
        let mut crop = Image::from_pens(
            &region.pixels,
            region.extent,
            &observation.colors,
            true,
        )?
        .crop()?;
        crop.origin.0 = crop.origin.0.saturating_add(region.origin.0);
        crop.origin.1 = crop.origin.1.saturating_add(region.origin.1);
        return Ok(crop);
    }
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

fn layer_limits() -> Result<ManifestLimits, String> {
    let (width, height) = Raster::LAYER_EXTENT;
    let invalid = || String::from("invalid layer budget");
    let size = u64::try_from(width.saturating_mul(height).saturating_mul(4))
        .map_err(|_error| invalid())?;
    let bytes = NonZeroU64::new(size).ok_or_else(invalid)?;
    let total = NonZeroU64::new(size.saturating_mul(3).saturating_add(4))
        .ok_or_else(invalid)?;
    Ok(ManifestLimits {
        payload_bytes: bytes,
        records: NonZeroUsize::MIN.saturating_add(3),
        total_bytes: total,
        ..limits()?
    })
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

fn parse_selection(mut values: Vec<OsString>) -> Result<Vec<Group>, String> {
    let complete = values
        .first()
        .is_some_and(|value| value == OsStr::new(COMPLETE_OBJECTS));
    if complete {
        let _flag = values.remove(0);
    }
    if !(1..=32).contains(&values.len()) {
        return Err(String::from(USAGE));
    }
    parse_groups(values, complete)
}

fn parse_groups(
    values: Vec<OsString>,
    complete: bool,
) -> Result<Vec<Group>, String> {
    let mut used = BTreeSet::new();
    let mut keys = BTreeSet::new();
    let mut parsed = Vec::new();
    for value in values {
        let (key, raw_indices) = text(&value)?
            .split_once(':')
            .ok_or_else(|| String::from("expected KEY:INDEX,INDEX"))?;
        AssetRecord::validate_descriptor(key, AssetFormat::Data, 1)
            .map_err(|error| format!("actor group key: {error:?}"))?;
        if key == "scene/background" || !keys.insert(key.to_owned()) {
            return Err(String::from("reserved or duplicate actor group key"));
        }
        let indices = parse_indices(raw_indices, &mut used)?;
        parsed.push(Group {
            complete,
            indices,
            key: key.to_owned(),
        });
    }
    Ok(parsed)
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
    image: &Image,
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
    use games_assets::SourceIdentity;
    use games_platform::ir::storage::fixtures::TestRoot;

    use super::*;

    #[test]
    fn actor_digests_require_disjoint_named_groups_and_complete_scene() {
        let digest = "0".repeat(64);
        let base = [
            "parent",
            &digest,
            "ram",
            "registers",
            "scroll",
            "palette",
            "frame",
            "objects",
            "controls",
        ];
        let parse = |groups: &[&str]| {
            ActorDigests::parse(
                base.into_iter()
                    .chain(groups.iter().copied())
                    .map(OsString::from)
                    .collect(),
            )
        };
        assert!(parse(&["homer:2,3", "marge:4"]).is_ok());
        assert!(
            parse(&["homer:2"]).is_ok_and(|args| args
                .groups
                .iter()
                .all(|group| !group.complete))
        );
        assert!(
            parse(&[COMPLETE_OBJECTS, "homer:2", "marge:4"])
                .is_ok_and(|args| args.groups.len() == 2
                    && args.groups.iter().all(|group| group.complete))
        );
        for invalid in [
            &[][..],
            &[COMPLETE_OBJECTS],
            &[COMPLETE_OBJECTS, COMPLETE_OBJECTS, "homer:2"],
            &["homer:2", COMPLETE_OBJECTS],
            &[COMPLETE_OBJECTS, "homer:2", "marge:2"],
            &["homer:2", "marge:2"],
            &["homer:2", "homer:3"],
            &[":2"],
            &["scene/background:2"],
            &["Homer:2"],
            &["../homer:2"],
            &["homer:256"],
            &["homer:"],
        ] {
            assert!(parse(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn layer_publication_requires_one_complete_scene() {
        let digest = "0".repeat(64);
        let args: Vec<_> = [
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
        .into_iter()
        .map(OsString::from)
        .collect();
        assert!(LayerPublication::parse(args.clone()).is_ok());
        let mut extra = args;
        extra.push(OsString::from("unexpected"));
        assert!(LayerPublication::parse(extra.clone()).is_err());
        extra.truncate(9);
        assert!(LayerPublication::parse(extra).is_err());
    }

    fn synthetic_layers() -> RegisteredLayers {
        let (width, height) = Raster::LAYER_EXTENT;
        let layer = |index, origin, pixel| RegisteredLayer {
            image: Image {
                height,
                pixels: vec![pixel; width.saturating_mul(height)],
                width,
            },
            index,
            origin,
        };
        RegisteredLayers {
            backdrop: 1,
            layers: vec![
                layer(1, (0, 31), [0; 4]),
                layer(2, (0, 31), [0; 4]),
                layer(0, (96, 16), [11, 22, 33, 255]),
            ],
        }
    }

    #[test]
    fn layer_publication_preserves_maps_and_deduplicates_payloads()
    -> Result<(), String> {
        let failure = |error| format!("layer staging: {error:?}");
        let root = TestRoot::new().map_err(|error| format!("{error:?}"))?;
        let parent = root.location();
        let layers = synthetic_layers();
        let backdrop = [12, 34, 56, 255];
        let colors = [[0; 4], backdrop];
        let limits = layer_limits()?;
        let mut stage =
            CanonicalStaging::new(parent, publication::staging_budget(limits)?)
                .map_err(failure)?;
        let records = layers.stage(&colors, &mut stage)?;
        let source = SourceIdentity {
            revision: String::from("fixture"),
            title: String::from("synthetic"),
        };
        let manifest = AssetManifest::new(source, records, limits)
            .map_err(|error| format!("layer manifest: {error:?}"))?;
        let staged = stage.finish(&manifest, limits).map_err(failure)?;
        let digest = staged.digest();
        staged
            .publish()
            .map_err(failure)?
            .durability
            .map_err(|error| format!("{error:?}"))?;
        let bundle = CanonicalBundle::open(parent, digest, limits)
            .map_err(|error| format!("{error:?}"))?;
        verify_published_layers(&bundle, &layers, backdrop)
    }

    fn verify_published_layers(
        bundle: &CanonicalBundle,
        layers: &RegisteredLayers,
        backdrop: [u8; 4],
    ) -> Result<(), String> {
        let read = |key: &str| {
            bundle
                .read_payload(key)
                .map_err(|error| format!("{error:?}"))
        };
        if read("scene/backdrop")? != backdrop {
            return Err(String::from("backdrop payload differs"));
        }
        for layer in &layers.layers {
            let key = format!("scene/layer-{}", layer.index);
            if read(&key)? != layer.image.pixels.as_flattened() {
                return Err(String::from("map payload differs"));
            }
        }
        let first = bundle.manifest().record("scene/layer-1");
        let second = bundle.manifest().record("scene/layer-2");
        if first
            .zip(second)
            .is_none_or(|(left, right)| left.digest != right.digest)
        {
            return Err(String::from("equal maps did not share a digest"));
        }
        Ok(())
    }

    #[test]
    fn strip_scene_count_respects_explicit_budget() {
        let digest = "0".repeat(64);
        let header = ["output", "parent", &digest, "28", "16"];
        let scene = [
            "ram",
            "registers",
            "scroll",
            "palette",
            "frame",
            "objects",
            "controls",
        ];
        let args = |budget: &[&str], count| {
            header
                .into_iter()
                .chain(budget.iter().copied())
                .chain(scene.into_iter().cycle().take(count * scene.len()))
                .map(OsString::from)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            Strip::parse(&args(&[], 8)).map(|strip| strip.scenes.len()),
            Ok(8)
        );
        assert!(Strip::parse(&args(&[], 9)).is_err());
        assert_eq!(
            Strip::parse(&args(&["--scene-limit", "10"], 10))
                .map(|strip| strip.scenes.len()),
            Ok(10)
        );
        assert!(Strip::parse(&args(&["--scene-limit", "9"], 10)).is_err());
        assert!(Strip::parse(&args(&["--scene-limit", "1"], 2)).is_err());
        assert!(Strip::parse(&args(&["--scene-limit", "10"], 1)).is_err());
        assert!(Strip::parse(&args(&["--scene-limit", "bad"], 2)).is_err());
        let mut incomplete = args(&["--scene-limit", "10"], 2);
        let _last_path = incomplete.pop();
        assert!(Strip::parse(&incomplete).is_err());
    }

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
            BackgroundPublication::parse(args.clone())
                .map(|value| value.selection.top),
            Ok(28)
        );
        let with_layer = |layer: &str| {
            let mut candidate = args.clone();
            candidate.extend(["--omit-layer", layer].map(OsString::from));
            BackgroundPublication::parse(candidate).map(|value| value.selection)
        };
        for layer in 0..3 {
            assert_eq!(
                with_layer(&layer.to_string()),
                Ok(BackgroundSelection {
                    omitted_layer: Some(layer),
                    top: 28
                })
            );
        }
        assert!(with_layer("3").is_err());
        assert!(with_layer("-1").is_err());
        assert!(with_layer("not-a-layer").is_err());
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
