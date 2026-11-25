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
//   - Decode observed Konami priority-mixer registers by machine wiring.
// - Must-Not:
//   - Execute game code, infer tile pixels or implement scene simulation.
// - Allows:
//   - Inputs: Exact register snapshot and configured mixer input mapping.
//   - Outputs: Palette banks, priorities and ordered layer identities.
//   - Side effects: None.
// - Split-When:
//   - Another mixer requires a different register layout.
// - Merge-When:
//   - Existing source adapters support the same mixer contract.
// - Summary:
//   - Recover the observed palette-bank and layer-priority assignment.
// - Description:
//   - Interpret six-bit K053251 registers without retaining source media.
// - Usage:
//   - Decode a complete register snapshot through the selected machine.
// - Defaults:
//   - Reject incorrect register extents and out-of-range register values.

//! Bounded K053251 palette and priority observations.

use std::mem::swap;

use crate::ImportError;

/// One mixer input's palette bank and priority value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixerInput {
    /// Base in groups of sixteen ordinary palette entries.
    pub palette_bank: u8,
    /// Larger values place a tile layer farther behind.
    pub priority: u8,
}

/// Mixer observations with layer identities in machine wiring order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoPriority {
    /// Backdrop input; its base is the backdrop's first pen.
    pub backdrop: MixerInput,
    /// Tile layers in original layer-number order.
    pub layers: [MixerInput; 3],
    /// Sprite input; individual objects may have their own priorities.
    pub sprites: MixerInput,
}

impl VideoPriority {
    /// Layer identities ordered back to front, including hardware tie order.
    #[must_use]
    pub const fn ordered_layers(self) -> [usize; 3] {
        let [first, second, third] = self.layers;
        let mut back = (0, first.priority);
        let mut middle = (1, second.priority);
        let mut front = (2, third.priority);
        if back.1 < middle.1 {
            swap(&mut back, &mut middle);
        }
        if back.1 < front.1 {
            swap(&mut back, &mut front);
        }
        if middle.1 < front.1 {
            swap(&mut middle, &mut front);
        }
        [back.0, middle.0, front.0]
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Wiring {
    pub(super) backdrop: usize,
    pub(super) layers: [usize; 3],
    pub(super) sprites: usize,
}

impl Wiring {
    pub(super) fn decode(
        self,
        registers: &[u8],
    ) -> Result<VideoPriority, ImportError> {
        let inputs = inputs(registers)?;
        let select = |index| {
            inputs
                .get(index)
                .copied()
                .ok_or(ImportError::UnsupportedMember)
        };
        let [first, second, third] = self.layers;
        Ok(VideoPriority {
            backdrop: select(self.backdrop)?,
            layers: [select(first)?, select(second)?, select(third)?],
            sprites: select(self.sprites)?,
        })
    }
}

fn inputs(registers: &[u8]) -> Result<[MixerInput; 5], ImportError> {
    let &[p0, p1, p2, p3, p4, _, _, _, _, wide, narrow, _, _, _, _, _] =
        <&[u8; 16]>::try_from(registers)
            .map_err(|_error| ImportError::UnsupportedMember)?;
    if registers.iter().any(|value| *value > 63) {
        return Err(ImportError::UnsupportedMember);
    }
    let input = |priority, palette_bank| MixerInput { palette_bank, priority };
    Ok([
        input(p0, (wide & 3) << 5),
        input(p1, ((wide >> 2) & 3) << 5),
        input(p2, ((wide >> 4) & 3) << 5),
        input(p3, (narrow & 7) << 4),
        input(p4, ((narrow >> 3) & 7) << 4),
    ])
}

#[cfg(test)]
mod tests {
    use super::{MixerInput, VideoPriority, Wiring};

    const WIRING: Wiring = Wiring {
        backdrop: 0,
        layers: [2, 3, 4],
        sprites: 1,
    };

    #[test]
    fn decodes_independent_banks_and_priority_order() {
        let registers = [9, 8, 7, 2, 5, 0, 0, 0, 0, 45, 53, 0, 0, 0, 0, 0];
        let expected = VideoPriority {
            backdrop: MixerInput {
                palette_bank: 32,
                priority: 9,
            },
            layers: [
                MixerInput {
                    palette_bank: 64,
                    priority: 7,
                },
                MixerInput {
                    palette_bank: 80,
                    priority: 2,
                },
                MixerInput {
                    palette_bank: 96,
                    priority: 5,
                },
            ],
            sprites: MixerInput {
                palette_bank: 96,
                priority: 8,
            },
        };
        assert_eq!(WIRING.decode(&registers), Ok(expected));
        assert_eq!(expected.ordered_layers(), [0, 2, 1]);
    }

    #[test]
    fn tie_order_and_register_bounds_are_explicit() {
        let mut registers = [0; 16];
        assert_eq!(
            WIRING.decode(&registers).map(VideoPriority::ordered_layers),
            Ok([0, 1, 2])
        );
        if let Some(priority) = registers.get_mut(4) {
            *priority = 1;
        }
        assert_eq!(
            WIRING.decode(&registers).map(VideoPriority::ordered_layers),
            Ok([2, 1, 0])
        );
        assert!(WIRING.decode(&[0; 15]).is_err());
        assert!(WIRING.decode(&[64; 16]).is_err());
    }
}
