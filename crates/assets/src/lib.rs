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
//   - Canonical asset identities, manifest encoding and payload integrity.
// - Must-Not:
//   - Interpret source platforms, access the filesystem or execute assets.
// - Allows:
//   - Inputs: Asset descriptors, source identities and explicit limits.
//   - Outputs: Deterministic metadata and validated payload identities.
//   - Side effects: None.
// - Split-When:
//   - A separate canonical media representation needs its own contract.
// - Merge-When:
//   - Shared contracts duplicate asset identity or integrity validation.
// - Summary:
//   - Versioned canonical asset manifests for importers and runtimes.
// - Description:
//   - Identifies content-addressed payloads independently of source formats.
// - Usage:
//   - Select source layout, construct a manifest and validate asset bytes.
// - Defaults:
//   - Reject unknown versions, malformed keys and invalid payload lengths.

//! Source-independent canonical asset contracts.
/// Shared archive-entry policy and decompression budgets.
pub mod archive;
/// Integrity-checked runtime access to indexed graphics.
pub mod indexed;
mod manifest_codec;
/// Integrity-checked runtime access to interleaved PCM audio.
pub mod pcm;

use std::fmt::{self, Display, Formatter};
use std::num::{NonZeroU8, NonZeroU32, NonZeroU64, NonZeroUsize};
use std::str::{FromStr, from_utf8};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const VERSION: u16 = 2;
const MAX_KEY_BYTES: usize = 255;

/// SHA-256 content identity, encoded as 32 bytes in a canonical manifest.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Hash an existing byte slice.
    #[must_use]
    pub fn calculate(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Construct from an independently calculated SHA-256 digest.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl Display for ContentHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for ContentHash {
    type Err = AssetError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64
            || !value.bytes().all(|byte| {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            })
        {
            return Err(AssetError::InvalidIdentity);
        }
        let mut digest = [0; 32];
        for (pair, byte) in
            value.as_bytes().as_chunks::<2>().0.iter().zip(&mut digest)
        {
            let hex = from_utf8(pair)
                .map_err(|_error| AssetError::InvalidIdentity)?;
            *byte = u8::from_str_radix(hex, 16)
                .map_err(|_error| AssetError::InvalidIdentity)?;
        }
        Ok(Self(digest))
    }
}

/// Canonical payload encoding, independent of the original platform.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssetFormat {
    /// Title-defined declarative data, never executable source code.
    Data,
    /// Tile-major, row-major unsigned eight-bit palette indices.
    IndexedTiles8 {
        /// Height of each tile in pixels.
        height: NonZeroU32,
        /// Number of complete tiles.
        tiles: NonZeroU32,
        /// Width of each tile in pixels.
        width: NonZeroU32,
    },
    /// Interleaved signed 16-bit little-endian PCM samples.
    PcmS16Le {
        /// Channels, limited to eight.
        channels: NonZeroU8,
        /// Number of complete sample frames.
        frames: NonZeroU64,
        /// Samples per second per channel.
        sample_rate: NonZeroU32,
    },
    /// Top-to-bottom row-major sRGB RGBA pixels with straight alpha.
    Rgba8 {
        /// Height in pixels.
        height: NonZeroU32,
        /// Width in pixels.
        width: NonZeroU32,
    },
}

impl AssetFormat {
    fn expected_size(self) -> Result<Option<u64>, AssetError> {
        let bytes = match self {
            Self::Data => return Ok(None),
            Self::IndexedTiles8 { height, tiles, width } => {
                NonZeroU64::from(height)
                    .checked_mul(NonZeroU64::from(width))
                    .and_then(|pixels| {
                        pixels
                            .checked_mul(NonZeroU64::from(tiles))
                            .map(NonZeroU64::get)
                    })
            },
            Self::PcmS16Le {
                channels,
                frames,
                sample_rate: _rate,
            } => {
                if channels.get() > 8 {
                    return Err(AssetError::InvalidFormat);
                }
                frames.get().checked_mul(u64::from(channels.get()) * 2)
            },
            Self::Rgba8 { height, width } => NonZeroU64::from(height)
                .checked_mul(NonZeroU64::from(width))
                .and_then(|pixels| pixels.get().checked_mul(4)),
        };
        bytes.map(Some).ok_or(AssetError::InvalidFormat)
    }
}

