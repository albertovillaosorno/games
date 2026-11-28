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
//   - Private Android software capability composition.
// - Must-Not:
//   - Expose JNI types or interpret gameplay and title media.
// - Allows:
//   - Inputs: Operating-system provided application resources.
//   - Outputs: Portable capability observations.
//   - Side effects: Explicit Android service access.
// - Split-When:
//   - A capability requires an independent Android adapter.
// - Merge-When:
//   - Existing software owners implement the same capability.
// - Summary:
//   - Android application entry and process adapters.
// - Description:
//   - Keeps operating-system selection outside the portable IR.
// - Usage:
//   - Select Android capabilities through the platform crate.
// - Defaults:
//   - Unavailable resources fail explicitly.

//! Private Android software adapter.

pub(super) mod process;
