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
//   - Registered cemetery backdrop atlas admission and layer ordering.
// - Must-Not:
//   - Advance simulation or infer actor poses.
// - Allows:
//   - Inputs: Four canonical map bundles, phase and delayed camera offset.
//   - Outputs: Cropped, wrapped original-width sprite commands.
//   - Side effects: Bounded atlas uploads during scene initialization.
// - Split-When:
//   - Additional rooms need independent map contracts.
// - Merge-When:
//   - A shared map catalogue supports the same registration contract.
// - Summary:
//   - Compose the verified cemetery maps without runtime CPU composition.
// - Description:
//   - Upload constant maps once and select the animated map by phase.
// - Usage:
//   - Supply four distinct phase bundles in zero-based phase order.
// - Defaults:
//   - No registered layer selection.

use std::num::NonZeroU32;

use games::presentation::{Dimensions, Viewport};
use games_assets::{AssetFormat, ContentHash};
use games_platform::ir::frame::RenderError;
use games_platform::ir::graphics::scene_renderer::SceneSession;
use games_platform::ir::scene::SpriteDraw;
use games_platform::ir::storage::bundle::{BundleSource, CanonicalBundle};

use super::super::{Art, failure, open_bundles};
use super::shadow;

pub(in crate::runtime) const ATLAS_BYTES: usize = 6 * 512 * 256 * 4 + 4;
pub(in crate::runtime) const ATLAS_COUNT: usize = PHASES + 3;
// The admitted offset range and HUD crop fit each map without a seam.
pub(in crate::runtime) const EXTRA_DRAWS: usize = 3;
pub(in crate::runtime) const PHASES: usize = 4;

const ANIMATED_KEY: &str = "scene/layer-1";
const CONSTANT_KEYS: [&str; 3] =
    ["scene/backdrop", "scene/layer-2", "scene/layer-0"];

#[derive(Debug)]
pub(super) struct Layers {
    animated: [Art; PHASES],
    backdrop: Art,
    front: Art,
    middle: Art,
}

impl Layers {
    pub(super) fn load(
        store: BundleSource<'_>,
        scene: &mut SceneSession<'_, '_>,
        hashes: [ContentHash; PHASES],
        mut sources: Option<&mut Vec<shadow::Source>>,
    ) -> Result<Self, RenderError> {
        let bundles = open_bundles(store, hashes)?;
        let [first, ..] = &bundles;
        let [backdrop, middle, front] = CONSTANT_KEYS;
        for key in CONSTANT_KEYS {
            validate_constant(&bundles, key)?;
        }
        let animated_digests = bundles
            .iter()
            .map(|bundle| digest(bundle, ANIMATED_KEY))
            .collect::<Result<Vec<_>, _>>()?;
        if !distinct(&animated_digests) {
            return Err(failure("duplicate cemetery animated map"));
        }
        for bundle in &bundles {
            validate(bundle, ANIMATED_KEY)?;
        }
        let animated = bundles
            .iter()
            .map(|bundle| {
                load_map(scene, bundle, ANIMATED_KEY, sources.as_deref_mut())
            })
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_error| RenderError::InvalidSprite)?;
        Ok(Self {
            animated,
            backdrop: load_map(scene, first, backdrop, sources.as_deref_mut())?,
            front: load_map(scene, first, front, sources.as_deref_mut())?,
            middle: load_map(scene, first, middle, sources)?,
        })
    }

    pub(super) fn queue(
        &self,
        phase: u8,
        offset: i16,
        draws: &mut Vec<SpriteDraw>,
    ) -> Result<(), RenderError> {
        let animated = self
            .animated
            .get(usize::from(phase))
            .ok_or(RenderError::InvalidSprite)?;
        let origin = u32::try_from(offset).map_err(failure)?;
        if origin > 31 {
            return Err(failure("cemetery camera offset out of range"));
        }
        let mut rect = self.backdrop.rect;
        rect.width = NonZeroU32::new(320).ok_or(RenderError::InvalidSprite)?;
        rect.height = NonZeroU32::new(196).ok_or(RenderError::InvalidSprite)?;
        rect.y = 28i32;
        draws.push(self.backdrop.draw(rect, false));
        for (art, x, y) in [
            (*animated, 0, origin),
            (self.middle, 0, origin),
            (self.front, 96, 16),
        ] {
            let view = Viewport {
                x,
                y: y.saturating_add(28),
                width: 320,
                height: 196,
            };
            let period = Dimensions::new(art.rect.width, art.rect.height);
            for mut draw in SpriteDraw::wrapped(art.atlas, period, view)? {
                draw.destination.y = draw.destination.y.saturating_add(28);
                draws.push(draw);
            }
        }
        Ok(())
    }
}

