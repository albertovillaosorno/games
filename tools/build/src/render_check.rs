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
//   - Composition of verified assets and a bounded native render check.
// - Must-Not:
//   - Infer palette assignment, implement gameplay or publish private assets.
// - Allows:
//   - Inputs: Bundle identity, palette observation and explicit tile selection.
//   - Outputs: Native logical-target verification summary.
//   - Side effects: Reads bounded local files, opens a hidden native window.
// - Split-When:
//   - A title-specific gameplay runtime needs independent presentation.
// - Merge-When:
//   - Another host command duplicates this diagnostic composition.
// - Summary:
//   - Exercise imported graphics through canonical atlas and sprite APIs.
// - Description:
//   - Compare native logical-target readback with CPU-packed RGBA pixels.
// - Usage:
//   - Select tiles and palette entries explicitly in a verified bundle.
// - Defaults:
//   - No palette inference, visible window, or private pixel publication.

//! Native rendering diagnostics for explicitly selected imported tiles.

use std::ffi::OsString;
use std::num::{NonZeroU32, NonZeroUsize};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use games::performance::PerformanceCollector;
use games_assets::ContentHash;
use games_assets::indexed::IndexedTiles;
use games_assets::indexed::atlas::{AtlasTile, TileAtlasLayout};
use games_platform::ir::frame::{FrameLayout, RenderError};
use games_platform::ir::graphics::NativeWindow;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::importer::reference::Machine;
use games_platform::ir::measurement::now;
use games_platform::ir::scene::{AtlasId, SceneLimits, SpriteDraw, SpriteRect};
use games_platform::ir::storage::bundle::CanonicalBundle;
use games_platform::ir::storage::file::snapshot;
use games_platform::ir::toolchain;

use crate::render_benchmark::Schedule;
use crate::{USAGE, reference_palette, render_benchmark, text};

const ATLAS_BYTES: NonZeroUsize = NonZeroUsize::MIN.saturating_add(4_194_303);
const MAX_BENCHMARK_FRAMES: u32 = 20_000;
const MAX_SIDE: u32 = 1024;
const MAX_TILES: u32 = 1024;

/// Explicit local inputs and limits for a finite native render check.
#[derive(Debug)]
pub struct Arguments {
    benchmark_frames: Option<NonZeroU32>,
    columns: NonZeroU32,
    count: NonZeroU32,
    digest: ContentHash,
    first: u32,
    key: String,
    parent: PathBuf,
    pen_count: NonZeroUsize,
    pen_start: usize,
    raw: PathBuf,
}

impl Arguments {
    fn end(&self) -> Result<u32, String> {
        if self.count.get() > MAX_TILES || self.columns > self.count {
            return Err(String::from("invalid tile selection size"));
        }
        self.first
            .checked_add(self.count.get())
            .ok_or_else(|| String::from("tile selection overflow"))
    }

