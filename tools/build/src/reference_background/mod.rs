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
//   - Bounded scene comparison against private screen observations.
// - Must-Not:
//   - Implement hardware decoding or claim gameplay parity.
// - Allows:
//   - Inputs: Pinned bundle, tile state, palette RAM and captured RGB32.
//   - Outputs: Counts of matching and differing visible pixels.
//   - Side effects: Bounded local file reads and stdout summaries.
// - Split-When:
//   - Another observation format requires independent comparison.
// - Merge-When:
//   - Shared reference comparison can express the same inputs.
// - Summary:
//   - Measure a reconstructed background or scene against a captured frame.
// - Description:
//   - Report exact matches and differences without inferred tolerance.
// - Usage:
//   - Supply observations aligned to the displayed original-machine frame.
// - Defaults:
//   - Exact file lengths, read-only and no private image publication.

//! Host-side comparison of original background and complete screen pixels.

/// Bounded canonical scene art publication.
pub mod art;
#[cfg(feature = "native-preview")]
/// Bounded changing-scene reconstruction and presentation benchmark.
pub mod sequence;

use std::ffi::{OsStr, OsString};
use std::num::NonZeroU64;
#[cfg(feature = "native-preview")]
use std::num::{NonZeroU32, NonZeroUsize};

use games_assets::ContentHash;
use games_assets::indexed::IndexedTiles;
#[cfg(feature = "native-preview")]
use games_platform::ir::frame::{FrameLayout, RenderError};
#[cfg(feature = "native-preview")]
use games_platform::ir::graphics::NativeWindow;
#[cfg(feature = "native-preview")]
use games_platform::ir::importer::reference::Raster;
use games_platform::ir::importer::reference::{
    Background, Machine, Pixel, RAM_BYTES, TilemapSnapshot, VideoPriority,
};
#[cfg(feature = "native-preview")]
use games_platform::ir::scene::{SceneLimits, SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::CanonicalBundle;
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::toolchain;

#[cfg(feature = "native-preview")]
use crate::render_check::present;
use crate::{USAGE, reference_palette, text};

/// Explicit paths to a single bounded, private original observation.
#[derive(Debug)]
pub struct Arguments {
    background_top: Option<usize>,
    digest: ContentHash,
    frame: OsString,
    layer_digests: bool,
    #[cfg(feature = "native-preview")]
    native: bool,
    palette: OsString,
    parent: OsString,
    ram: OsString,
    registers: OsString,
    scroll: OsString,
    sprites: Option<SpritePaths>,
}

#[derive(Debug)]
struct SpritePaths {
    controls: OsString,
    ram: OsString,
}

impl Arguments {
    fn compare(
        &self,
        machine: Machine,
        bundle: &CanonicalBundle,
    ) -> Result<(), String> {
        let payloads = GraphicsPayloads::load(bundle, self.sprites.is_some())?;
        let assets = payloads.bind(bundle)?;
        let (observation, frame) = Observation::load(self, machine)?;
        let mut pens =
            vec![Pixel::default(); machine.background_raster().pixels()];
        observation.reconstruct(machine, &assets, &mut pens)?;
        let colors = &observation.colors;
        let matched = count_matches(&pens, colors, &frame)?;
        if (self.layer_digests || self.background_top.is_some())
            && matched != pens.len()
        {
            return Err(String::from("scene differs from observation"));
        }
        if self.layer_digests {
            return art::report_layers(
                (machine, &assets),
                observation,
                &mut pens,
            );
        }
        if let Some(top) = self.background_top {
            return art::report_background(
                (machine, &assets),
                observation,
                &mut pens,
                top,
            );
        }
        #[cfg(feature = "native-preview")]
        if self.native {
            native_check(machine.background_raster(), &pens, colors)?;
        }
        let label = if self.sprites.is_some() {
            "Scene"
        } else {
            "Background-only"
        };
        toolchain::write_line(&format!(
            "{label}: {matched}/{} pixels match; {} differ",
            pens.len(),
            pens.len().saturating_sub(matched),
        ))
        .map_err(|error| error.to_string())
    }

    /// Parse exact bundle and observation arguments.
    /// # Errors
    /// Rejects malformed or missing arguments.
    fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let [parent, digest, ram, registers, scroll, palette, frame] =
            <[OsString; 7]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        Ok(Self {
            background_top: None,
            digest: text(&digest)?
                .parse()
                .map_err(|_error| String::from("invalid manifest hash"))?,
            frame,
            layer_digests: false,
            #[cfg(feature = "native-preview")]
            native: false,
            palette,
            parent,
            ram,
            registers,
            scroll,
            sprites: None,
        })
    }

    /// Parse one background or scene diagnostic invocation.
    /// # Errors
    /// Rejects unknown commands or malformed arguments.
    pub fn parse_command(
        action: &str,
        mut args: Vec<OsString>,
    ) -> Result<Self, String> {
        match action {
            "background-check" => Self::parse(args),
            "scene-check" => Self::parse_scene(args),
            "scene-layer-digests" => {
                let mut parsed = Self::parse_scene(args)?;
                parsed.layer_digests = true;
                Ok(parsed)
            },
            "scene-background-digest" => {
                let top = args.pop().ok_or_else(|| String::from(USAGE))?;
                let mut parsed = Self::parse_scene(args)?;
                parsed.background_top =
                    Some(text(&top)?.parse().map_err(|_error| {
                        String::from("invalid top exclusion")
                    })?);
                Ok(parsed)
            },
            #[cfg(feature = "native-preview")]
            "scene-render-check" => {
                let mut parsed = Self::parse_scene(args)?;
                parsed.native = true;
                Ok(parsed)
            },
            _ => Err(String::from(USAGE)),
        }
    }

    /// Parse a complete background plus device object observation.
    /// # Errors
    /// Rejects malformed or missing arguments.
    fn parse_scene(mut args: Vec<OsString>) -> Result<Self, String> {
        if args.len() != 9 {
            return Err(String::from(USAGE));
        }
        let [ram, controls] = <[OsString; 2]>::try_from(args.split_off(7))
            .map_err(|_args| String::from(USAGE))?;
        let mut parsed = Self::parse(args)?;
        parsed.sprites = Some(SpritePaths { controls, ram });
        Ok(parsed)
    }

    /// Compare a reconstructed observation with a full original frame.
    /// # Errors
    /// Rejects invalid inputs or unavailable assets, not pixel differences.
    pub fn run(&self, machine: Machine) -> Result<(), String> {
        let bundle = CanonicalBundle::open(
            &self.parent,
            self.digest,
            machine.graphics_limits(),
        )
        .map_err(|error| format!("bundle: {error:?}"))?;
        machine
            .validate_manifest(bundle.manifest())
            .map_err(|error| format!("profile manifest: {error:?}"))?;
        self.compare(machine, &bundle)
    }
}

