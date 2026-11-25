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
//   - Profiled palette-word decoding for observed arcade palette RAM.
// - Must-Not:
//   - Execute source code, infer palette assignment or access the host.
// - Allows:
//   - Inputs: Exact raw palette bytes and a caller-owned RGBA buffer.
//   - Outputs: Complete opaque RGBA entries in hardware pen order.
//   - Side effects: Writes the validated output without allocation.
// - Split-When:
//   - Additional palette encodings need independent decoder ownership.
// - Merge-When:
//   - Other source adapters duplicate packed palette-word decoding.
// - Summary:
//   - Convert palette RAM words through the selected source profile.
// - Description:
//   - Validate the extent and decode every word in its declared byte order.
// - Usage:
//   - Select the machine profile and provide an observed palette snapshot.
// - Defaults:
//   - Entries are opaque; transparency and shadow semantics are external.

//! Palette-word decoding selected by the source profile.

use std::num::NonZeroUsize;

use crate::ImportError;

type DecodeWord = fn([u8; 2]) -> [u8; 4];

#[derive(Clone, Copy, Debug)]
pub(super) struct Palette {
    pub(super) decode_word: DecodeWord,
    pub(super) entries: NonZeroUsize,
}

impl Palette {
    pub(super) fn decode(
        self,
        source: &[u8],
        output: &mut [[u8; 4]],
    ) -> Result<(), ImportError> {
        let bytes = self
            .entries
            .get()
            .checked_mul(2)
            .ok_or(ImportError::TooLarge)?;
        if source.len() != bytes || output.len() != self.entries.get() {
            return Err(ImportError::UnsupportedMember);
        }
        for (word, color) in source.as_chunks::<2>().0.iter().zip(output) {
            *color = (self.decode_word)(*word);
        }
        Ok(())
    }
}

const fn expand5(channel: u8) -> u8 {
    (channel << 3) | (channel >> 2)
}

pub(super) const fn xbgr555_be([upper, lower]: [u8; 2]) -> [u8; 4] {
    [
        expand5(lower & 0x1f),
        expand5(((upper & 0x03) << 3) | (lower >> 5)),
        expand5((upper >> 2) & 0x1f),
        255,
    ]
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::{Palette, xbgr555_be};
    use crate::ImportError;

    #[test]
    fn decodes_channels_and_ignores_the_unused_bit() {
        assert_eq!(xbgr555_be([0, 0x1f]), [255, 0, 0, 255]);
        assert_eq!(xbgr555_be([0x03, 0xe0]), [0, 255, 0, 255]);
        assert_eq!(xbgr555_be([0x7c, 0]), [0, 0, 255, 255]);
        assert_eq!(xbgr555_be([0xff, 0xff]), [255; 4]);
        assert_eq!(xbgr555_be([0x80, 0]), [0, 0, 0, 255]);
        assert_eq!(xbgr555_be([0x04, 0x21]), [8, 8, 8, 255]);
        assert_eq!(xbgr555_be([0x42, 0x10]), [132, 132, 132, 255]);
    }

    #[test]
    fn checks_complete_extent_before_writing() {
        let layout = Palette {
            decode_word: xbgr555_be,
            entries: NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN),
        };
        let mut output = [[42; 4]; 2];
        assert_eq!(
            layout.decode(&[0x7c, 0, 0], &mut output),
            Err(ImportError::UnsupportedMember)
        );
        assert_eq!(output, [[42; 4]; 2]);
        assert_eq!(layout.decode(&[0x7c, 0, 0, 0x1f], &mut output), Ok(()));
        assert_eq!(output, [[0, 0, 255, 255], [255, 0, 0, 255]]);
        let mut short = [[42; 4]; 1];
        assert_eq!(
            layout.decode(&[0x7c, 0, 0, 0x1f], &mut short),
            Err(ImportError::UnsupportedMember)
        );
        assert_eq!(short, [[42; 4]; 1]);
    }
}
