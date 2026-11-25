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
//   - Viewport geometry, language fallback and display pacing policy.
// - Must-Not:
//   - Own a GPU backend, OS event loop or title-specific translations.
// - Allows:
//   - Inputs: Surface sizes, logical sizes, locale strings and frame caps.
//   - Outputs: Fitted viewport, resolved strings and frame intervals.
//   - Side effects: Stores caller-supplied translation catalogues.
// - Split-When:
//   - Actual renderer and platform adapters gain distinct ownership.
// - Merge-When:
//   - Shared presentation policies supersede this module.
// - Summary:
//   - Platform-neutral presentation configuration contracts.
// - Description:
//   - Keeps responsive layouts and frame caps independent of simulation.
// - Usage:
//   - Configure in a platform adapter; feed layout into its renderer.
// - Defaults:
//   - Uncapped presentation, preserve aspect ratio, fallback translation.
//
//! Deterministic presentation configuration without a rendering backend.

use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroU64};
use std::str;
use std::time::Duration;

/// An integer pixel rectangle fitted within a display surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Viewport {
    /// Fitted viewport height in surface pixels.
    pub height: u32,
    /// Fitted viewport width in surface pixels.
    pub width: u32,
    /// Left inset in surface pixels.
    pub x: u32,
    /// Top inset in surface pixels.
    pub y: u32,
}

/// How a logical frame occupies the available physical surface.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PresentationMode {
    /// Extend the logical camera horizontally when the title supports it.
    Adaptive,
    /// Preserve the aspect ratio and letterbox any remaining area.
    #[default]
    AspectFit,
    /// Fill the surface using a centered crop of the logical frame.
    Fill,
    /// Preserve the aspect ratio while using whole-number pixel scaling.
    Integer,
    /// Fill the surface without preserving the aspect ratio.
    Stretch,
}

/// Nonzero surface or logical frame dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dimensions {
    /// Vertical pixel extent.
    pub height: NonZeroU32,
    /// Horizontal pixel extent.
    pub width: NonZeroU32,
}

impl Dimensions {
    /// Create dimensions in width, height order.
    #[must_use]
    pub const fn new(width: NonZeroU32, height: NonZeroU32) -> Self {
        Self { height, width }
    }
}

/// The exact source and destination rectangles used for presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationLayout {
    /// Complete logical frame dimensions.
    pub design: Dimensions,
    /// Destination rectangle within the physical surface.
    pub destination: Viewport,
    /// Source rectangle within the complete logical frame.
    pub source: Viewport,
    /// Complete physical surface dimensions.
    pub surface: Dimensions,
}

impl PresentationMode {
    /// All supported presentation choices in menu cycling order.
    pub const ALL: [Self; 5] = [
        Self::AspectFit,
        Self::Integer,
        Self::Fill,
        Self::Stretch,
        Self::Adaptive,
    ];

