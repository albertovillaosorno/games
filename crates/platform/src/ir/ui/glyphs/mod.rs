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
//   - Compact bitmap glyph coverage and canonical Spanish composition.
// - Must-Not:
//   - Own locale selection, message formatting or platform font discovery.
// - Allows:
//   - Inputs: Unicode text or an admitted compact glyph.
//   - Outputs: Display glyphs, atlas indices and bitmap rows.
//   - Side effects: None.
// - Split-When:
//   - Additional scripts require shaping or variable-width glyphs.
// - Merge-When:
//   - Another compact UI renderer duplicates glyph coverage or composition.
// - Summary:
//   - Share project-authored ASCII and Mexican Spanish glyphs.
// - Description:
//   - Preserve uppercase bitmap styling and show a replacement for missing
//     glyphs. Compose supported decomposed accents before measurement.
// - Usage:
//   - Iterate display glyphs before measuring, rasterizing or indexing.
// - Defaults:
//   - Unsupported characters remain visibly represented.

use std::iter;

const REPLACEMENT: char = '\u{fffd}';
const EXTRA: [char; 10] = [
    '\u{c1}',
    '\u{c9}',
    '\u{cd}',
    '\u{d3}',
    '\u{da}',
    '\u{dc}',
    '\u{d1}',
    '\u{a1}',
    '\u{bf}',
    REPLACEMENT,
];
const ASCII_COUNT: usize = 95;
pub(super) const COUNT: usize = ASCII_COUNT + EXTRA.len();

#[derive(Clone, Copy, Debug)]
pub(super) struct Glyph {
    pub(super) end: usize,
    pub(super) source: char,
    pub(super) start: usize,
    pub(super) value: char,
}

pub(super) fn labels() -> impl Iterator<Item = char> {
    (b' '..=b'~').map(char::from).chain(EXTRA)
}

pub(super) fn tile(letter: char) -> usize {
    if let Ok(byte @ b' '..=b'~') = u8::try_from(letter) {
        return usize::from(byte.saturating_sub(b' '));
    }
    ASCII_COUNT.saturating_add(
        EXTRA
            .iter()
            .position(|candidate| *candidate == letter)
            .unwrap_or_else(|| EXTRA.len().saturating_sub(1)),
    )
}

pub(super) fn display(text: &str) -> impl Iterator<Item = char> + '_ {
    spans(text).map(|glyph| glyph.value)
}

pub(super) fn first_missing(text: &str) -> Option<char> {
    spans(text).find_map(|glyph| {
        (glyph.value == REPLACEMENT && glyph.source != REPLACEMENT)
            .then_some(glyph.source)
    })
}

pub(super) fn spans(text: &str) -> impl Iterator<Item = Glyph> + '_ {
    let mut characters = text.char_indices().peekable();
    iter::from_fn(move || {
        let (start, source) = characters.next()?;
        let letter = normalize(source);
        let value = characters
            .peek()
            .and_then(|(_offset, mark)| compose(letter, *mark))
            .map_or(letter, |composed| {
                let _consumed = characters.next();
                composed
            });
        let end = characters.peek().map_or(text.len(), |(offset, _)| *offset);
        Some(Glyph {
            end,
            source,
            start,
            value,
        })
    })
}

const fn normalize(letter: char) -> char {
    match letter {
        ' '..='~' => letter.to_ascii_uppercase(),
        '\u{e1}' | '\u{c1}' => '\u{c1}',
        '\u{e9}' | '\u{c9}' => '\u{c9}',
        '\u{ed}' | '\u{cd}' => '\u{cd}',
        '\u{f3}' | '\u{d3}' => '\u{d3}',
        '\u{fa}' | '\u{da}' => '\u{da}',
        '\u{fc}' | '\u{dc}' => '\u{dc}',
        '\u{f1}' | '\u{d1}' => '\u{d1}',
        '\u{a1}' | '\u{bf}' => letter,
        _ => REPLACEMENT,
    }
}

const fn compose(letter: char, mark: char) -> Option<char> {
    match (letter, mark) {
        ('A', '\u{301}') => Some('\u{c1}'),
        ('E', '\u{301}') => Some('\u{c9}'),
        ('I', '\u{301}') => Some('\u{cd}'),
        ('O', '\u{301}') => Some('\u{d3}'),
        ('U', '\u{301}') => Some('\u{da}'),
        ('U', '\u{308}') => Some('\u{dc}'),
        ('N', '\u{303}') => Some('\u{d1}'),
        _ => None,
    }
}

