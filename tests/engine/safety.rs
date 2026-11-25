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
//   - Synthetic archive traversal, link, size and bad-save tests.
// - Must-Not:
//   - Execute imported code, read original media or extract archives.
// - Allows:
//   - Inputs: Fake archive metadata and in-memory save buffers.
//   - Outputs: Public acceptance, bounds and integrity assertions.
//   - Side effects: None outside local test memory.
// - Split-When:
//   - Real importer filesystem extraction gains separate fixtures.
// - Merge-When:
//   - One generic hostile-input harness covers all tested invariants.
// - Summary:
//   - Hostile asset metadata and save regression tests.
// - Description:
//   - Checks fail-closed validation before executing risky operations.
// - Usage:
//   - cargo validate
// - Defaults:
//   - No file permissions, archive extraction or original game data.
//

use std::num::NonZeroU64;

use super::{EntryKind, SafetyError, decode_save, encode_save, validate_entry};

fn limit(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap_or(NonZeroU64::MIN)
}

#[test]
fn archive_entries_reject_traversal_drive_paths_and_control_codes() {
    for name in [
        "../escape",
        "a/../../escape",
        "/etc/passwd",
        "C:/Windows",
        "\\\\server\\path",
        "a\\..\\evil",
        "a//b",
        "a/./b",
        "a/",
        "",
        "a\0b",
        "disk:stream",
    ] {
        assert_eq!(
            validate_entry(name, EntryKind::File, 1, limit(100)),
            Err(SafetyError::UnsafePath),
            "{name:?}",
        );
    }
    assert_eq!(
        validate_entry("textures/menu.png", EntryKind::File, 10, limit(100)),
        Ok(()),
    );
}

#[test]
fn links_special_files_and_executables_are_rejected() {
    for kind in [EntryKind::Symlink, EntryKind::Hardlink, EntryKind::Other] {
        assert_eq!(
            validate_entry("data/icon.png", kind, 2, limit(100)),
            Err(SafetyError::LinkOrSpecial),
        );
    }
    for name in ["run.EXE", "scripts/start.sh", "mod.wasm", "plugin.DLL"] {
        assert_eq!(
            validate_entry(name, EntryKind::File, 2, limit(100)),
            Err(SafetyError::Executable),
        );
    }
}

#[test]
fn oversized_files_are_rejected_before_allocation() {
    assert_eq!(
        validate_entry("textures/huge.png", EntryKind::File, 101, limit(100)),
        Err(SafetyError::TooLarge),
    );
    assert_eq!(
        encode_save(b"oversize", limit(2)),
        Err(SafetyError::TooLarge),
    );
}

#[test]
fn valid_save_roundtrips_and_corruption_is_rejected() {
    let original = b"checkpoint;health=100;score=4";
    let encoded = encode_save(original, limit(1024));
    assert!(encoded.is_ok(), "payload fits the configured budget");
    if let Ok(save) = encoded {
        assert_eq!(decode_save(&save, limit(1024)), Ok(original.as_slice()));
        assert_eq!(decode_save(&save, limit(2)), Err(SafetyError::TooLarge));
        for index in [0, 4, 8, 39, 40] {
            let mut modified = save.clone();
            if let Some(byte) = modified.get_mut(index) {
                *byte ^= 1;
            }
            assert_eq!(
                decode_save(&modified, limit(1024)),
                Err(SafetyError::InvalidSave),
            );
        }
        assert_eq!(
            decode_save(save.get(..3).unwrap_or(&[]), limit(1024)),
            Err(SafetyError::InvalidSave)
        );
    }
}

#[test]
fn every_truncated_save_prefix_is_rejected_without_panicking() {
    let encoded = encode_save(b"durable state", limit(1024));
    assert!(encoded.is_ok(), "save fixture must be encodable");
    if let Ok(save) = encoded {
        for prefix_length in 0..save.len() {
            if let Some(prefix) = save.get(..prefix_length) {
                assert_eq!(
                    decode_save(prefix, limit(1024)),
                    Err(SafetyError::InvalidSave),
                );
            }
        }
        assert_eq!(
            decode_save(&save, limit(1024)),
            Ok(b"durable state".as_slice())
        );
    }
}

#[test]
fn an_empty_save_is_valid_only_with_an_intact_envelope() {
    let encoded = encode_save(b"", limit(1));
    assert!(encoded.is_ok(), "empty save is valid data");
    if let Ok(mut save) = encoded {
        assert_eq!(decode_save(&save, limit(1)), Ok(b"".as_slice()));
        if let Some(magic) = save.get_mut(3) {
            *magic = b'0';
        }
        assert_eq!(decode_save(&save, limit(1)), Err(SafetyError::InvalidSave));
    }
}
