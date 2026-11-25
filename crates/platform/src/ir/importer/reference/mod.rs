// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
// Boundary-Contract:
// - Owns:
//   - Read-only original-format observation interfaces for research.
// - Must-Not:
//   - Execute originals, publish data or expose conversion dispatch.
// - Allows:
//   - Inputs: Bounded observations from independently recorded references.
//   - Outputs: Typed decoded observations and comparison results.
//   - Side effects: None.
// - Split-When:
//   - A reference family requires a distinct observation contract.
// - Merge-When:
//   - Another observation interface owns the same reference format.
// - Summary:
//   - Reference diagnostic types exposed through the import IR.
// - Description:
//   - Keeps format research separate from canonical import requests.
// - Usage:
//   - Compare lawful local observations with the pure format decoder.
// - Defaults:
//   - Reject invalid observations; never invent missing reference data.

//! Format-specific reference observations for contributor diagnostics.

pub use games_importer::mame::Machine;
pub use games_importer::mame::background::{Background, Pixel, Raster};
pub use games_importer::mame::objects::RAM_BYTES;
pub use games_importer::mame::priority::VideoPriority;
pub use games_importer::mame::samples::compare_observations;
pub use games_importer::mame::tilemap::TilemapSnapshot;
pub use games_importer::wave::{WAVE_HEADER_BYTES, WavePcm};
