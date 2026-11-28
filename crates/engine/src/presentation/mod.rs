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

mod localisation;
mod message;

use std::num::{NonZeroU32, NonZeroU64};
use std::str;
use std::time::Duration;

pub use localisation::{
    CatalogError, CatalogLimits, LocaleTag, TranslationCatalog,
};
pub use message::{
    MessageArguments, MessageFormatError, MessageParameter, MessageValue,
};

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

/// A nonempty integer rectangle in atlas or logical scene pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpriteRect {
    /// Rectangle height.
    pub height: NonZeroU32,
    /// Rectangle width.
    pub width: NonZeroU32,
    /// Horizontal origin.
    pub x: i32,
    /// Vertical origin.
    pub y: i32,
}

impl SpriteRect {
    /// Position the rectangle around a logical center, preserving its size.
    ///
    /// Odd dimensions use the lower integer half. Coordinates saturate
    /// rather than overflow at the representable scene boundaries.
    #[must_use]
    pub fn centered_at(self, horizontal: i32, vertical: i32) -> Self {
        Self {
            x: horizontal.saturating_sub(
                i32::try_from(self.width.get() >> 1u32).unwrap_or(i32::MAX),
            ),
            y: vertical.saturating_sub(
                i32::try_from(self.height.get() >> 1u32).unwrap_or(i32::MAX),
            ),
            ..self
        }
    }

    /// Test a pixel using widened arithmetic, including negative origins.
    #[must_use]
    pub fn contains_pixel(self, horizontal: u32, vertical: u32) -> bool {
        let local_x = i64::from(horizontal).saturating_sub(i64::from(self.x));
        let local_y = i64::from(vertical).saturating_sub(i64::from(self.y));
        (0..i64::from(self.width.get())).contains(&local_x)
            && (0..i64::from(self.height.get())).contains(&local_y)
    }

    /// Select a left-aligned portion of this rectangle, without truncation.
    /// Zero columns or a portion exceeding the extent have no rectangle.
    #[must_use]
    pub const fn left_columns(self, columns: u32) -> Option<Self> {
        if columns > self.width.get() {
            return None;
        }
        match NonZeroU32::new(columns) {
            Some(width) => Some(Self { width, ..self }),
            None => None,
        }
    }
}

/// Caller-selected horizontal camera geometry for a padded art strip.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StripViewport {
    /// Width of the unexpanded design.
    pub design_width: u32,
    /// Maximum requested viewport width.
    pub maximum_width: u32,
    /// Art inset on either side of the strip.
    pub side_inset: u32,
}

impl StripViewport {
    /// Resolve a camera origin without crossing the available art bounds.
    #[must_use]
    pub fn origin(self, strip: u32, width: u32, camera: u32) -> Option<u32> {
        let maximum = self.width(strip)?;
        if !(self.design_width..=maximum).contains(&width) {
            return None;
        }
        let (available, inset) = if width > self.design_width {
            (strip, self.side_inset)
        } else {
            (strip.checked_add(self.side_inset.checked_mul(2)?)?, 0)
        };
        camera
            .min(available.saturating_sub(width))
            .checked_add(inset)
    }

    /// Fit the view to the supplied art and caller-selected width range.
    #[must_use]
    pub fn width(self, strip: u32) -> Option<u32> {
        if self.design_width == 0 || self.maximum_width < self.design_width {
            return None;
        }
        Some(strip.clamp(self.design_width, self.maximum_width))
    }
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

/// A source rectangle and its corresponding drawable destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationRegion {
    /// Physical pixels occupied by the selected region.
    pub destination: Viewport,
    /// Logical pixels selected from the complete frame.
    pub source: Viewport,
}

/// The exact source and destination rectangles used for presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationLayout {
    /// Complete logical frame dimensions.
    pub design: Dimensions,
    /// Destination rectangle within the physical surface.
    pub destination: Viewport,
    /// Optional uncropped region drawn after the primary rectangle.
    pub protected: Option<PresentationRegion>,
    /// Source rectangle within the complete logical frame.
    pub source: Viewport,
    /// Complete physical surface dimensions.
    pub surface: Dimensions,
}