    /// Parse the explicit bundle and tile selection.
    /// # Errors
    /// Rejects incomplete, invalid or excessive selections.
    pub fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let [parent, digest, key, raw, first, count, columns, start, pens] =
            <[OsString; 9]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        let result = Self {
            benchmark_frames: None,
            columns: number(&columns)?,
            count: number(&count)?,
            digest: number(&digest)?,
            first: number(&first)?,
            key: text(&key)?.to_owned(),
            parent: parent.into(),
            pen_count: number(&pens)?,
            pen_start: number(&start)?,
            raw: raw.into(),
        };
        let _end = result.end()?;
        Ok(result)
    }

    /// Parse an identical scene selection and a bounded frame count.
    /// # Errors
    /// Rejects invalid or excessive frame counts and scene selections.
    pub fn parse_benchmark(mut args: Vec<OsString>) -> Result<Self, String> {
        let frames: NonZeroU32 =
            number(&args.pop().ok_or_else(|| String::from(USAGE))?)?;
        if frames.get() > MAX_BENCHMARK_FRAMES {
            return Err(String::from("benchmark frame count exceeds limit"));
        }
        let mut parsed = Self::parse(args)?;
        parsed.benchmark_frames = Some(frames);
        Ok(parsed)
    }

    fn prepare(
        &self,
        sheet: &IndexedTiles<'_>,
        colors: &[[u8; 4]],
    ) -> Result<PreparedAtlas, String> {
        let end = self.end()?;
        if end > sheet.tiles().get() {
            return Err(String::from("tile selection exceeds asset"));
        }
        let pen_end = self
            .pen_start
            .checked_add(self.pen_count.get())
            .ok_or_else(|| String::from("palette selection overflow"))?;
        let palette = colors
            .get(self.pen_start..pen_end)
            .ok_or_else(|| String::from("palette selection exceeds entries"))?;
        let layout =
            TileAtlasLayout::new(sheet, self.count, self.columns, ATLAS_BYTES)
                .map_err(|error| format!("atlas layout: {error:?}"))?;
        if layout.width().get() > MAX_SIDE || layout.height().get() > MAX_SIDE {
            return Err(String::from("atlas side exceeds diagnostic limit"));
        }
        let selections: Vec<_> = (self.first..end)
            .map(|index| AtlasTile { index, palette })
            .collect();
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(layout.pixels())
            .map_err(|_error| String::from("atlas allocation failed"))?;
        pixels.resize(layout.pixels(), [0; 4]);
        layout
            .pack(sheet, &selections, &mut pixels)
            .map_err(|error| format!("atlas pack: {error:?}"))?;
        Ok(PreparedAtlas { layout, pixels })
    }

    /// Verify a local bundle and exercise two native presentations.
    /// # Errors
    /// Rejects invalid data, exceeded budgets or unequal logical pixels.
    pub fn run(&self, machine: Machine) -> Result<(), String> {
        let started = now().map_err(|error| error.to_string())?;
        let bundle = CanonicalBundle::open(
            &self.parent,
            self.digest,
            machine.graphics_limits(),
        )
        .map_err(|error| format!("bundle: {error:?}"))?;
        machine
            .validate_manifest(bundle.manifest())
            .map_err(|error| format!("profile manifest: {error:?}"))?;
        let record = bundle
            .manifest()
            .record(&self.key)
            .ok_or_else(|| String::from("unknown indexed asset key"))?;
        let bytes = bundle
            .read_payload(&self.key)
            .map_err(|error| format!("asset: {error:?}"))?;
        let sheet = IndexedTiles::new(record, &bytes)
            .map_err(|error| format!("indexed asset: {error:?}"))?;
        let raw =
            snapshot(&self.raw, reference_palette::byte_limit(machine, 2)?)
                .map_err(|error| format!("palette RAM: {error:?}"))?;
        let colors = reference_palette::decode(machine, &raw)?;
        let atlas = self.prepare(&sheet, &colors)?;
        let execution = atlas
            .execute(self.count, self.benchmark_frames, started)
            .map_err(|error| format!("native renderer: {error:?}"))?;
        if let Some(collector) = execution.performance {
            return render_benchmark::report(
                &collector,
                &execution.renderer,
                &format!("{} sprites", self.count),
            );
        }
        let renderer = execution.renderer;
        toolchain::write_line(&format!(
            "Verified {} tiles, {} RGBA bytes, two frames ({renderer})",
            self.count,
            atlas.layout.bytes(),
        ))
        .map_err(|error| error.to_string())
    }
}

struct Execution {
    performance: Option<PerformanceCollector>,
    renderer: String,
}

struct PreparedAtlas {
    layout: TileAtlasLayout,
    pixels: Vec<[u8; 4]>,
}

impl PreparedAtlas {
    fn check_frame(
        &self,
        scene: &mut SceneSession<'_, '_>,
        commands: &[SpriteDraw],
    ) -> Result<(), RenderError> {
        present(scene, commands)?;
        let pixels = scene.readback()?;
        let matched = pixels.as_chunks::<4>().0.iter().zip(&self.pixels).all(
            |(actual, expected)| {
                let [red, green, blue, alpha] = *expected;
                let opaque = if alpha == 0 {
                    [0, 0, 0, 255]
                } else {
                    [red, green, blue, alpha]
                };
                actual == &opaque
            },
        );
        if !matched || pixels.len() != self.layout.bytes() {
            return Err(RenderError::Backend(String::from(
                "logical target does not match the packed atlas",
            )));
        }
        Ok(())
    }

    fn commands(
        &self,
        id: AtlasId,
        count: NonZeroU32,
    ) -> Result<Vec<SpriteDraw>, RenderError> {
        (0..count.get())
            .map(|slot| {
                let rect =
                    self.layout.rect(slot).ok_or(RenderError::InvalidSprite)?;
                let sprite_rect = SpriteRect {
                    height: rect.height,
                    width: rect.width,
                    x: i32::try_from(rect.x)
                        .map_err(|_error| RenderError::InvalidDimensions)?,
                    y: i32::try_from(rect.y)
                        .map_err(|_error| RenderError::InvalidDimensions)?,
                };
                Ok(SpriteDraw {
                    atlas: id,
                    destination: sprite_rect,
                    flip_horizontal: false,
                    flip_vertical: false,
                    source: sprite_rect,
                })
            })
            .collect()
    }

