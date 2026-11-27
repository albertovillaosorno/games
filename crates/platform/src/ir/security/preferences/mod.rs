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
//   - Portable audio, input and locale preference payload formats and
//     admission.
// - Must-Not:
//   - Access storage, resolve device bindings or change title state.
// - Allows:
//   - Inputs: Bounded memory-resident preference payloads and values.
//   - Outputs: Canonical encodings or typed payload rejections.
//   - Side effects: Bounded allocations for encoded preference text.
// - Split-When:
//   - An independent preference format requires a separate contract.
// - Merge-When:
//   - A shared settings codec owns identical version and size rules.
// - Summary:
//   - Shared, versioned preference formats independent of native storage.
// - Description:
//   - Keeps invalid and incompatible payloads distinct from missing data.
// - Usage:
//   - Validate settings before publishing or applying them.
// - Defaults:
//   - Reject malformed UTF-8, empty profiles and excess profile bytes.

//! Portable preference payload admission.

use std::num::NonZeroU64;

use games::audio::{AudioSource, MasterVolume};
use games::presentation::LocaleTag;

const VOLUME_PREFIX: &str = "master-volume-v1\n";
const EFFECTS_PREFIX: &str = "effects-volume-v1\n";
const MUSIC_PREFIX: &str = "music-volume-v1\n";
const CONTROL_PREFIX: &str = "input-profile-v1\n";
const LOCALE_PREFIX: &str = "locale-v1\n";

/// Rejection of an incompatible, malformed or oversized preference payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidPreference;

/// Decode a versioned master-volume preference.
///
/// # Errors
/// Rejects incompatible versions, malformed UTF-8 and invalid volume values.
pub fn decode_volume(bytes: &[u8]) -> Result<MasterVolume, InvalidPreference> {
    decode_level(bytes, VOLUME_PREFIX)
}

/// Encode a versioned master-volume preference.
#[must_use]
pub fn encode_volume(volume: MasterVolume) -> String {
    format!("{VOLUME_PREFIX}{}", volume.percent())
}

/// Decode a versioned preference for exactly the requested source.
/// # Errors
/// Rejects incompatible versions, source identities and invalid values.
pub fn decode_source_volume(
    bytes: &[u8],
    source: AudioSource,
) -> Result<MasterVolume, InvalidPreference> {
    decode_level(bytes, source_prefix(source))
}

/// Encode a versioned source-volume preference.
#[must_use]
pub fn encode_source_volume(
    source: AudioSource,
    volume: MasterVolume,
) -> String {
    format!("{}{}", source_prefix(source), volume.percent())
}

fn decode_level(
    bytes: &[u8],
    prefix: &str,
) -> Result<MasterVolume, InvalidPreference> {
    let text = str::from_utf8(bytes).map_err(|_error| InvalidPreference)?;
    let value = text.strip_prefix(prefix).ok_or(InvalidPreference)?;
    MasterVolume::parse(value).map_err(|_error| InvalidPreference)
}

const fn source_prefix(source: AudioSource) -> &'static str {
    match source {
        AudioSource::Effects => EFFECTS_PREFIX,
        AudioSource::Music => MUSIC_PREFIX,
    }
}

/// Compute the whole encoded byte bound for an input profile.
///
/// # Errors
/// Rejects overflow in the payload plus its format prefix.
pub fn control_payload_limit(
    maximum: NonZeroU64,
) -> Result<NonZeroU64, InvalidPreference> {
    let prefix = u64::try_from(CONTROL_PREFIX.len())
        .map_err(|_error| InvalidPreference)?;
    maximum
        .get()
        .checked_add(prefix)
        .and_then(NonZeroU64::new)
        .ok_or(InvalidPreference)
}

/// Decode a bounded, nonempty input profile without parsing title bindings.
///
/// # Errors
/// Rejects malformed UTF-8, incompatible versions and invalid profile length.
pub fn decode_control(
    bytes: &[u8],
    maximum: NonZeroU64,
) -> Result<&str, InvalidPreference> {
    let text = str::from_utf8(bytes).map_err(|_error| InvalidPreference)?;
    let profile = text.strip_prefix(CONTROL_PREFIX).ok_or(InvalidPreference)?;
    admit_control(profile, maximum)?;
    Ok(profile)
}

/// Encode admitted profile text using the stable on-disk version.
///
/// # Errors
/// Rejects empty or oversized input and unrepresentable allocation.
pub fn encode_control(
    profile: &str,
    maximum: NonZeroU64,
) -> Result<String, InvalidPreference> {
    admit_control(profile, maximum)?;
    let size = CONTROL_PREFIX
        .len()
        .checked_add(profile.len())
        .ok_or(InvalidPreference)?;
    let mut payload = String::new();
    payload
        .try_reserve_exact(size)
        .map_err(|_error| InvalidPreference)?;
    payload.push_str(CONTROL_PREFIX);
    payload.push_str(profile);
    Ok(payload)
}

/// Compute the maximum encoded locale preference size.
/// # Errors
/// Rejects an unrepresentable payload bound.
pub fn locale_payload_limit() -> Result<NonZeroU64, InvalidPreference> {
    let size = LOCALE_PREFIX
        .len()
        .checked_add(LocaleTag::MAX_BYTES)
        .ok_or(InvalidPreference)?;
    u64::try_from(size)
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or(InvalidPreference)
}