/// One immutable logical asset and its content-addressed payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRecord {
    /// Exact SHA-256 identity of the canonical payload.
    pub digest: ContentHash,
    /// Canonical encoding of the payload.
    pub format: AssetFormat,
    /// Lowercase logical key, not a source filename or filesystem path.
    pub key: String,
    /// Exact number of bytes in the canonical payload.
    pub size: u64,
}

impl AssetRecord {
    /// Validate the logical key, format and declared payload size.
    ///
    /// This does not verify payload bytes or their digest.
    ///
    /// # Errors
    /// Rejects invalid identities or incompatible representation sizes.
    pub fn validate(&self) -> Result<(), AssetError> {
        Self::validate_descriptor(&self.key, self.format, self.size)
    }

    /// Check a borrowed descriptor before allocating a record or hashing.
    ///
    /// # Errors
    /// Rejects invalid identities or incompatible representation sizes.
    pub fn validate_descriptor(
        key: &str,
        format: AssetFormat,
        size: u64,
    ) -> Result<(), AssetError> {
        if !valid_key(key) {
            return Err(AssetError::InvalidIdentity);
        }
        if format
            .expected_size()?
            .is_some_and(|expected| expected != size)
        {
            return Err(AssetError::InvalidFormat);
        }
        Ok(())
    }

    /// Check payload length, representation and SHA-256 before consumption.
    ///
    /// # Errors
    /// Rejects malformed descriptors or mismatched payloads.
    pub fn verify(&self, payload: &[u8]) -> Result<(), AssetError> {
        self.validate()?;
        if u64::try_from(payload.len()).ok() != Some(self.size)
            || ContentHash::calculate(payload) != self.digest
        {
            return Err(AssetError::PayloadMismatch);
        }
        Ok(())
    }
}

/// Source selection, recorded as provenance rather than runtime behaviour.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    /// Selected source edition and supported layout identifier.
    pub revision: String,
    /// Canonical catalogue title identifier.
    pub title: String,
}

impl SourceIdentity {
    /// Check canonical title and edition identifiers.
    ///
    /// # Errors
    /// Rejects noncanonical identities.
    pub fn validate(&self) -> Result<(), AssetError> {
        if !valid_slug(&self.title) || !valid_slug(&self.revision) {
            return Err(AssetError::InvalidIdentity);
        }
        Ok(())
    }
}

/// Caller-selected admission limits for asset metadata and payloads.
#[derive(Clone, Copy, Debug)]
pub struct ManifestLimits {
    /// Maximum encoded manifest length.
    pub manifest_bytes: NonZeroUsize,
    /// Maximum size of any individual asset.
    pub payload_bytes: NonZeroU64,
    /// Maximum number of logical records.
    pub records: NonZeroUsize,
    /// Maximum sum of declared asset sizes.
    pub total_bytes: NonZeroU64,
}

/// Asset contract failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetError {
    /// An encoding is malformed, ambiguous or has unsupported fields.
    InvalidEncoding,
    /// The dimensions, channels or declared payload size are incompatible.
    InvalidFormat,
    /// An identity is not a canonical lowercase logical key.
    InvalidIdentity,
    /// Duplicate or out-of-order logical asset identities.
    InvalidOrder,
    /// Actual canonical payload differs from the manifest.
    PayloadMismatch,
    /// A metadata, count or payload budget was exceeded.
    TooLarge,
    /// The manifest version is unsupported.
    UnsupportedVersion,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ManifestDocument {
    records: Vec<AssetRecord>,
    source: SourceIdentity,
    version: u16,
}

/// Validated versioned asset metadata and content-addressed payloads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetManifest(ManifestDocument);

/// Fallibly grow an asset buffer under an explicit element-count limit.
///
/// The requested capacity grows geometrically but never above `maximum`.
/// An existing allocation is reused, not shrunk; allocator overhead is
/// outside this contract. This does not append elements to the buffer.
///
/// Returns `None` when the requested length overflows, exceeds the bound,
/// or the allocation fails.
pub fn reserve_bounded<T>(
    items: &mut Vec<T>,
    additional: usize,
    maximum: usize,
) -> Option<()> {
    let required = items.len().checked_add(additional)?;
    if required > maximum {
        return None;
    }
    if required > items.capacity() {
        let target = items
            .capacity()
            .saturating_mul(2)
            .max(required)
            .min(maximum);
        items
            .try_reserve_exact(target.checked_sub(items.len())?)
            .ok()?;
    }
    Some(())
}

