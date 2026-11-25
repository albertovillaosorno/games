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
//   - Bounded background comparison against private screen observations.
// - Must-Not:
//   - Implement hardware decoding, sprite rendering or claim full parity.
// - Allows:
//   - Inputs: Pinned bundle, tile state, palette RAM and captured RGB32.
//   - Outputs: Counts of matching and differing visible pixels.
//   - Side effects: Bounded local file reads and stdout summaries.
// - Split-When:
//   - Another observation format requires independent comparison.
// - Merge-When:
//   - Shared reference comparison can express the same inputs.
// - Summary:
//   - Measure the observed background against a captured complete frame.
// - Description:
//   - Missing sprites may contribute differences; no tolerance is inferred.
// - Usage:
//   - Supply all observations from the same original-machine frame.
// - Defaults:
//   - Exact file lengths, read-only and no private image publication.

//! Host-side comparison of original background and complete screen pixels.

use std::ffi::OsString;
use std::io::{Write as _, stdout};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use games_assets::ContentHash;
use games_assets::indexed::IndexedTiles;
use games_importer::mame::Machine;
use games_importer::mame::background::Background;
use games_platform_storage::bundle::CanonicalBundle;
use games_platform_storage::file::snapshot;

use crate::{USAGE, reference_palette, text};

/// Explicit paths to a single bounded, private original observation.
#[derive(Debug)]
pub struct Arguments {
    digest: ContentHash,
    frame: PathBuf,
    palette: PathBuf,
    parent: PathBuf,
    ram: PathBuf,
    registers: PathBuf,
    scroll: PathBuf,
}

impl Arguments {
    fn compare(
        &self,
        machine: Machine,
        tiles: &IndexedTiles<'_>,
    ) -> Result<(), String> {
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
        let raster = machine.background_raster();
        let mut pens = vec![0; raster.pixels()];
        raster
            .compose(
                Background {
                    state,
                    scroll_control: *control,
                    tiles,
                },
                &mut pens,
            )
            .map_err(|error| format!("background: {error:?}"))?;
        let raw =
            snapshot(&self.palette, reference_palette::byte_limit(machine, 2)?)
                .map_err(|error| format!("palette RAM: {error:?}"))?;
        let colors = reference_palette::decode(machine, &raw)?;
        let frame = read(&self.frame, raster.pixels().saturating_mul(4))?;
        let matched = count_matches(&pens, &colors, &frame)?;
        writeln!(
            stdout().lock(),
            "Background-only: {matched}/{} pixels match; {} differ",
            pens.len(),
            pens.len().saturating_sub(matched),
        )
        .map_err(|error| error.to_string())
    }

    /// Parse exact bundle and observation arguments.
    /// # Errors
    /// Rejects malformed or missing arguments.
    pub fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let [parent, digest, ram, registers, scroll, palette, frame] =
            <[OsString; 7]>::try_from(args)
                .map_err(|_args| String::from(USAGE))?;
        Ok(Self {
            digest: text(&digest)?
                .parse()
                .map_err(|_error| String::from("invalid manifest hash"))?,
            frame: frame.into(),
            palette: palette.into(),
            parent: parent.into(),
            ram: ram.into(),
            registers: registers.into(),
            scroll: scroll.into(),
        })
    }

    /// Compare a background reconstruction with a full original frame.
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
        let key = "graphics/tiles";
        let record = bundle
            .manifest()
            .record(key)
            .ok_or_else(|| String::from("missing background tiles"))?;
        let bytes = bundle
            .read_payload(key)
            .map_err(|error| format!("tiles: {error:?}"))?;
        let tiles = IndexedTiles::new(record, &bytes)
            .map_err(|error| format!("indexed tiles: {error:?}"))?;
        self.compare(machine, &tiles)
    }
}

fn count_matches(
    pens: &[u16],
    colors: &[[u8; 4]],
    frame: &[u8],
) -> Result<usize, String> {
    let (words, remainder) = frame.as_chunks::<4>();
    if !remainder.is_empty() || words.len() != pens.len() {
        return Err(String::from("incorrect frame length"));
    }
    let mut matched = 0usize;
    for (&pen, &[blue, green, red, alpha]) in pens.iter().zip(words) {
        let color = colors
            .get(usize::from(pen))
            .ok_or_else(|| String::from("background pen outside palette"))?;
        if *color == [red, green, blue, alpha] {
            matched = matched.saturating_add(1);
        }
    }
    Ok(matched)
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
    fn comparison_counts_only_exact_pixels_and_checks_extent() {
        let colors = [[1, 2, 3, 255], [4, 5, 6, 255]];
        let frame = [3, 2, 1, 255, 6, 5, 0, 255];
        assert_eq!(count_matches(&[0, 1], &colors, &frame), Ok(1));
        assert!(count_matches(&[0, 2], &colors, &frame).is_err());
        assert!(count_matches(&[0], &colors, &frame).is_err());
    }
}
