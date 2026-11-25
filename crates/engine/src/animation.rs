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
//   - Deterministic animation keyframe lookup and cycle timing.
// - Must-Not:
//   - Interpret title assets, character rules or platform clocks.
// - Allows:
//   - Inputs: Ordered keyframes, elapsed ticks and cycle duration.
//   - Outputs: The active frame or explicit absence.
//   - Side effects: None.
// - Split-When:
//   - Animation blending needs independent contracts.
// - Merge-When:
//   - Another shared timeline owns identical selection semantics.
// - Summary:
//   - Reusable sparse animation timelines.
// - Description:
//   - Select the latest frame without crossing animation group boundaries.
// - Usage:
//   - Supply title-defined animation keys and durations.
// - Defaults:
//   - Missing frames and zero duration produce no selection.

use std::collections::BTreeMap;

/// Sparse keyframes indexed by subject, track and caller-selected tick type.
pub type Keyframes<Subject, Track, Tick, Value> =
    BTreeMap<(Subject, Track, Tick), Value>;

/// Wrap elapsed simulation ticks into a nonempty caller-selected cycle.
#[must_use]
pub fn cycle_tick<Tick>(elapsed: u64, duration: Tick) -> Option<Tick>
where
    Tick: Into<u64> + TryFrom<u64>,
{
    Tick::try_from(elapsed.checked_rem(duration.into())?).ok()
}

/// Select the latest frame at or before the requested tick in one track.
/// Never fall back to a different subject or animation kind.
#[must_use]
pub fn keyframe<Subject, Track, Tick, Value>(
    entries: &Keyframes<Subject, Track, Tick, Value>,
    key: (Subject, Track, Tick),
) -> Option<&Value>
where
    Subject: Copy + Ord,
    Track: Copy + Ord,
    Tick: Ord,
{
    let (subject, track, _) = key;
    let ((owner, kind, _), value) = entries.range(..=key).next_back()?;
    (*owner == subject && *kind == track).then_some(value)
}

/// An absent optional track is valid; a present track needs every key.
#[must_use]
pub fn optional_track_complete<Subject, Track, Tick, Value>(
    entries: &Keyframes<Subject, Track, Tick, Value>,
    subject: Subject,
    track: Track,
    required: &[Tick],
) -> bool
where
    Subject: Copy + Ord,
    Track: Copy + Ord,
    Tick: Copy + Ord,
{
    !entries
        .keys()
        .any(|(owner, kind, _)| *owner == subject && *kind == track)
        || required
            .iter()
            .all(|&tick| entries.contains_key(&(subject, track, tick)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{cycle_tick, keyframe, optional_track_complete};

    #[test]
    fn long_timelines_and_optional_tracks_are_not_byte_limited() {
        let entries = BTreeMap::from([
            ((0u8, 0u8, 300u64), "first"),
            ((0, 0, 10_000), "second"),
            ((1, 0, 0), "other"),
        ]);
        assert_eq!(keyframe(&entries, (0, 0, 299)), None);
        assert_eq!(keyframe(&entries, (0, 0, 9_999)), Some(&"first"));
        assert_eq!(keyframe(&entries, (0, 0, u64::MAX)), Some(&"second"));
        assert!(optional_track_complete(&entries, 0, 0, &[300, 10_000]));
        assert!(!optional_track_complete(&entries, 0, 0, &[301]));
        assert!(optional_track_complete(&entries, 0, 1, &[0]));
        assert_eq!(cycle_tick(12_345, 10_000u64), Some(2_345));
        assert_eq!(cycle_tick(u64::MAX, u64::MAX), Some(0));
    }

    #[test]
    fn sparse_tracks_never_borrow_frames_from_other_tracks() {
        let entries = BTreeMap::from([
            ((0u8, 0u8, 0u8), "idle"),
            ((0, 1, 3), "first"),
            ((0, 1, 8), "second"),
            ((1, 1, 0), "other"),
        ]);
        assert_eq!(keyframe(&entries, (0, 1, 2)), None);
        assert_eq!(keyframe(&entries, (0, 1, 3)), Some(&"first"));
        assert_eq!(keyframe(&entries, (0, 1, 7)), Some(&"first"));
        assert_eq!(keyframe(&entries, (0, 1, 255)), Some(&"second"));
        assert_eq!(keyframe(&entries, (1, 0, 255)), None);
        assert_eq!(keyframe(&entries, (1, 1, 0)), Some(&"other"));
    }

    #[test]
    fn cycle_is_checked_at_exact_boundaries_and_large_ticks() {
        assert_eq!(cycle_tick(0, 0u8), None);
        assert_eq!(cycle_tick(9, 10u8), Some(9));
        assert_eq!(cycle_tick(10, 10u8), Some(0));
        assert_eq!(cycle_tick(21, 10u8), Some(1));
        assert_eq!(cycle_tick(u64::MAX, 1u8), Some(0));
        assert_eq!(cycle_tick(u64::MAX, 255u8), Some(0));
    }
}