    /// Portable settings value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::AspectFit => "aspect-fit",
            Self::Fill => "fill",
            Self::Integer => "integer",
            Self::Stretch => "stretch",
        }
    }

    /// Determine the admissible logical extent before rendering a frame.
    ///
    /// Adaptive retains the base height and may widen only to the supplied
    /// supported camera limit. Pass the base width for fixed scenes.
    #[must_use]
    pub fn design_for_surface(
        self,
        surface: Dimensions,
        base: Dimensions,
        maximum_width: NonZeroU32,
    ) -> Dimensions {
        if self != Self::Adaptive || maximum_width <= base.width {
            return base;
        }
        let desired = NonZeroU64::from(surface.width)
            .get()
            .saturating_mul(NonZeroU64::from(base.height).get())
            .checked_div(NonZeroU64::from(surface.height).get())
            .unwrap_or(0);
        let admitted = desired
            .min(NonZeroU64::from(maximum_width).get())
            .max(NonZeroU64::from(base.width).get());
        let width = u32::try_from(admitted)
            .ok()
            .and_then(NonZeroU32::new)
            .unwrap_or(base.width);
        Dimensions::new(width, base.height)
    }

    /// Select source and destination pixel rectangles.
    ///
    /// Adaptive fits the actual supplied logical frame; call
    /// `design_for_surface` before composing an extendable scene.
    /// Integer mode falls back to aspect-fit below one logical pixel scale.
    /// Fill rounds its source crop to whole logical pixels.
    #[must_use]
    pub fn layout(
        self,
        surface: Dimensions,
        design: Dimensions,
    ) -> PresentationLayout {
        let Dimensions {
            height: surface_height,
            width: surface_width,
        } = surface;
        let Dimensions {
            height: design_height,
            width: design_width,
        } = design;
        let full_source = Viewport::full(design_width, design_height);
        let full_surface = Viewport::full(surface_width, surface_height);
        let fit = || {
            Viewport::fit(
                surface_width,
                surface_height,
                design_width,
                design_height,
            )
        };
        let (source, destination) = match self {
            Self::Adaptive | Self::AspectFit => (full_source, fit()),
            Self::Integer => {
                let horizontal = surface_width
                    .get()
                    .checked_div(design_width.get())
                    .unwrap_or(0);
                let vertical = surface_height
                    .get()
                    .checked_div(design_height.get())
                    .unwrap_or(0);
                let scale = horizontal.min(vertical);
                let destination = if scale == 0 {
                    fit()
                } else {
                    full_surface.center(
                        design_width.get().saturating_mul(scale),
                        design_height.get().saturating_mul(scale),
                    )
                };
                (full_source, destination)
            },
            Self::Fill => (
                Viewport::fit(
                    design_width,
                    design_height,
                    surface_width,
                    surface_height,
                ),
                full_surface,
            ),
            Self::Stretch => (full_source, full_surface),
        };
        PresentationLayout {
            design,
            destination,
            source,
            surface,
        }
    }

    /// Cycle presentation, optionally skipping modes that crop controls.
    #[must_use]
    pub const fn next(self, restrict_fill: bool) -> Self {
        match self {
            Self::Adaptive => Self::AspectFit,
            Self::AspectFit => Self::Integer,
            Self::Integer if !restrict_fill => Self::Fill,
            Self::Integer | Self::Fill => Self::Stretch,
            Self::Stretch => Self::Adaptive,
        }
    }

    /// Parse a portable settings value.
    ///
    /// # Errors
    /// Rejects unknown or ambiguous presentation modes.
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "adaptive" => Ok(Self::Adaptive),
            "aspect-fit" => Ok(Self::AspectFit),
            "fill" => Ok(Self::Fill),
            "integer" => Ok(Self::Integer),
            "stretch" => Ok(Self::Stretch),
            _ => Err("expected adaptive, aspect-fit, fill, integer or stretch"),
        }
    }
}

/// Whether the host uses a normal or desktop-fullscreen window.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WindowMode {
    /// Occupy the current desktop without changing the display resolution.
    Fullscreen,
    /// Keep the window resizable on the desktop.
    #[default]
    Windowed,
}

impl WindowMode {
    /// Both portable window choices.
    pub const ALL: [Self; 2] = [Self::Windowed, Self::Fullscreen];

    /// Portable settings value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fullscreen => "fullscreen",
            Self::Windowed => "windowed",
        }
    }

    /// Parse a portable settings value.
    ///
    /// # Errors
    /// Rejects unknown or ambiguous window modes.
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "fullscreen" => Ok(Self::Fullscreen),
            "windowed" => Ok(Self::Windowed),
            _ => Err("expected fullscreen or windowed"),
        }
    }

    /// Toggle desktop fullscreen without changing resolution.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Fullscreen => Self::Windowed,
            Self::Windowed => Self::Fullscreen,
        }
    }
}

/// Persistent presentation choices independent of a game's logical layout.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DisplaySettings {
    /// Select how the logical frame occupies the window.
    pub presentation: PresentationMode,
    /// Select desktop-fullscreen or resizable windowed presentation.
    pub window: WindowMode,
}