impl PresentationLayout {
    /// Regions in drawing order, with the primary region first.
    #[must_use]
    pub const fn regions(self) -> [Option<PresentationRegion>; 2] {
        [
            Some(PresentationRegion {
                destination: self.destination,
                source: self.source,
            }),
            self.protected,
        ]
    }
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
            protected: None,
            source,
            surface,
        }
    }

    /// Keep a logical bottom strip visible while cropping the upper scene.
    ///
    /// Fill reserves a proportional drawable height for the strip and fits
    /// it without cropping. Other modes already preserve the whole frame.
    /// A one-pixel-tall surface cannot split, so it fits the whole frame.
    /// Returns None if the strip would consume the entire logical frame.
    #[must_use]
    pub fn layout_with_protected_bottom(
        self,
        surface: Dimensions,
        design: Dimensions,
        protected_bottom: Option<NonZeroU32>,
    ) -> Option<PresentationLayout> {
        let Some(bottom) = protected_bottom else {
            return Some(self.layout(surface, design));
        };
        let top =
            NonZeroU32::new(design.height.get().checked_sub(bottom.get())?)?;
        if self != Self::Fill {
            return Some(self.layout(surface, design));
        }
        if surface.height.get() == 1 {
            return Some(Self::AspectFit.layout(surface, design));
        }
        let rows = NonZeroU64::from(surface.height)
            .get()
            .checked_mul(NonZeroU64::from(bottom).get())?
            .checked_div(NonZeroU64::from(design.height).get())?;
        let reserved = u32::try_from(rows)
            .ok()?
            .clamp(1, surface.height.get().checked_sub(1)?);
        let upper =
            NonZeroU32::new(surface.height.get().checked_sub(reserved)?)?;
        let lower = NonZeroU32::new(reserved)?;
        let primary = self.layout(
            Dimensions::new(surface.width, upper),
            Dimensions::new(design.width, top),
        );
        let panel = Self::AspectFit.layout(
            Dimensions::new(surface.width, lower),
            Dimensions::new(design.width, bottom),
        );
        Some(PresentationLayout {
            design,
            destination: primary.destination,
            protected: Some(PresentationRegion {
                destination: Viewport {
                    y: upper.get().checked_add(panel.destination.y)?,
                    ..panel.destination
                },
                source: Viewport {
                    y: top.get(),
                    ..panel.source
                },
            }),
            source: primary.source,
            surface,
        })
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

    /// Split a wrapped sampling rectangle into one-to-one regions.
    ///
    /// The source origin is periodic in both axes. The destination starts
    /// at zero. A view within one period needs at most four regions, with
    /// no empty rectangles or repeated destination pixels. Reject an empty
    /// view or one wider or taller than the source period.
    #[must_use]
    pub fn wrapped_regions(
        self,
        period: Dimensions,
    ) -> Option<[Option<PresentationRegion>; 4]> {
        let width = period.width.get();
        let height = period.height.get();
        if self.width == 0
            || self.height == 0
            || self.width > width
            || self.height > height
        {
            return None;
        }
        let origin_x = self.x.checked_rem(width)?;
        let origin_y = self.y.checked_rem(height)?;
        let first_width = self.width.min(width.saturating_sub(origin_x));
        let first_height = self.height.min(height.saturating_sub(origin_y));
        let left = (origin_x, 0, first_width);
        let right = (0, first_width, self.width.saturating_sub(first_width));
        let top = (origin_y, 0, first_height);
        let bottom =
            (0, first_height, self.height.saturating_sub(first_height));
        let region = |(sx, dx, span_x), (sy, dy, span_y)| {
            (span_x != 0 && span_y != 0).then_some(PresentationRegion {
                destination: Self {
                    height: span_y,
                    width: span_x,
                    x: dx,
                    y: dy,
                },
                source: Self {
                    height: span_y,
                    width: span_x,
                    x: sx,
                    y: sy,
                },
            })
        };
        Some([
            region(left, top),
            region(right, top),
            region(left, bottom),
            region(right, bottom),
        ])
    }
}

