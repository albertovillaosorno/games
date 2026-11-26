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
//   - Private physical device protocol implementation namespace.
// - Must-Not:
//   - Access host services, select operating systems or open devices.
// - Allows:
//   - Inputs: Portable admitted device protocol data.
//   - Outputs: Portable device reports and feedback data.
//   - Side effects: None.
// - Split-When:
//   - A verified physical device family has independent protocol semantics.
// - Merge-When:
//   - Device families share identical report and feedback contracts.
// - Summary:
//   - Host-independent physical device protocol ownership.
// - Description:
//   - Physical protocols decode admitted reports without opening devices.
// - Usage:
//   - Extend the shared IR and verified protocol fixtures first.
// - Defaults:
//   - No device family support is implied.

//! Private owner for physical device-specific protocol implementations.

mod keyboard;