/// Decode a locale without applying any title-specific language policy.
/// # Errors
/// Rejects incompatible versions, invalid UTF-8 or invalid locale tags.
pub fn decode_locale(bytes: &[u8]) -> Result<LocaleTag, InvalidPreference> {
    let text = str::from_utf8(bytes).map_err(|_error| InvalidPreference)?;
    let tag = text.strip_prefix(LOCALE_PREFIX).ok_or(InvalidPreference)?;
    LocaleTag::parse(tag).map_err(|_error| InvalidPreference)
}

/// Encode a validated canonical locale identifier.
#[must_use]
pub fn encode_locale(locale: &LocaleTag) -> String {
    format!("{LOCALE_PREFIX}{}", locale.as_str())
}

fn admit_control(
    profile: &str,
    maximum: NonZeroU64,
) -> Result<(), InvalidPreference> {
    if profile.is_empty()
        || u64::try_from(profile.len())
            .map_or(true, |size| size > maximum.get())
    {
        return Err(InvalidPreference);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_codec_normalizes_tags_and_rejects_other_payloads()
    -> Result<(), InvalidPreference> {
        let locale =
            LocaleTag::parse("ES-mx").map_err(|_error| InvalidPreference)?;
        if encode_locale(&locale) != "locale-v1\nes-MX"
            || decode_locale(b"locale-v1\nES-mx")? != locale
        {
            return Err(InvalidPreference);
        }
        for bytes in [
            b"locale-v2\nes-MX".as_slice(),
            b"locale-v1\nes_MX",
            b"locale-v1\n",
            b"locale-v1\n../en",
            b"locale-v1\nen\n",
            b"locale-v1\n\xff",
            b"master-volume-v1\n100",
        ] {
            if decode_locale(bytes) != Err(InvalidPreference) {
                return Err(InvalidPreference);
            }
        }
        let oversized = format!(
            "locale-v1\n{}",
            "a".repeat(LocaleTag::MAX_BYTES.saturating_add(1))
        );
        if decode_locale(oversized.as_bytes()) != Err(InvalidPreference) {
            return Err(InvalidPreference);
        }
        if locale_payload_limit()?.get()
            != u64::try_from(
                LOCALE_PREFIX.len().saturating_add(LocaleTag::MAX_BYTES),
            )
            .map_err(|_error| InvalidPreference)?
        {
            return Err(InvalidPreference);
        }
        Ok(())
    }

    fn limit(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).unwrap_or(NonZeroU64::MIN)
    }

    #[test]
    fn volume_codec_preserves_existing_payload_and_rejects_other_types()
    -> Result<(), String> {
        for percent in ["0", "25", "50", "75", "100"] {
            let volume = MasterVolume::parse(percent)
                .map_err(|error| format!("{error:?}"))?;
            let expected = format!("master-volume-v1\n{percent}");
            let encoded = encode_volume(volume);
            if encoded != expected
                || decode_volume(encoded.as_bytes()) != Ok(volume)
            {
                return Err(String::from("volume codec changed its format"));
            }
        }
        for bytes in [
            b"master-volume-v1\n101".as_slice(),
            b"master-volume-v1\n1",
            b"master-volume-v1\n50\n",
            b"master-volume-v2\n50",
            b"master-volume-v1\n\xff",
            b"input-profile-v1\n50",
        ] {
            if decode_volume(bytes) != Err(InvalidPreference) {
                return Err(String::from("invalid volume was admitted"));
            }
        }
        Ok(())
    }

    #[test]
    fn source_volume_codec_rejects_cross_source_and_master_payloads() {
        for level in MasterVolume::ALL {
            let effect = encode_source_volume(AudioSource::Effects, level);
            let music = encode_source_volume(AudioSource::Music, level);
            for (source, valid, invalid) in [
                (AudioSource::Effects, &effect, &music),
                (AudioSource::Music, &music, &effect),
            ] {
                assert_eq!(
                    decode_source_volume(valid.as_bytes(), source),
                    Ok(level)
                );
                assert_eq!(
                    decode_source_volume(invalid.as_bytes(), source),
                    Err(InvalidPreference)
                );
                assert_eq!(
                    decode_source_volume(
                        encode_volume(level).as_bytes(),
                        source
                    ),
                    Err(InvalidPreference)
                );
                assert_eq!(
                    decode_volume(valid.as_bytes()),
                    Err(InvalidPreference)
                );
            }
        }
    }

    #[test]
    fn profile_codec_counts_utf8_bytes_and_preserves_exact_text()
    -> Result<(), String> {
        let text = "\u{e9}\n";
        let maximum = limit(3);
        let encoded = encode_control(text, maximum)
            .map_err(|error| format!("{error:?}"))?;
        if encoded != "input-profile-v1\n\u{e9}\n"
            || decode_control(encoded.as_bytes(), maximum) != Ok(text)
            || decode_control(encoded.as_bytes(), limit(2))
                != Err(InvalidPreference)
            || encode_control(text, limit(2)) != Err(InvalidPreference)
            || encode_control("", maximum) != Err(InvalidPreference)
            || control_payload_limit(limit(u64::MAX)) != Err(InvalidPreference)
        {
            return Err(String::from("inconsistent profile byte admission"));
        }
        let bound = control_payload_limit(maximum)
            .map_err(|error| format!("{error:?}"))?;
        if u64::try_from(encoded.len()).ok() != Some(bound.get()) {
            return Err(String::from("incorrect encoded byte bound"));
        }
        for bytes in [
            b"input-profile-v1\n".as_slice(),
            b"input-profile-v1\n\xff",
            b"input-profile-v2\na",
            b"master-volume-v1\n1",
        ] {
            if decode_control(bytes, maximum) != Err(InvalidPreference) {
                return Err(String::from("invalid profile was admitted"));
            }
        }
        Ok(())
    }
}