fn valid_key(value: &str) -> bool {
    value.len() <= MAX_KEY_BYTES && value.split('/').all(valid_slug)
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_KEY_BYTES
        && value.split('-').all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit()
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> ManifestLimits {
        ManifestLimits {
            manifest_bytes: NonZeroUsize::new(4096)
                .unwrap_or(NonZeroUsize::MIN),
            payload_bytes: NonZeroU64::new(16).unwrap_or(NonZeroU64::MIN),
            records: NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
            total_bytes: NonZeroU64::new(24).unwrap_or(NonZeroU64::MIN),
        }
    }

    fn source() -> SourceIdentity {
        SourceIdentity {
            revision: String::from("world-set-1"),
            title: String::from("synthetic-title"),
        }
    }

    fn record(key: &str, payload: &[u8]) -> AssetRecord {
        AssetRecord {
            digest: ContentHash::calculate(payload),
            format: AssetFormat::Data,
            key: String::from(key),
            size: u64::try_from(payload.len()).unwrap_or(u64::MAX),
        }
    }

    fn require(condition: bool) -> Result<(), AssetError> {
        condition.then_some(()).ok_or(AssetError::InvalidEncoding)
    }

    #[test]
    fn bounded_growth_rejects_overflow_without_changing_contents()
    -> Result<(), AssetError> {
        let mut buffer = Vec::new();
        reserve_bounded(&mut buffer, 2, 3).ok_or(AssetError::TooLarge)?;
        buffer.extend_from_slice(b"ab");
        let capacity = buffer.capacity();
        require(reserve_bounded(&mut buffer, 2, 3).is_none())?;
        require(reserve_bounded(&mut buffer, usize::MAX, 3).is_none())?;
        require(buffer == b"ab" && buffer.capacity() == capacity)?;
        reserve_bounded(&mut buffer, 1, 3).ok_or(AssetError::TooLarge)?;
        buffer.push(b'c');
        require(buffer == b"abc")
    }

    #[test]
    fn manifests_are_deterministic_and_payloads_verified()
    -> Result<(), AssetError> {
        let left = record("stage/tiles", b"tiles");
        let right = record("stage/sprites", b"sprites");
        let first = AssetManifest::new(
            source(),
            vec![left.clone(), right.clone()],
            limits(),
        )?;
        let second = AssetManifest::new(source(), vec![right, left], limits())?;
        let bytes = first.encode(limits())?;
        require(bytes == second.encode(limits())?)?;
        let parsed = AssetManifest::parse(&bytes, limits())?;
        require(parsed == first)?;
        require(parsed.source() == &source())?;
        let entry = parsed.records().first().ok_or(AssetError::InvalidOrder)?;
        require(entry.key == "stage/sprites")?;
        entry.verify(b"sprites")?;
        require(entry.verify(b"wrongxx") == Err(AssetError::PayloadMismatch))?;
        require(entry.verify(b"sprite") == Err(AssetError::PayloadMismatch))
    }

    #[test]
    fn duplicate_keys_invalid_paths_and_budgets_are_rejected() {
        let item = record("stage/tile", b"tile");
        assert_eq!(
            AssetManifest::new(source(), vec![item.clone(), item], limits()),
            Err(AssetError::InvalidOrder),
        );
        for key in ["", "../escape", "Stage/tile", "stage//tile", "stage/tile."]
        {
            assert_eq!(
                AssetManifest::new(source(), vec![record(key, b"x")], limits()),
                Err(AssetError::InvalidIdentity),
            );
        }
        assert_eq!(
            AssetManifest::new(
                source(),
                vec![record("large", &[0; 17])],
                limits(),
            ),
            Err(AssetError::TooLarge),
        );
        assert_eq!(
            AssetManifest::new(
                source(),
                vec![record("first", &[0; 16]), record("second", &[0; 9])],
                limits(),
            ),
            Err(AssetError::TooLarge),
        );
    }

    fn image_record() -> AssetRecord {
        AssetRecord {
            format: AssetFormat::Rgba8 {
                height: NonZeroU32::MIN,
                width: NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
            },
            ..record("sprite/hero", b"abcdefgh")
        }
    }

    #[test]
    fn media_replacements_require_explicit_existing_slots()
    -> Result<(), AssetError> {
        let original = image_record();
        let data = record("stage/rules", b"abc");
        let manifest = AssetManifest::new(
            source(),
            vec![original.clone(), data.clone()],
            limits(),
        )?;
        let replacement = AssetRecord {
            digest: ContentHash::calculate(b"12345678"),
            ..original
        };
        let allowed = ["sprite/hero", "sprite/unknown", "stage/rules"];
        manifest.verify_media_replacement(
            &allowed,
            &replacement,
            b"12345678",
        )?;
        require(
            manifest.verify_media_replacement(&[], &replacement, b"12345678")
                == Err(AssetError::InvalidIdentity),
        )?;
        let unknown = AssetRecord {
            key: String::from("sprite/unknown"),
            ..replacement
        };
        require(
            manifest.verify_media_replacement(&allowed, &unknown, b"12345678")
                == Err(AssetError::InvalidIdentity),
        )?;
        require(
            manifest.verify_media_replacement(&allowed, &data, b"abc")
                == Err(AssetError::InvalidFormat),
        )
    }

    #[test]
    fn image_replacements_preserve_dimensions_and_payload_integrity()
    -> Result<(), AssetError> {
        let original = image_record();
        let manifest =
            AssetManifest::new(source(), vec![original.clone()], limits())?;
        let allowed = ["sprite/hero"];
        let replacement = AssetRecord {
            digest: ContentHash::calculate(b"12345678"),
            ..original
        };
        require(
            manifest.verify_media_replacement(
                &allowed,
                &replacement,
                b"87654321",
            ) == Err(AssetError::PayloadMismatch),
        )?;
        require(
            manifest.verify_media_replacement(&allowed, &replacement, b"short")
                == Err(AssetError::PayloadMismatch),
        )?;
        let transposed = AssetRecord {
            format: AssetFormat::Rgba8 {
                height: NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
                width: NonZeroU32::MIN,
            },
            ..replacement
        };
        require(
            manifest.verify_media_replacement(
                &allowed,
                &transposed,
                b"12345678",
            ) == Err(AssetError::InvalidFormat),
        )
    }

    #[test]
    fn pcm_replacements_preserve_sample_layout() -> Result<(), AssetError> {
        let original = AssetRecord {
            format: AssetFormat::PcmS16Le {
                channels: NonZeroU8::MIN,
                frames: NonZeroU64::new(2).unwrap_or(NonZeroU64::MIN),
                sample_rate: NonZeroU32::new(48_000).unwrap_or(NonZeroU32::MIN),
            },
            ..record("sound/hit", b"abcd")
        };
        let manifest =
            AssetManifest::new(source(), vec![original.clone()], limits())?;
        let replacement = AssetRecord {
            digest: ContentHash::calculate(b"efgh"),
            ..original
        };
        manifest.verify_media_replacement(
            &["sound/hit"],
            &replacement,
            b"efgh",
        )?;
        let different_rate = AssetRecord {
            format: AssetFormat::PcmS16Le {
                channels: NonZeroU8::MIN,
                frames: NonZeroU64::new(2).unwrap_or(NonZeroU64::MIN),
                sample_rate: NonZeroU32::new(44_100).unwrap_or(NonZeroU32::MIN),
            },
            ..replacement
        };
        require(
            manifest.verify_media_replacement(
                &["sound/hit"],
                &different_rate,
                b"efgh",
            ) == Err(AssetError::InvalidFormat),
        )
    }

    #[test]
    fn indexed_tiles_round_trip_and_reject_unapproved_replacement()
    -> Result<(), AssetError> {
        let mut item = record("graphics/tiles", b"1234");
        item.format = AssetFormat::IndexedTiles8 {
            height: NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
            tiles: NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
            width: NonZeroU32::MIN,
        };
        let manifest =
            AssetManifest::new(source(), vec![item.clone()], limits())?;
        let encoded = manifest.encode(limits())?;
        require(
            AssetManifest::parse(&encoded, limits()) == Ok(manifest.clone()),
        )?;
        require(manifest.record("graphics/tiles") == Some(&item))?;
        require(manifest.record("graphics/missing").is_none())?;
        require(item.verify(b"1234") == Ok(()))?;
        require(
            manifest.verify_media_replacement(
                &["graphics/tiles"],
                &item,
                b"1234",
            ) == Err(AssetError::InvalidFormat),
        )?;
        item.size = 3;
        require(item.validate() == Err(AssetError::InvalidFormat))?;
        item.format = AssetFormat::IndexedTiles8 {
            height: NonZeroU32::MAX,
            tiles: NonZeroU32::MAX,
            width: NonZeroU32::MAX,
        };
        require(item.validate() == Err(AssetError::InvalidFormat))
    }

    #[test]
    fn representations_require_exact_checked_sizes() {
        let mut item = record("image", &[0; 4]);
        item.format = AssetFormat::Rgba8 {
            height: NonZeroU32::MIN,
            width: NonZeroU32::MIN,
        };
        assert_eq!(item.verify(&[0; 4]), Ok(()));
        item.size = 3;
        assert_eq!(item.verify(&[0; 3]), Err(AssetError::InvalidFormat));
        item.format = AssetFormat::Rgba8 {
            height: NonZeroU32::MAX,
            width: NonZeroU32::MAX,
        };
        assert_eq!(item.verify(&[]), Err(AssetError::InvalidFormat));
        item.format = AssetFormat::PcmS16Le {
            channels: NonZeroU8::MIN,
            frames: NonZeroU64::MAX,
            sample_rate: NonZeroU32::MIN,
        };
        assert_eq!(item.verify(&[]), Err(AssetError::InvalidFormat));
    }

    #[test]
    fn parse_rejects_versions_and_unknown_or_duplicate_fields()
    -> Result<(), AssetError> {
        let manifest = AssetManifest::new(source(), Vec::new(), limits())?;
        let bytes = manifest.encode(limits())?;
        let document = String::from_utf8(bytes)
            .map_err(|_error| AssetError::InvalidEncoding)?;
        let unknown_version =
            document.replace("\"version\":2", "\"version\":3");
        require(
            AssetManifest::parse(unknown_version.as_bytes(), limits())
                == Err(AssetError::UnsupportedVersion),
        )?;
        for replacement in
            ["\"version\":2,\"extra\":0", "\"version\":2,\"version\":2"]
        {
            let malformed = document.replace("\"version\":2", replacement);
            require(
                AssetManifest::parse(malformed.as_bytes(), limits())
                    == Err(AssetError::InvalidEncoding),
            )?;
        }
        let mut restrictive = limits();
        restrictive.manifest_bytes = NonZeroUsize::MIN;
        require(
            AssetManifest::parse(document.as_bytes(), restrictive)
                == Err(AssetError::TooLarge),
        )
    }

    #[test]
    fn legacy_manifest_is_checked_and_normalized_without_source_fingerprint()
    -> Result<(), AssetError> {
        let manifest = AssetManifest::new(
            source(),
            vec![record("tile", b"tile")],
            limits(),
        )?;
        let encoded = manifest.encode(limits())?;
        let document = String::from_utf8(encoded.clone())
            .map_err(|_error| AssetError::InvalidEncoding)?;
        let digest = serde_json::to_string(&ContentHash::from_bytes([7; 32]))
            .map_err(|_error| AssetError::InvalidEncoding)?;
        let with_digest = document.replace(
            "\"source\":{",
            &format!("\"source\":{{\"digest\":{digest},"),
        );
        let legacy = with_digest.replace("\"version\":2", "\"version\":1");
        let parsed =
            AssetManifest::parse_canonical(legacy.as_bytes(), limits())?;
        require(parsed == manifest)?;
        require(parsed.encode(limits())? == encoded)?;
        require(
            ContentHash::calculate(legacy.as_bytes())
                != ContentHash::calculate(&encoded),
        )?;
        require(
            AssetManifest::parse(with_digest.as_bytes(), limits())
                == Err(AssetError::InvalidEncoding),
        )?;
        let noncanonical = format!("{legacy} ");
        require(
            AssetManifest::parse_canonical(noncanonical.as_bytes(), limits())
                == Err(AssetError::InvalidEncoding),
        )
    }

    #[test]
    fn record_budget_stops_before_decoding_an_excess_record()
    -> Result<(), AssetError> {
        let manifest = AssetManifest::new(
            source(),
            vec![record("first", b"x"), record("second", b"y")],
            limits(),
        )?;
        let bytes = manifest.encode(limits())?;
        let mut restrictive = limits();
        restrictive.records = NonZeroUsize::MIN;
        require(
            AssetManifest::parse(&bytes, restrictive)
                == Err(AssetError::TooLarge),
        )?;
        let document = String::from_utf8(bytes)
            .map_err(|_error| AssetError::InvalidEncoding)?;
        let malformed = document.replace("\"key\":\"second\"", "\"key\":false");
        require(
            AssetManifest::parse(malformed.as_bytes(), restrictive)
                == Err(AssetError::TooLarge),
        )?;
        require(
            AssetManifest::parse(malformed.as_bytes(), limits())
                == Err(AssetError::InvalidEncoding),
        )?;
        restrictive.records = NonZeroUsize::MIN.saturating_add(1);
        require(
            AssetManifest::parse(document.as_bytes(), restrictive)
                == Ok(manifest),
        )
    }

    #[test]
    fn record_admission_rejects_before_decoding_later_descriptors()
    -> Result<(), AssetError> {
        let manifest = AssetManifest::new(
            source(),
            vec![
                record("first", b"1234"),
                record("second", b"5678"),
                record("third", b"9012"),
            ],
            limits(),
        )?;
        let bytes = manifest.encode(limits())?;
        let document = String::from_utf8(bytes)
            .map_err(|_error| AssetError::InvalidEncoding)?;
        let malformed = document.replace("\"key\":\"third\"", "\"key\":false");
        let mut restrictive = limits();
        restrictive.payload_bytes =
            NonZeroU64::new(3).ok_or(AssetError::TooLarge)?;
        require(
            AssetManifest::parse(malformed.as_bytes(), restrictive)
                == Err(AssetError::TooLarge),
        )?;
        restrictive.payload_bytes =
            NonZeroU64::new(4).ok_or(AssetError::TooLarge)?;
        restrictive.total_bytes =
            NonZeroU64::new(7).ok_or(AssetError::TooLarge)?;
        require(
            AssetManifest::parse_canonical(malformed.as_bytes(), restrictive)
                == Err(AssetError::TooLarge),
        )?;
        let invalid_key =
            malformed.replace("\"key\":\"first\"", "\"key\":\"../x\"");
        require(
            AssetManifest::parse(invalid_key.as_bytes(), limits())
                == Err(AssetError::InvalidIdentity),
        )?;
        let duplicate =
            malformed.replace("\"key\":\"second\"", "\"key\":\"first\"");
        require(
            AssetManifest::parse_canonical(duplicate.as_bytes(), limits())
                == Err(AssetError::InvalidOrder),
        )?;
        restrictive.total_bytes =
            NonZeroU64::new(12).ok_or(AssetError::TooLarge)?;
        require(
            AssetManifest::parse_canonical(document.as_bytes(), restrictive)
                == Ok(manifest),
        )
    }

    #[test]
    fn encoded_byte_budget_accepts_exact_length_without_changing_wire_bytes()
    -> Result<(), AssetError> {
        let manifest = AssetManifest::new(
            source(),
            vec![record("asset", b"bytes")],
            limits(),
        )?;
        let expected = serde_json::to_vec(&manifest.0)
            .map_err(|_error| AssetError::InvalidEncoding)?;
        let mut restrictive = limits();
        restrictive.manifest_bytes = NonZeroUsize::new(expected.len())
            .ok_or(AssetError::InvalidEncoding)?;
        require(manifest.encode(restrictive)? == expected)?;
        require(AssetManifest::parse(&expected, restrictive)? == manifest)?;
        restrictive.manifest_bytes = NonZeroUsize::new(expected.len() - 1)
            .ok_or(AssetError::InvalidEncoding)?;
        require(manifest.encode(restrictive) == Err(AssetError::TooLarge))?;
        require(
            AssetManifest::new(
                source(),
                vec![record("asset", b"bytes")],
                restrictive,
            ) == Err(AssetError::TooLarge),
        )
    }

    #[test]
    fn content_hash_parser_rejects_noncanonical_values() {
        let digest = ContentHash::calculate(b"source");
        let encoded = digest.to_string();
        assert_eq!(encoded.parse::<ContentHash>(), Ok(digest));
        for invalid in ["", "abc", encoded.to_uppercase().as_str()] {
            assert_eq!(
                invalid.parse::<ContentHash>(),
                Err(AssetError::InvalidIdentity)
            );
        }
        assert_eq!(
            "z".repeat(64).parse::<ContentHash>(),
            Err(AssetError::InvalidIdentity)
        );
    }

    #[test]
    fn content_hash_uses_sha256_and_canonical_hex() {
        assert_eq!(
            ContentHash::calculate(b"abc").to_string(),
            concat!(
                "ba7816bf8f01cfea414140de5dae2223",
                "b00361a396177a9cb410ff61f20015ad"
            ),
        );
    }
}