impl DisplaySettings {
    /// Resolve explicitly supplied values over an existing configuration.
    ///
    /// # Errors
    /// Rejects unknown presentation or window mode names.
    pub fn configured(
        self,
        presentation: Option<&str>,
        window: Option<&str>,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            presentation: presentation
                .map(PresentationMode::parse)
                .transpose()?
                .unwrap_or(self.presentation),
            window: window
                .map(WindowMode::parse)
                .transpose()?
                .unwrap_or(self.window),
        })
    }

    /// Decode a complete version-one portable preferences payload.
    ///
    /// # Errors
    /// Rejects invalid UTF-8, versions, modes and trailing fields.
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let text = str::from_utf8(bytes)
            .map_err(|_error| "invalid display preferences")?;
        let mut fields = text.split('\n');
        match (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) {
            (
                Some("display-v1"),
                Some(presentation),
                Some(window),
                Some(""),
                None,
            ) => Self::default().configured(Some(presentation), Some(window)),
            _ => Err("invalid display preferences"),
        }
    }

    /// Encode preferences using stable mode names rather than enum ordinals.
    #[must_use]
    pub fn encode(self) -> String {
        format!(
            "display-v1\n{}\n{}\n",
            self.presentation.as_str(),
            self.window.as_str()
        )
    }
}

/// A policy for whether the host should cap rendering frequency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramePacing {
    /// The renderer should target at most the requested frequency.
    Capped(NonZeroU32),
    /// The renderer may present whenever a frame is ready.
    Uncapped,
}

impl FramePacing {
    /// Return the approximate minimum duration between rendered frames.
    ///
    /// This does not alter the simulation clock or sleep a host thread.
    #[must_use]
    pub fn interval(self) -> Option<Duration> {
        match self {
            Self::Uncapped => None,
            Self::Capped(frames_per_second) => {
                let nanos = 1_000_000_000u64
                    .checked_div(NonZeroU64::from(frames_per_second).get())
                    .unwrap_or(1)
                    .max(1);
                Some(Duration::from_nanos(nanos))
            },
        }
    }
}

impl Viewport {
    fn center(self, width: u32, height: u32) -> Self {
        Self {
            height,
            width,
            x: self.width.saturating_sub(width).checked_div(2).unwrap_or(0),
            y: self
                .height
                .saturating_sub(height)
                .checked_div(2)
                .unwrap_or(0),
        }
    }

    /// Fit a logical design size inside the surface without stretching it.
    ///
    /// A renderer may choose how to fill the remaining letterbox area.
    #[must_use]
    pub fn fit(
        surface_width: NonZeroU32,
        surface_height: NonZeroU32,
        design_width: NonZeroU32,
        design_height: NonZeroU32,
    ) -> Self {
        let sw = NonZeroU64::from(surface_width).get();
        let sh = NonZeroU64::from(surface_height).get();
        let dw = NonZeroU64::from(design_width).get();
        let dh = NonZeroU64::from(design_height).get();
        let width_limited = sw.saturating_mul(dh) <= sh.saturating_mul(dw);
        let (fitted_width, fitted_height) = if width_limited {
            (sw, sw.saturating_mul(dh).checked_div(dw).unwrap_or(1))
        } else {
            (sh.saturating_mul(dw).checked_div(dh).unwrap_or(1), sh)
        };
        let width = u32::try_from(fitted_width.max(1).min(sw))
            .unwrap_or_else(|_| surface_width.get());
        let height = u32::try_from(fitted_height.max(1).min(sh))
            .unwrap_or_else(|_| surface_height.get());
        Self::full(surface_width, surface_height).center(width, height)
    }

    const fn full(width: NonZeroU32, height: NonZeroU32) -> Self {
        Self {
            height: height.get(),
            width: width.get(),
            x: 0,
            y: 0,
        }
    }
}

/// A caller-supplied translation catalogue with configured locale fallback.
type LocaleMessages = BTreeMap<String, String>;

/// Translation catalogue with a configurable fallback locale.
#[derive(Clone, Debug)]
pub struct TranslationCatalog {
    fallback: String,
    locales: BTreeMap<String, LocaleMessages>,
}

impl TranslationCatalog {
    fn get(&self, locale: &str, key: &str) -> Option<&str> {
        self.locales.get(locale)?.get(key).map(String::as_str)
    }

    /// Add or replace a caller-provided message for one locale.
    pub fn insert(&mut self, locale: &str, key: &str, text: &str) {
        let _previous = self
            .locales
            .entry(String::from(locale))
            .or_default()
            .insert(String::from(key), String::from(text));
    }

