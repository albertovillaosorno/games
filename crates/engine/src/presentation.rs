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
        Self {
            height,
            width,
            x: surface_width
                .get()
                .saturating_sub(width)
                .checked_div(2)
                .unwrap_or(0),
            y: surface_height
                .get()
                .saturating_sub(height)
                .checked_div(2)
                .unwrap_or(0),
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
#[path = "../../../tests/engine/presentation.rs"]
mod tests;