/// Center an unchanged design in an expanded logical viewport.
#[must_use]
pub const fn centered_expansion(design: u32, viewport: u32) -> u32 {
    viewport.saturating_sub(design) >> 1
}

/// Distribute extra viewport space across evenly spaced anchor slots.
///
/// A single slot is centered. Multiple slots preserve both outer anchors.
/// Returns None for an index outside the nonempty anchor set.
#[must_use]
pub fn expanded_anchor(
    design: u32,
    viewport: u32,
    index: u32,
    count: NonZeroU32,
) -> Option<u32> {
    if index >= count.get() {
        return None;
    }
    if count.get() == 1 {
        return Some(centered_expansion(design, viewport));
    }
    let extra = u64::from(viewport.saturating_sub(design));
    let span = u64::from(count.get().saturating_sub(1));
    let offset = extra.checked_mul(u64::from(index))?.checked_div(span)?;
    u32::try_from(offset).ok()
}

/// Return an exact RGBA row segment, with no wrapping into another row.
/// Coordinates and lengths are caller-selected; arithmetic is checked.
#[must_use]
pub fn rgba_row(
    pixels: &[u8],
    width: u32,
    origin: (u32, u32),
    columns: u32,
) -> Option<&[u8]> {
    let (column, row) = origin;
    if width == 0 || column.checked_add(columns)? > width {
        return None;
    }
    let stride = u64::from(width);
    let start = u64::from(row)
        .checked_mul(stride)?
        .checked_add(u64::from(column))?;
    let end = start.checked_add(u64::from(columns))?;
    let first = usize::try_from(start.checked_mul(4)?).ok()?;
    let last = usize::try_from(end.checked_mul(4)?).ok()?;
    pixels.get(first..last)
}