    fn execute(
        &self,
        count: NonZeroU32,
        benchmark_frames: Option<NonZeroU32>,
        started: Duration,
    ) -> Result<Execution, RenderError> {
        let layout = FrameLayout::new(
            self.layout.width(),
            self.layout.height(),
            ATLAS_BYTES,
        )?;
        let mut window = NativeWindow::open(
            "Imported tile verification",
            layout.width(),
            layout.height(),
            true,
        )?;
        let renderer = window.renderer_name();
        let limits = SceneLimits {
            atlas_bytes: ATLAS_BYTES,
            atlases: NonZeroUsize::MIN,
            sprites: NonZeroUsize::try_from(count)
                .map_err(|_error| RenderError::TooLarge)?,
        };
        window.with_scene(layout, limits, |scene| {
            let id = scene.load_atlas(layout, self.pixels.as_flattened())?;
            let commands = self.commands(id, count)?;
            let loaded = now()
                .map_err(|error| RenderError::Backend(error.to_string()))?;
            self.check_frame(scene, &commands)?;
            let performance = if let Some(frames) = benchmark_frames {
                Some(render_benchmark::measure(
                    scene,
                    &commands,
                    layout,
                    Schedule { frames, loaded, started },
                )?)
            } else {
                self.check_frame(scene, &commands)?;
                None
            };
            Ok(Execution { performance, renderer })
        })
    }
}

fn number<Value>(value: &OsString) -> Result<Value, String>
where
    Value: FromStr,
{
    text(value)?
        .parse()
        .map_err(|_error| String::from("invalid render-check argument"))
}

pub(super) fn present(
    scene: &mut SceneSession<'_, '_>,
    commands: &[SpriteDraw],
) -> Result<(), RenderError> {
    if !scene.present([0; 3], commands)? {
        return Err(RenderError::Backend(String::from("no drawable")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use games_assets::{AssetFormat, AssetRecord};

    use super::*;

    fn args(first: &str, count: &str, columns: &str) -> Vec<OsString> {
        let args = [
            "parent",
            &"0".repeat(64),
            "graphics/tiles",
            "palette.bin",
            first,
            count,
            columns,
            "0",
            "16",
        ];
        args.map(OsString::from).to_vec()
    }

    fn parse(
        first: &str,
        count: &str,
        columns: &str,
    ) -> Result<Arguments, String> {
        Arguments::parse(args(first, count, columns))
    }

    #[test]
    fn prepares_tiles_and_rejects_invalid_ranges() -> Result<(), String> {
        let indices = [0, 1, 1, 0, 1, 0];
        let record = AssetRecord {
            digest: ContentHash::calculate(&indices),
            format: AssetFormat::IndexedTiles8 {
                height: NonZeroU32::MIN,
                tiles: NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN),
                width: NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
            },
            key: String::from("graphics/test"),
            size: 6,
        };
        let sheet = IndexedTiles::new(&record, &indices)
            .map_err(|error| format!("{error:?}"))?;
        let mut args = parse("0", "3", "2")?;
        args.pen_count = NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN);
        args.pen_start = 1;
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        let clear = [0; 4];
        let palette = [clear, red, blue];
        let atlas = args.prepare(&sheet, &palette)?;
        if atlas.pixels != [red, blue, blue, red, blue, red, clear, clear] {
            return Err(String::from("packed colors differ"));
        }
        args.first = 1;
        if args.prepare(&sheet, &palette).is_ok() {
            return Err(String::from("out-of-range selection accepted"));
        }
        args.first = 0;
        args.pen_start = 2;
        if args.prepare(&sheet, &palette).is_ok() {
            return Err(String::from("out-of-range selection accepted"));
        }
        Ok(())
    }

    #[test]
    fn benchmark_checks_frame_budget() {
        for (frames, valid) in
            [("0", false), ("1", true), ("20000", true), ("20001", false)]
        {
            let mut values = args("0", "1", "1");
            values.push(OsString::from(frames));
            assert_eq!(Arguments::parse_benchmark(values).is_ok(), valid);
        }
    }

    #[test]
    fn selection_checks_overflow_and_limits() {
        assert!(parse("0", "1024", "32").is_ok());
        assert!(parse("0", "1025", "32").is_err());
        assert!(parse("0", "1", "2").is_err());
        assert!(parse("4294967295", "1", "1").is_err());
        assert!(parse("0", "0", "1").is_err());
        assert!(Arguments::parse(Vec::new()).is_err());
    }
}