pub(super) fn rows(letter: char) -> [u8; 5] {
    if let Ok(byte @ b' '..=b'~') = u8::try_from(letter) {
        return ascii_rows(byte);
    }
    match letter {
        '\u{c1}' => [1, 2, 5, 7, 5],
        '\u{c9}' => [1, 7, 6, 4, 7],
        '\u{cd}' => [1, 7, 2, 2, 7],
        '\u{d3}' => [1, 7, 5, 5, 7],
        '\u{da}' => [1, 5, 5, 5, 7],
        '\u{dc}' => [5, 0, 5, 5, 7],
        '\u{d1}' => [3, 6, 5, 7, 5],
        '\u{a1}' => [2, 0, 2, 2, 2],
        '\u{bf}' => [2, 0, 2, 4, 7],
        _ => [7, 5, 2, 5, 7],
    }
}
const fn ascii_rows(letter: u8) -> [u8; 5] {
    match letter {
        b'0' | b'O' => [7, 5, 5, 5, 7],
        b'1' => [2, 6, 2, 2, 7],
        b'2' => [7, 1, 7, 4, 7],
        b'3' => [7, 1, 7, 1, 7],
        b'4' => [5, 5, 7, 1, 1],
        b'5' | b'S' => [7, 4, 7, 1, 7],
        b'6' => [7, 4, 7, 5, 7],
        b'7' => [7, 1, 2, 2, 2],
        b'8' => [7, 5, 7, 5, 7],
        b'9' => [7, 5, 7, 1, 7],
        b'A' => [2, 5, 7, 5, 5],
        b'B' => [6, 5, 6, 5, 6],
        b'C' => [7, 4, 4, 4, 7],
        b'D' => [6, 5, 5, 5, 6],
        b'E' => [7, 4, 6, 4, 7],
        b'F' => [7, 4, 6, 4, 4],
        b'G' => [7, 4, 5, 5, 7],
        b'H' => [5, 5, 7, 5, 5],
        b'I' => [7, 2, 2, 2, 7],
        b'J' => [1, 1, 1, 5, 7],
        b'K' => [5, 5, 6, 5, 5],
        b'L' => [4, 4, 4, 4, 7],
        b'M' => [5, 7, 7, 5, 5],
        b'N' => [5, 7, 7, 7, 5],
        b'P' => [6, 5, 6, 4, 4],
        b'Q' => [7, 5, 5, 7, 1],
        b'R' => [6, 5, 6, 5, 5],
        b'T' => [7, 2, 2, 2, 2],
        b'U' => [5, 5, 5, 5, 7],
        b'V' => [5, 5, 5, 5, 2],
        b'W' => [5, 5, 7, 7, 5],
        b'X' => [5, 5, 2, 5, 5],
        b'Y' => [5, 5, 2, 2, 2],
        b'Z' => [7, 1, 2, 4, 7],
        _ => symbol_rows(letter),
    }
}

const fn symbol_rows(letter: u8) -> [u8; 5] {
    match letter {
        b'!' => [2, 2, 2, 0, 2],
        b'"' => [5, 5, 0, 0, 0],
        b'#' => [5, 7, 5, 7, 5],
        b'$' => [2, 7, 6, 3, 2],
        b'%' => [5, 1, 2, 4, 5],
        b'&' => [2, 5, 2, 5, 3],
        b'\'' => [2, 2, 0, 0, 0],
        b'(' => [1, 2, 2, 2, 1],
        b')' => [4, 2, 2, 2, 4],
        b'*' => [0, 5, 2, 5, 0],
        b'+' => [0, 2, 7, 2, 0],
        b',' => [0, 0, 0, 2, 4],
        b'-' => [0, 0, 7, 0, 0],
        b'.' => [0, 0, 0, 0, 2],
        b'/' => [1, 1, 2, 4, 4],
        b':' => [0, 2, 0, 2, 0],
        b';' => [0, 2, 0, 2, 4],
        b'<' => [1, 2, 4, 2, 1],
        b'=' => [0, 7, 0, 7, 0],
        b'>' => [4, 2, 1, 2, 4],
        b'@' => [7, 5, 7, 4, 7],
        b'[' => [3, 2, 2, 2, 3],
        b'\\' => [4, 4, 2, 1, 1],
        b']' => [6, 2, 2, 2, 6],
        b'^' => [2, 5, 0, 0, 0],
        b'_' => [0, 0, 0, 0, 7],
        b'\x60' => [4, 2, 0, 0, 0],
        b'{' => [1, 2, 6, 2, 1],
        b'|' => [2, 2, 2, 2, 2],
        b'}' => [4, 2, 3, 2, 4],
        b'~' => [0, 0, 3, 6, 0],
        b' ' => [0; 5],
        _ => [7, 1, 2, 0, 2],
    }
}