pub(in crate::runtime) fn parse(
    line: &str,
) -> Result<[ContentHash; PHASES], RenderError> {
    let mut fields = line.split_whitespace();
    if fields.next() != Some("cemetery-room-layers") {
        return Err(failure("unexpected cemetery layer record"));
    }
    let hashes: [ContentHash; PHASES] = fields
        .take(PHASES + 1)
        .map(|field| field.parse::<ContentHash>().map_err(failure))
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_error| failure("expected four cemetery layer phases"))?;
    if !distinct(&hashes) {
        return Err(failure("duplicate cemetery layer phase"));
    }
    Ok(hashes)
}

fn digest(
    bundle: &CanonicalBundle,
    key: &str,
) -> Result<ContentHash, RenderError> {
    bundle
        .manifest()
        .record(key)
        .map(|record| record.digest)
        .ok_or(RenderError::InvalidSprite)
}

fn distinct(items: &[ContentHash]) -> bool {
    items.iter().enumerate().all(|(index, item)| {
        items
            .iter()
            .skip(index.saturating_add(1))
            .all(|other| item != other)
    })
}

fn load_map(
    scene: &mut SceneSession<'_, '_>,
    bundle: &CanonicalBundle,
    key: &str,
    sources: Option<&mut Vec<shadow::Source>>,
) -> Result<Art, RenderError> {
    let (art, pixels) = Art::load_with_pixels(scene, bundle, key)?;
    if let Some(retained) = sources {
        retained.push(shadow::Source::new(art, &pixels)?);
    }
    Ok(art)
}

fn validate(bundle: &CanonicalBundle, key: &str) -> Result<(), RenderError> {
    let record = bundle
        .manifest()
        .record(key)
        .ok_or(RenderError::InvalidSprite)?;
    let AssetFormat::Rgba8 { width, height } = record.format else {
        return Err(RenderError::InvalidSprite);
    };
    let backdrop = key == "scene/backdrop";
    let extent = if backdrop {
        (1, 1)
    } else {
        (512, 256)
    };
    if (width.get(), height.get()) != extent {
        return Err(failure("invalid cemetery layer extent"));
    }
    let pixels = bundle.view_payload(key).map_err(failure)?;
    if !valid_alpha(&pixels, backdrop) {
        return Err(failure("invalid cemetery layer alpha"));
    }
    Ok(())
}

fn validate_constant(
    bundles: &[CanonicalBundle; PHASES],
    key: &str,
) -> Result<(), RenderError> {
    let [first, ..] = bundles;
    let expected = digest(first, key)?;
    for bundle in bundles {
        if digest(bundle, key)? != expected {
            return Err(failure("cemetery constant layer differs"));
        }
        validate(bundle, key)?;
    }
    Ok(())
}

fn valid_alpha(bytes: &[u8], backdrop: bool) -> bool {
    let (pixels, remainder) = bytes.as_chunks::<4>();
    !pixels.is_empty()
        && remainder.is_empty()
        && pixels
            .iter()
            .all(|[_, _, _, alpha]| *alpha == 255 || (!backdrop && *alpha == 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_alpha_is_binary_and_backdrop_is_opaque() {
        assert!(valid_alpha(&[0, 0, 0, 255], true));
        assert!(!valid_alpha(&[0, 0, 0, 0], true));
        assert!(valid_alpha(&[0, 0, 0, 0, 1, 2, 3, 255], false));
        assert!(!valid_alpha(&[0, 0, 0, 128], false));
        assert!(!valid_alpha(&[0, 0, 0], false));
    }

    #[test]
    fn phase_record_requires_four_distinct_hashes() {
        let hashes: Vec<_> = (0u8..4).map(|n| format!("{n:064x}")).collect();
        let line = format!("cemetery-room-layers {}", hashes.join(" "));
        assert!(parse(&line).is_ok());
        assert!(parse(&format!("{line} {}", hashes.join(" "))).is_err());
        assert!(parse("cemetery-room-layers").is_err());
        let repeated = vec!["0".repeat(64); 4].join(" ");
        let duplicate = format!("cemetery-room-layers {repeated}");
        assert!(parse(&duplicate).is_err());
    }
}