#[derive(Debug)]
struct GraphicsPayloads {
    sprites: Option<Vec<u8>>,
    tiles: Vec<u8>,
}

impl GraphicsPayloads {
    fn bind<'payload>(
        &'payload self,
        bundle: &CanonicalBundle,
    ) -> Result<SceneAssets<'payload>, String> {
        let sprites = self
            .sprites
            .as_deref()
            .map(|bytes| indexed(bundle, "graphics/sprites", bytes))
            .transpose()?;
        Ok(SceneAssets {
            sprites,
            tiles: indexed(bundle, "graphics/tiles", &self.tiles)?,
        })
    }

    fn load(
        bundle: &CanonicalBundle,
        with_sprites: bool,
    ) -> Result<Self, String> {
        let read_key = |key| {
            bundle
                .read_payload(key)
                .map_err(|error| format!("graphics payload: {error:?}"))
        };
        Ok(Self {
            sprites: with_sprites
                .then(|| read_key("graphics/sprites"))
                .transpose()?,
            tiles: read_key("graphics/tiles")?,
        })
    }
}

#[derive(Debug)]
struct Observation {
    colors: reference_palette::Colors,
    ram: Vec<u8>,
    registers: Vec<u8>,
    scroll: u8,
    sprites: Option<SpriteObservation>,
}

type LoadedObservation = (Observation, Vec<u8>);
type TileState<'source> = (TilemapSnapshot<'source>, VideoPriority);

impl Observation {
    fn load(
        paths: &Arguments,
        machine: Machine,
    ) -> Result<LoadedObservation, String> {
        let scroll = read(&paths.scroll, 1)?;
        let [control] = scroll.as_slice() else {
            return Err(String::from("invalid scroll control"));
        };
        let raw = snapshot(
            paths.palette.as_os_str(),
            reference_palette::byte_limit(machine, 2)?,
        )
        .map_err(|error| format!("palette RAM: {error:?}"))?;
        let observation = Self {
            colors: reference_palette::decode(machine, &raw)?,
            ram: read(&paths.ram, 24_576)?,
            registers: read(&paths.registers, 22)?,
            scroll: *control,
            sprites: paths
                .sprites
                .as_ref()
                .map(SpriteObservation::load)
                .transpose()?,
        };
        let frame = read(
            &paths.frame,
            machine.background_raster().pixels().saturating_mul(4),
        )?;
        Ok((observation, frame))
    }

