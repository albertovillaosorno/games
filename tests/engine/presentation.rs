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
//   - Responsive layout, localization and frame-pacing policy tests.
// - Must-Not:
//   - Pretend the actual GPU pipeline or host event loop was exercised.
// - Allows:
//   - Inputs: Synthetic screen sizes, messages and capped FPS.
//   - Outputs: Public layout rectangles, locale text and frame intervals.
//   - Side effects: None beyond in-memory translation catalogues.
// - Split-When:
//   - Native rendering tests acquire separate platform ownership.
// - Merge-When:
//   - Platform-neutral presentation fixtures serve multiple consumers.
// - Summary:
//   - Viewport, localization and frame pacing regression coverage.
// - Description:
//   - Tests presentation policies independently of simulation timing.
// - Usage:
//   - cargo validate
// - Defaults:
//   - No renderer, device drivers or original game media required.
//

use std::num::NonZeroU32;
use std::time::Duration;

use super::{FramePacing, TranslationCatalog, Viewport};

fn nz(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
}

#[test]
fn responsive_layout_centers_letterboxed_content() {
    assert_eq!(Viewport::fit(nz(800), nz(600), nz(16), nz(9)), Viewport {
        height: 450,
        width: 800,
        x: 0,
        y: 75
    },);
    assert_eq!(
        Viewport::fit(nz(480), nz(960), nz(800), nz(600)),
        Viewport {
            height: 360,
            width: 480,
            x: 0,
            y: 300
        },
    );
    assert_eq!(Viewport::fit(nz(1280), nz(720), nz(16), nz(9)), Viewport {
        height: 720,
        width: 1280,
        x: 0,
        y: 0
    },);
}

#[test]
fn fitted_viewports_remain_inside_extreme_surface_sizes() {
    for width in [1, 2, 3, 64, 320, 1080, u32::MAX] {
        for height in [1, 2, 3, 64, 1080, u32::MAX] {
            let rect = Viewport::fit(nz(width), nz(height), nz(1920), nz(1080));
            assert!(rect.width > 0 && rect.height > 0);
            assert!(rect.x.saturating_add(rect.width) <= width);
            assert!(rect.y.saturating_add(rect.height) <= height);
        }
    }
}

#[test]
fn locale_and_regional_fallback_preserve_declared_messages() {
    let mut catalogue = TranslationCatalog::new("en");
    catalogue.insert("en", "start", "Start");
    catalogue.insert("en", "quit", "Quit");
    catalogue.insert("es", "start", "Comenzar");
    catalogue.insert("es-MX", "start", "Jugar");
    assert_eq!(catalogue.resolve("es-MX", "start"), Some("Jugar"));
    assert_eq!(catalogue.resolve("es-ES", "start"), Some("Comenzar"));
    assert_eq!(catalogue.resolve("es-MX", "quit"), Some("Quit"));
    assert_eq!(catalogue.resolve("fr", "start"), Some("Start"));
    assert_eq!(catalogue.resolve("fr", "missing"), None);
}

#[test]
fn frame_caps_do_not_require_a_fixed_render_rate() {
    assert_eq!(FramePacing::Uncapped.interval(), None);
    assert_eq!(
        FramePacing::Capped(nz(30)).interval(),
        Some(Duration::from_nanos(33_333_333)),
    );
    assert_eq!(
        FramePacing::Capped(nz(120)).interval(),
        Some(Duration::from_nanos(8_333_333)),
    );
    assert_eq!(
        FramePacing::Capped(nz(u32::MAX)).interval(),
        Some(Duration::from_nanos(1)),
    );
}