/// Verify a nonempty RGBA mask with a caller-selected visible colour.
/// Every other pixel must match the supplied transparent colour.
#[must_use]
pub fn solid_mask(
    pixels: &[u8],
    transparent: [u8; 4],
    visible: [u8; 4],
) -> bool {
    let (rgba, remainder) = pixels.as_chunks::<4>();
    transparent != visible
        && remainder.is_empty()
        && rgba.contains(&visible)
        && rgba
            .iter()
            .all(|pixel| *pixel == transparent || *pixel == visible)
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::time::Duration;

    use super::{
        Dimensions, DisplaySettings, FramePacing, PresentationMode, SpriteRect,
        StripViewport, Viewport, WindowMode, centered_expansion,
        expanded_anchor, rgba_row, solid_mask,
    };

    type SampledPixel = (u32, u32, Option<u32>, Option<u32>);

    fn nz(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
    }

    #[test]
    fn wrapped_regions_cover_each_pixel_once_with_exact_source() {
        let period = Dimensions::new(nz(7), nz(5));
        for horizontal in 0..15 {
            for vertical in 0..11 {
                let view = Viewport {
                    height: 4,
                    width: 6,
                    x: horizontal,
                    y: vertical,
                };
                verify_wrapped_view(view, period);
            }
        }
    }

    fn verify_wrapped_view(view: Viewport, period: Dimensions) {
        let regions = view.wrapped_regions(period);
        let mut actual: Vec<_> = regions
            .into_iter()
            .flatten()
            .flatten()
            .flat_map(region_pixels)
            .collect();
        actual.sort_unstable();
        let expected: Vec<_> = (0..view.width)
            .flat_map(|column| {
                (0..view.height).map(move |row| {
                    (
                        column,
                        row,
                        view.x
                            .saturating_add(column)
                            .checked_rem(period.width.get()),
                        view.y
                            .saturating_add(row)
                            .checked_rem(period.height.get()),
                    )
                })
            })
            .collect();
        assert_eq!(actual, expected);
    }

    fn region_pixels(
        region: super::PresentationRegion,
    ) -> impl Iterator<Item = SampledPixel> {
        let source = region.source;
        let destination = region.destination;
        assert_eq!(source.width, destination.width);
        assert_eq!(source.height, destination.height);
        (0..source.width).flat_map(move |column| {
            (0..source.height).map(move |row| {
                (
                    destination.x.saturating_add(column),
                    destination.y.saturating_add(row),
                    source.x.checked_add(column),
                    source.y.checked_add(row),
                )
            })
        })
    }

    #[test]
    fn wrapped_regions_admit_full_period_and_large_origins() {
        let period = Dimensions::new(nz(512), nz(256));
        let view = Viewport {
            height: 256,
            width: 512,
            x: u32::MAX,
            y: 255,
        };
        assert_eq!(
            view.wrapped_regions(period)
                .map(|regions| regions.into_iter().flatten().count()),
            Some(4)
        );
        for invalid in [
            Viewport { width: 513, ..view },
            Viewport { height: 257, ..view },
            Viewport { width: 0, ..view },
            Viewport { height: 0, ..view },
        ] {
            assert_eq!(invalid.wrapped_regions(period), None);
        }
        let aligned = Viewport { x: 1024, y: 512, ..view };
        assert_eq!(
            aligned
                .wrapped_regions(period)
                .map(|regions| regions.into_iter().flatten().count()),
            Some(1)
        );
    }

    #[test]
    fn padded_strip_camera_is_configurable_and_checked() {
        let policy = StripViewport {
            design_width: 320,
            maximum_width: 512,
            side_inset: 16,
        };
        assert_eq!(policy.width(732), Some(512));
        assert_eq!(policy.origin(732, 320, 444), Some(444));
        assert_eq!(policy.origin(732, 512, 444), Some(236));
        assert_eq!(policy.origin(732, 513, 444), None);
        let larger = StripViewport {
            design_width: 1024,
            maximum_width: 100_000,
            side_inset: 32,
        };
        assert_eq!(larger.origin(100_000, 80_000, 60_000), Some(20_032));
        assert_eq!(larger.origin(u32::MAX, 1024, 0), None);
    }

    #[test]
    fn configurable_pixel_geometry_and_masks_are_exact() {
        let rectangle = SpriteRect {
            height: nz(2),
            width: nz(u32::MAX),
            x: -1,
            y: 0,
        };
        assert!(rectangle.contains_pixel(0, 1));
        assert!(rectangle.contains_pixel(u32::MAX - 2, 1));
        assert!(!rectangle.contains_pixel(u32::MAX - 1, 1));
        assert!(!rectangle.contains_pixel(0, 2));
        assert_eq!(rectangle.left_columns(0), None);
        assert_eq!(
            rectangle.left_columns(65_536).map(|rect| rect.width),
            Some(nz(65_536))
        );
    }

    #[test]
    fn rgba_rows_and_configurable_masks_reject_invalid_input() {
        let pixels = [1u8; 32];
        assert_eq!(rgba_row(&pixels, 4, (1, 1), 2), Some(&pixels[20..28]));
        assert_eq!(rgba_row(&pixels, 4, (3, 0), 2), None);
        assert_eq!(rgba_row(&pixels, u32::MAX, (0, u32::MAX), 1), None);
        assert!(solid_mask(&[1, 2, 3, 9], [0; 4], [1, 2, 3, 9]));
        assert!(!solid_mask(&[0; 4], [0; 4], [1, 2, 3, 9]));
    }

    #[test]
    fn expansion_preserves_outer_anchors_without_overflow() {
        for (index, expected) in [0, 14, 28, 42].into_iter().enumerate() {
            let slot = u32::try_from(index).unwrap_or(u32::MAX);
            assert_eq!(expanded_anchor(320, 362, slot, nz(4)), Some(expected));
            assert_eq!(expanded_anchor(320, 300, slot, nz(4)), Some(0));
        }
        assert_eq!(expanded_anchor(320, 363, 0, nz(1)), Some(21));
        assert_eq!(expanded_anchor(320, 363, 4, nz(4)), None);
        assert_eq!(centered_expansion(320, 300), 0u32);
        assert_eq!(
            expanded_anchor(
                0,
                u32::MAX,
                u32::MAX.saturating_sub(1),
                nz(u32::MAX)
            ),
            Some(u32::MAX),
        );
    }

    #[test]
    fn sprite_centering_preserves_extent_and_saturates() {
        let rectangle = SpriteRect {
            height: nz(5),
            width: nz(8),
            x: 9,
            y: 10,
        };
        assert_eq!(
            rectangle.centered_at(100, 50),
            SpriteRect {
                x: 96,
                y: 48,
                ..rectangle
            },
            "centering retains dimensions and floors odd half extents",
        );
        assert_eq!(
            rectangle.centered_at(i32::MIN, i32::MIN),
            SpriteRect {
                x: i32::MIN,
                y: i32::MIN,
                ..rectangle
            },
            "extreme negative coordinates never wrap",
        );
        let largest = SpriteRect {
            width: nz(u32::MAX),
            ..rectangle
        };
        assert_eq!(largest.centered_at(i32::MAX, 0).x, 0i32);
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
    fn fill_protects_the_entire_bottom_strip_across_drawable_shapes() {
        let design = Dimensions::new(nz(320), nz(304));
        for (width, height) in [(1920, 1080), (720, 1280), (2, 2)] {
            let surface = Dimensions::new(nz(width), nz(height));
            let layout = PresentationMode::Fill.layout_with_protected_bottom(
                surface,
                design,
                Some(nz(80)),
            );
            assert!(layout.is_some_and(|value| {
                value.design == design
                    && value.surface == surface
                    && value.destination.x == 0
                    && value.destination.y == 0
                    && value.destination.width == width
                    && value.source.y.saturating_add(value.source.height) <= 224
                    && value.regions().into_iter().flatten().count() == 2
            }));
            assert!(layout.and_then(|value| value.protected).is_some_and(
                |panel| {
                    panel.source
                        == (Viewport {
                            height: 80,
                            width: 320,
                            x: 0,
                            y: 224,
                        })
                        && panel.destination.width > 0
                        && panel.destination.height > 0
                        && panel
                            .destination
                            .x
                            .saturating_add(panel.destination.width)
                            <= width
                        && panel
                            .destination
                            .y
                            .saturating_add(panel.destination.height)
                            <= height
                }
            ));
            assert!(layout.is_some_and(|value| value.protected.is_some_and(
                |panel| { value.destination.height <= panel.destination.y }
            )));
        }
    }

    #[test]
    fn protected_bottom_preserves_existing_modes_and_handles_tiny_drawables() {
        let design = Dimensions::new(nz(320), nz(304));
        let surface = Dimensions::new(nz(1920), nz(1080));
        let bottom = Some(nz(80));
        for mode in [
            PresentationMode::Adaptive,
            PresentationMode::AspectFit,
            PresentationMode::Integer,
            PresentationMode::Stretch,
        ] {
            assert_eq!(
                mode.layout_with_protected_bottom(surface, design, bottom),
                Some(mode.layout(surface, design))
            );
        }
        let tiny = Dimensions::new(nz(20), nz(1));
        assert_eq!(
            PresentationMode::Fill
                .layout_with_protected_bottom(tiny, design, bottom),
            Some(PresentationMode::AspectFit.layout(tiny, design))
        );
        assert_eq!(
            PresentationMode::Fill.layout_with_protected_bottom(
                tiny,
                design,
                Some(design.height)
            ),
            None
        );
        assert_eq!(
            PresentationMode::Fill
                .layout_with_protected_bottom(tiny, design, None),
            Some(PresentationMode::Fill.layout(tiny, design))
        );
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