    /// Create a catalogue with a configured fallback locale identifier.
    #[must_use]
    pub fn new(fallback: &str) -> Self {
        Self {
            fallback: String::from(fallback),
            locales: BTreeMap::new(),
        }
    }

    /// Resolve the full locale, primary language, then fallback locale.
    ///
    /// Returns `None` if no supplied catalogue contains the key.
    #[must_use]
    pub fn resolve(&self, locale: &str, key: &str) -> Option<&str> {
        self.get(locale, key)
            .or_else(|| {
                locale
                    .split_once('-')
                    .and_then(|(base, _)| self.get(base, key))
            })
            .or_else(|| self.get(&self.fallback, key))
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::time::Duration;

    use super::{
        Dimensions, DisplaySettings, FramePacing, PresentationMode,
        TranslationCatalog, Viewport, WindowMode,
    };

    fn nz(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
    }

    #[test]
    fn presentation_modes_select_distinct_source_and_destination_rectangles() {
        let surface = Dimensions::new(nz(800), nz(600));
        let design = Dimensions::new(nz(320), nz(224));
        let fit = PresentationMode::AspectFit.layout(surface, design);
        let integer = PresentationMode::Integer.layout(surface, design);
        let fill = PresentationMode::Fill.layout(surface, design);
        let stretch = PresentationMode::Stretch.layout(surface, design);
        assert_eq!(fit.destination, Viewport {
            height: 560,
            width: 800,
            x: 0,
            y: 20,
        });
        assert_eq!(integer.destination, Viewport {
            height: 448,
            width: 640,
            x: 80,
            y: 76,
        });
        assert_eq!(fill.destination, stretch.destination);
        assert_eq!(fill.source, Viewport {
            height: 224,
            width: 298,
            x: 11,
            y: 0,
        });
        assert_eq!(fit.source, stretch.source);
        assert_eq!(integer.source, stretch.source);
        assert_eq!(stretch.destination, Viewport {
            height: 600,
            width: 800,
            x: 0,
            y: 0,
        });
    }

    #[test]
    fn integer_mode_falls_back_to_fit_when_the_design_cannot_fit_at_one() {
        let surface = Dimensions::new(nz(100), nz(700));
        let design = Dimensions::new(nz(320), nz(224));
        assert_eq!(
            PresentationMode::Integer.layout(surface, design),
            PresentationMode::AspectFit.layout(surface, design),
        );
    }

    #[test]
    fn adaptive_extends_only_within_an_explicit_supported_width() {
        let base = Dimensions::new(nz(320), nz(224));
        let wide = Dimensions::new(nz(1920), nz(1080));
        let tall = Dimensions::new(nz(400), nz(800));
        let mode = PresentationMode::Adaptive;
        assert_eq!(mode.design_for_surface(wide, base, base.width), base);
        assert_eq!(mode.design_for_surface(tall, base, nz(800)), base);
        assert_eq!(
            mode.design_for_surface(wide, base, nz(800)),
            Dimensions::new(nz(398), base.height),
        );
        assert_eq!(
            mode.design_for_surface(wide, base, nz(360)),
            Dimensions::new(nz(360), base.height),
        );
        assert_eq!(
            mode.layout(wide, base),
            PresentationMode::AspectFit.layout(wide, base),
        );
        assert_eq!(
            PresentationMode::Fill.design_for_surface(wide, base, nz(800)),
            base,
        );
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
                let rect =
                    Viewport::fit(nz(width), nz(height), nz(1920), nz(1080));
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

    #[test]
    fn display_preferences_round_trip_every_mode() {
        for presentation in PresentationMode::ALL {
            for window in WindowMode::ALL {
                let settings = DisplaySettings { presentation, window };
                assert_eq!(
                    DisplaySettings::decode(settings.encode().as_bytes()),
                    Ok(settings)
                );
            }
        }
    }

    #[test]
    fn display_preferences_reject_incompatible_payloads() {
        for bytes in [
            b"display-v2\ninteger\nwindowed\n".as_slice(),
            b"display-v1\nunknown\nwindowed\n",
            b"display-v1\ninteger\nunknown\n",
            b"display-v1\ninteger\nwindowed",
            b"display-v1\ninteger\nwindowed\n\n",
            b"\xff",
        ] {
            assert!(DisplaySettings::decode(bytes).is_err());
        }
    }
}