    fn reconstruct(
        &self,
        machine: Machine,
        assets: &SceneAssets<'_>,
        pens: &mut [Pixel],
    ) -> Result<(), String> {
        let (state, mixer) = self.tile_state(machine)?;
        machine
            .background_raster()
            .compose(
                Background {
                    state,
                    scroll_control: self.scroll,
                    tiles: &assets.tiles,
                },
                pens,
            )
            .map_err(|error| format!("background: {error:?}"))?;
        if let Some(sprites) = &self.sprites {
            let tiles = assets
                .sprites
                .as_ref()
                .ok_or_else(|| String::from("missing sprite tiles"))?;
            sprites.overlay(machine, (tiles, mixer), pens)?;
        }
        Ok(())
    }

    fn rightward_shift(
        &self,
        next: &Self,
        machine: Machine,
    ) -> Result<usize, String> {
        let (state, _) = self.tile_state(machine)?;
        let (next_state, _) = next.tile_state(machine)?;
        state
            .rightward_shift(next_state, self.scroll, next.scroll)
            .map_err(|error| format!("camera shift: {error:?}"))
    }

    fn tile_state(&self, machine: Machine) -> Result<TileState<'_>, String> {
        let (controls, mixer_raw) = self
            .registers
            .split_first_chunk::<6>()
            .ok_or_else(|| String::from("missing controls"))?;
        let mixer = machine
            .decode_video_priority(mixer_raw)
            .map_err(|error| format!("mixer: {error:?}"))?;
        let state = machine
            .tilemap_snapshot(&self.ram, controls, mixer)
            .map_err(|error| format!("tile state: {error:?}"))?;
        Ok((state, mixer))
    }
}

#[derive(Debug)]
struct SceneAssets<'payload> {
    sprites: Option<IndexedTiles<'payload>>,
    tiles: IndexedTiles<'payload>,
}

#[derive(Debug)]
struct SpriteObservation {
    controls: Vec<u8>,
    ram: Vec<u8>,
}

impl SpriteObservation {
    fn load(paths: &SpritePaths) -> Result<Self, String> {
        Ok(Self {
            controls: read(&paths.controls, 44)?,
            ram: read(&paths.ram, RAM_BYTES)?,
        })
    }

    fn overlay(
        &self,
        machine: Machine,
        context: (&IndexedTiles<'_>, VideoPriority),
        pens: &mut [Pixel],
    ) -> Result<(), String> {
        let (tiles, mixer) = context;
        let state = machine
            .sprite_snapshot(&self.ram, &self.controls, mixer)
            .map_err(|error| format!("sprite state: {error:?}"))?;
        state
            .overlay(machine.background_raster(), tiles, pens)
            .map_err(|error| format!("sprite overlay: {error:?}"))
    }
}

fn count_matches(
    pens: &[Pixel],
    colors: &[[u8; 4]],
    frame: &[u8],
) -> Result<usize, String> {
    let (words, remainder) = frame.as_chunks::<4>();
    if !remainder.is_empty() || words.len() != pens.len() {
        return Err(String::from("incorrect frame length"));
    }
    let mut matched = 0usize;
    for (&pen, &[blue, green, red, alpha]) in pens.iter().zip(words) {
        let reconstructed = pen
            .rgba(colors)
            .ok_or_else(|| String::from("pixel pen outside palette"))?;
        if reconstructed == [red, green, blue, alpha] {
            matched = matched.saturating_add(1);
        }
    }
    Ok(matched)
}

fn indexed<'payload>(
    bundle: &CanonicalBundle,
    key: &str,
    bytes: &'payload [u8],
) -> Result<IndexedTiles<'payload>, String> {
    let record = bundle
        .manifest()
        .record(key)
        .ok_or_else(|| String::from("missing graphics record"))?;
    IndexedTiles::new(record, bytes)
        .map_err(|error| format!("indexed tiles: {error:?}"))
}

#[cfg(feature = "native-preview")]
fn native_check(
    raster: Raster,
    pens: &[Pixel],
    colors: &[[u8; 4]],
) -> Result<(), String> {
    let pixels: Vec<_> = pens
        .iter()
        .map(|pen| pen.rgba(colors))
        .collect::<Option<_>>()
        .ok_or_else(|| String::from("pixel pen outside palette"))?;
    let layout = native_layout(raster, pixels.as_flattened().len())?;
    let bytes = NonZeroUsize::new(layout.bytes())
        .ok_or_else(|| String::from("empty raster"))?;
    let mut window = NativeWindow::open(
        "Reconstructed scene verification",
        layout.width(),
        layout.height(),
        true,
    )
    .map_err(|error| format!("native window: {error:?}"))?;
    let renderer = window.renderer_name();
    let limits = SceneLimits {
        atlas_bytes: bytes,
        atlases: NonZeroUsize::MIN,
        sprites: NonZeroUsize::MIN,
    };
    let rect = SpriteRect {
        height: layout.height(),
        width: layout.width(),
        x: 0,
        y: 0,
    };
    window
        .with_scene(layout, limits, |scene| {
            let id = scene.load_atlas(layout, pixels.as_flattened())?;
            present(scene, &[SpriteDraw {
                atlas: id,
                destination: rect,
                flip_horizontal: false,
                flip_vertical: false,
                source: rect,
            }])?;
            if scene.readback()? != pixels.as_flattened() {
                return Err(RenderError::Backend(String::from(
                    "reconstructed scene readback differs",
                )));
            }
            Ok(())
        })
        .map_err(|error| format!("native scene: {error:?}"))?;
    toolchain::write_line(&format!(
        "Native scene readback matches ({renderer})"
    ))
    .map_err(|error| error.to_string())
}

