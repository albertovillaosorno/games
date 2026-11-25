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

use std::ffi::OsString;
use std::io::{Write as _, stdout};
use std::num::NonZeroU64;
#[cfg(feature = "native-preview")]
use std::num::{NonZeroU32, NonZeroUsize};
use std::path::{Path, PathBuf};

use games_assets::ContentHash;
use games_assets::indexed::IndexedTiles;
use games_importer::mame::Machine;
#[cfg(feature = "native-preview")]
use games_importer::mame::background::Raster;
use games_importer::mame::background::{Background, Pixel};
use games_importer::mame::objects::RAM_BYTES;
use games_importer::mame::priority::VideoPriority;
#[cfg(feature = "native-preview")]
use games_platform_graphics::linux::sprites::{
    SceneLimits, SpriteDraw, SpriteRect,
};
#[cfg(feature = "native-preview")]
use games_platform_graphics::linux::{FrameLayout, NativeWindow, RenderError};
use games_platform_storage::bundle::CanonicalBundle;
use games_platform_storage::file::snapshot;

#[cfg(feature = "native-preview")]
use crate::render_check::present;
use crate::{USAGE, reference_palette, text};

/// Explicit paths to a single bounded, private original observation.
#[derive(Debug)]
pub struct Arguments {
    digest: ContentHash,
    frame: PathBuf,
    #[cfg(feature = "native-preview")]
    native: bool,
    palette: PathBuf,
    parent: PathBuf,
    ram: PathBuf,
    registers: PathBuf,
    scroll: PathBuf,
    sprites: Option<SpritePaths>,
}

#[derive(Debug)]
struct SpritePaths {
    controls: PathBuf,
    ram: PathBuf,
}

impl Arguments {
    fn compare(
        &self,
        machine: Machine,
        bundle: &CanonicalBundle,
    ) -> Result<(), String> {
        let pens = self.reconstruct(machine, bundle)?;
        let raw =
            snapshot(&self.palette, reference_palette::byte_limit(machine, 2)?)
                .map_err(|error| format!("palette RAM: {error:?}"))?;
        let colors = reference_palette::decode(machine, &raw)?;
        let frame = read(&self.frame, pens.len().saturating_mul(4))?;
        let matched = count_matches(&pens, &colors, &frame)?;
        #[cfg(feature = "native-preview")]
        if self.native {
            native_check(machine.background_raster(), &pens, &colors)?;
        }
        let label = if self.sprites.is_some() {
            "Scene"
        } else {
            "Background-only"
        };
        writeln!(
            stdout().lock(),
            "{label}: {matched}/{} pixels match; {} differ",
            pens.len(),
            pens.len().saturating_sub(matched),
        )
        .map_err(|error| error.to_string())
    }

    fn overlay(
        machine: Machine,
        bundle: &CanonicalBundle,
        observation: (&SpritePaths, VideoPriority),
        pens: &mut [Pixel],
    ) -> Result<(), String> {
        let (paths, mixer) = observation;
        let ram = read(&paths.ram, RAM_BYTES)?;
        let controls = read(&paths.controls, 44)?;
        let state = machine
            .sprite_snapshot(&ram, &controls, mixer)
            .map_err(|error| format!("sprite state: {error:?}"))?;
        let key = "graphics/sprites";
        let bytes = bundle
            .read_payload(key)
            .map_err(|error| format!("sprites: {error:?}"))?;
        let tiles = indexed(bundle, key, &bytes)?;
        state
            .overlay(machine.background_raster(), &tiles, pens)
            .map_err(|error| format!("sprite overlay: {error:?}"))
    }

    /// Parse exact bundle and observation arguments.
    /// # Errors
    /// Rejects malformed or missing arguments.
    fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let [parent, digest, ram, registers, scroll, palette, frame] =
            <[OsString; 7]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        Ok(Self {
            digest: text(&digest)?
                .parse()
                .map_err(|_error| String::from("invalid manifest hash"))?,
            frame: frame.into(),
            #[cfg(feature = "native-preview")]
            native: false,
            palette: palette.into(),
            parent: parent.into(),
            ram: ram.into(),
            registers: registers.into(),
            scroll: scroll.into(),
            sprites: None,
        })
    }

    /// Parse one background or scene diagnostic invocation.
    /// # Errors
    /// Rejects unknown commands or malformed arguments.
    pub fn parse_command(
        action: &str,
        args: Vec<OsString>,
    ) -> Result<Self, String> {
        match action {
            "background-check" => Self::parse(args),
            "scene-check" => Self::parse_scene(args),
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
        parsed.sprites = Some(SpritePaths {
            controls: controls.into(),
            ram: ram.into(),
        });
        Ok(parsed)
    }

    fn reconstruct(
        &self,
        machine: Machine,
        bundle: &CanonicalBundle,
    ) -> Result<Vec<Pixel>, String> {
        let ram = read(&self.ram, 24_576)?;
        let registers = read(&self.registers, 22)?;
        let scroll = read(&self.scroll, 1)?;
        let [control] = scroll.as_slice() else {
            return Err(String::from("invalid scroll control"));
        };
        let (controls, mixer_raw) = registers
            .split_first_chunk::<6>()
            .ok_or_else(|| String::from("missing controls"))?;
        let mixer = machine
            .decode_video_priority(mixer_raw)
            .map_err(|error| format!("mixer: {error:?}"))?;
        let state = machine
            .tilemap_snapshot(&ram, controls, mixer)
            .map_err(|error| format!("tile state: {error:?}"))?;
        let key = "graphics/tiles";
        let bytes = bundle
            .read_payload(key)
            .map_err(|error| format!("tiles: {error:?}"))?;
        let tiles = indexed(bundle, key, &bytes)?;
        let raster = machine.background_raster();
        let mut pens = vec![Pixel::default(); raster.pixels()];
        raster
            .compose(
                Background {
                    state,
                    scroll_control: *control,
                    tiles: &tiles,
                },
                &mut pens,
            )
            .map_err(|error| format!("background: {error:?}"))?;
        if let Some(paths) = &self.sprites {
            Self::overlay(machine, bundle, (paths, mixer), &mut pens)?;
        }
        Ok(pens)
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
    writeln!(
        stdout().lock(),
        "Native scene readback matches ({renderer})"
    )
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

fn read(path: &Path, length: usize) -> Result<Vec<u8>, String> {
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