#[cfg(feature = "native-preview")]
fn native_layout(raster: Raster, bytes: usize) -> Result<FrameLayout, String> {
    let dimension = |value| {
        u32::try_from(value)
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or_else(|| String::from("invalid raster dimensions"))
    };
    let (width, height) = raster.extent();
    let budget =
        NonZeroUsize::new(bytes).ok_or_else(|| String::from("empty raster"))?;
    FrameLayout::new(dimension(width)?, dimension(height)?, budget)
        .map_err(|error| format!("frame layout: {error:?}"))
}

fn read(path: &OsStr, length: usize) -> Result<Vec<u8>, String> {
    let limit = u64::try_from(length)
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or_else(|| String::from("invalid observation length"))?;
    let bytes = snapshot(path, limit)
        .map_err(|error| format!("observation: {error:?}"))?;
    if bytes.len() != length {
        return Err(String::from("incorrect observation length"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_admission_requires_exact_scene_arguments() {
        let digest = "0".repeat(64);
        let base = [
            "parent",
            &digest,
            "ram",
            "registers",
            "scroll",
            "palette",
            "frame",
        ]
        .map(OsString::from)
        .to_vec();
        assert!(
            Arguments::parse_command("background-check", base.clone()).is_ok()
        );
        assert!(Arguments::parse_command("scene-check", base.clone()).is_err());
        let mut scene = base;
        scene.extend(["objects", "controls"].map(OsString::from));
        assert!(Arguments::parse_command("scene-check", scene.clone()).is_ok());
        assert!(
            Arguments::parse_command("background-check", scene.clone())
                .is_err()
        );
        assert_eq!(
            Arguments::parse_command("scene-render-check", scene.clone())
                .is_ok(),
            cfg!(feature = "native-preview")
        );
        scene.push(OsString::from("extra"));
        assert!(Arguments::parse_command("scene-check", scene).is_err());
    }

    #[test]
    fn layer_digests_require_complete_scene() {
        let digest = "0".repeat(64);
        let mut args = [
            "parent",
            &digest,
            "ram",
            "registers",
            "scroll",
            "palette",
            "frame",
            "objects",
        ]
        .map(OsString::from)
        .to_vec();
        let parse = |values| {
            Arguments::parse_command("scene-layer-digests", values)
                .map(|value| value.layer_digests)
        };
        assert!(parse(args.clone()).is_err());
        args.push(OsString::from("controls"));
        assert_eq!(parse(args.clone()), Ok(true));
        args.push(OsString::from("extra"));
        assert!(parse(args).is_err());
    }

    #[test]
    fn background_digest_requires_complete_scene_and_explicit_crop() {
        let digest = "0".repeat(64);
        let mut args = [
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
        let parse = |values| {
            Arguments::parse_command("scene-background-digest", values)
                .map(|value| value.background_top)
        };
        assert!(parse(args.clone()).is_err());
        args.push(OsString::from("28"));
        assert_eq!(parse(args.clone()), Ok(Some(28)));
        assert!(Arguments::parse_command("scene-check", args.clone()).is_err());
        let _top = args.pop();
        args.push(OsString::from("-1"));
        assert!(parse(args).is_err());
    }

    #[test]
    fn comparison_counts_only_exact_pixels_and_checks_extent() {
        let colors = [[1, 2, 3, 255], [4, 5, 6, 255]];
        let frame = [3, 2, 1, 255, 6, 5, 0, 255];
        assert_eq!(
            count_matches(
                &[Pixel::from_pen(0), Pixel::from_pen(1)],
                &colors,
                &frame
            ),
            Ok(1)
        );
        assert!(
            count_matches(
                &[Pixel::from_pen(0), Pixel::from_pen(2)],
                &colors,
                &frame
            )
            .is_err()
        );
        assert!(count_matches(&[Pixel::from_pen(0)], &colors, &frame).is_err());
    }
}
