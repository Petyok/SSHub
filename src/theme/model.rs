//! Theme data model: identifiers and the fully resolved runtime theme.
//!
//! A [`ResolvedTheme`] is the end state of the pipeline (parse → validate →
//! inherit → resolve). It carries no optional values and no unresolved
//! strings: every semantic slot and every component role is a concrete
//! ratatui value, so renderers never fall back at draw time.

use std::fmt;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::theme::catalog::{ColorRole, PaintRole, StyleRole, TintRole};

/// Technical identifier of a theme.
///
/// For user themes this is the file stem of `themes/<id>.toml`, which is why
/// the accepted character set is deliberately narrower than the display name:
/// an id is used to build a path, so anything that could escape the themes
/// directory or vary by filesystem case folding is rejected.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThemeId(String);

/// Why a string is not a usable [`ThemeId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeIdError {
    Empty,
    InvalidCharacter(char),
}

impl fmt::Display for ThemeIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "theme id must not be empty"),
            Self::InvalidCharacter(c) => write!(
                f,
                "invalid character {c:?} in theme id (allowed: a-z, 0-9, '-', '_')"
            ),
        }
    }
}

impl std::error::Error for ThemeIdError {}

impl ThemeId {
    /// Parse and validate a theme id.
    pub fn parse(raw: &str) -> Result<Self, ThemeIdError> {
        if raw.is_empty() {
            return Err(ThemeIdError::Empty);
        }
        if let Some(bad) = raw
            .chars()
            .find(|c| !matches!(c, 'a'..='z' | '0'..='9' | '-' | '_'))
        {
            return Err(ThemeIdError::InvalidCharacter(bad));
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ThemeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How strictly a theme file is checked.
///
/// The CLI checker runs [`ValidationMode::Strict`] so authors see every
/// problem; the runtime runs [`ValidationMode::Compatible`] so a theme written
/// against a newer SSHub, which may name component roles this build does not
/// know yet, still loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationMode {
    Strict,
    Compatible,
}

/// Index of a resolved gradient inside [`ResolvedTheme::gradients`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradientId(usize);

impl GradientId {
    pub fn new(index: usize) -> Self {
        Self(index)
    }

    pub fn index(self) -> usize {
        self.0
    }
}

/// Direction a gradient is sampled along, relative to the component rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientDirection {
    Horizontal,
    Vertical,
    DiagonalDown,
    DiagonalUp,
    Perimeter,
}

/// A paint role's resolved value: a single colour or a named gradient.
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedPaint {
    Solid(Color),
    Gradient(GradientId),
}

/// A tint role's resolved value. `Native` keeps the asset's own colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedTint {
    Native,
    Color(Color),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGradientStop {
    pub position: f32,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGradient {
    pub direction: GradientDirection,
    pub stops: Vec<ResolvedGradientStop>,
}

impl ResolvedGradient {
    /// Colour at relative position `t`, clamped to `0.0..=1.0`.
    ///
    /// Channels are interpolated in sRGB and rounded per the V1 colour rules,
    /// so a gradient renders identically on every platform.
    pub fn sample(&self, t: f32) -> Color {
        let Some(first) = self.stops.first() else {
            return Color::Reset;
        };
        let t = t.clamp(0.0, 1.0);
        let mut lower = first;
        for stop in &self.stops {
            if stop.position <= t {
                lower = stop;
            } else {
                let span = stop.position - lower.position;
                let local = if span <= f32::EPSILON {
                    0.0
                } else {
                    (t - lower.position) / span
                };
                return mix_srgb(lower.color, stop.color, local);
            }
        }
        lower.color
    }
}

/// Linear per-channel sRGB mix; non-RGB endpoints cannot be interpolated and
/// snap to the nearer end instead of silently producing black.
fn mix_srgb(from: Color, to: Color, t: f32) -> Color {
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (from, to) else {
        return if t < 0.5 { from } else { to };
    };
    let channel = |a: u8, b: u8| -> u8 {
        let value = a as f32 + (b as f32 - a as f32) * t;
        value.clamp(0.0, 255.0).round() as u8
    };
    Color::Rgb(channel(r0, r1), channel(g0, g1), channel(b0, b1))
}

/// The fixed semantic core of schema version 1 — exactly 23 slots.
///
/// Component fallbacks only ever reference these names, so overriding one
/// semantic slot re-tints every component that inherits from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedSemantic {
    pub background: Color,
    pub canvas: Color,
    pub surface: Color,
    pub surface_raised: Color,
    pub border: Color,
    pub border_focus: Color,
    pub border_popup: Color,
    pub text: Color,
    pub text_bright: Color,
    pub text_highlight: Color,
    pub text_muted: Color,
    pub text_dim: Color,
    pub text_inverse: Color,
    pub accent: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,
    pub connecting: Color,
    pub exited: Color,
    pub unknown: Color,
}

/// Every component role of the V1 catalogue, indexed by its typed enum.
///
/// Storage is four flat arrays sized by the generated `COUNT` constants: role
/// lookup is an array index, never a string map lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedComponents {
    colors: [Color; ColorRole::COUNT],
    styles: [Style; StyleRole::COUNT],
    paints: [ResolvedPaint; PaintRole::COUNT],
    tints: [ResolvedTint; TintRole::COUNT],
}

impl ResolvedComponents {
    /// Build the component table. Consuming the arrays is what keeps a
    /// resolved theme immutable: there is no other way in or out.
    pub fn new(
        colors: [Color; ColorRole::COUNT],
        styles: [Style; StyleRole::COUNT],
        paints: [ResolvedPaint; PaintRole::COUNT],
        tints: [ResolvedTint; TintRole::COUNT],
    ) -> Self {
        Self {
            colors,
            styles,
            paints,
            tints,
        }
    }
}

/// A validated, fully inherited theme ready to render with.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTheme {
    pub id: ThemeId,
    pub name: String,
    pub description: Option<String>,
    pub semantic: ResolvedSemantic,
    pub gradients: Vec<ResolvedGradient>,
    pub components: ResolvedComponents,
}

impl ResolvedTheme {
    pub fn color(&self, role: ColorRole) -> Color {
        self.components.colors[role as usize]
    }

    pub fn style(&self, role: StyleRole) -> Style {
        self.components.styles[role as usize]
    }

    pub fn paint(&self, role: PaintRole) -> &ResolvedPaint {
        &self.components.paints[role as usize]
    }

    pub fn tint(&self, role: TintRole) -> &ResolvedTint {
        &self.components.tints[role as usize]
    }

    /// The gradient behind a paint role, or `None` when it resolved to a solid
    /// colour. Callers use this to decide whether a cheap solid render needs
    /// the gradient post-pass at all.
    pub fn paint_gradient(&self, role: PaintRole) -> Option<&ResolvedGradient> {
        match self.paint(role) {
            ResolvedPaint::Solid(_) => None,
            ResolvedPaint::Gradient(id) => self.gradients.get(id.index()),
        }
    }

    /// Colour of a paint role at one cell of `area`.
    ///
    /// Solid paints ignore the position; gradients are sampled with the
    /// direction semantics of the V1 spec, where coordinates are always
    /// relative to the component rect rather than the screen.
    pub fn paint_color_at(&self, role: PaintRole, area: Rect, x: u16, y: u16) -> Color {
        match self.paint(role) {
            ResolvedPaint::Solid(color) => *color,
            ResolvedPaint::Gradient(id) => match self.gradients.get(id.index()) {
                Some(gradient) => {
                    gradient.sample(gradient_position(gradient.direction, area, x, y))
                }
                None => Color::Reset,
            },
        }
    }
}

/// Relative sample position of cell `(x, y)` within `area`.
fn gradient_position(direction: GradientDirection, area: Rect, x: u16, y: u16) -> f32 {
    let norm = |value: u16, origin: u16, len: u16| -> f32 {
        if len <= 1 {
            0.0
        } else {
            (value.saturating_sub(origin)) as f32 / (len - 1) as f32
        }
    };
    let hx = norm(x, area.x, area.width);
    let vy = norm(y, area.y, area.height);
    let flat_x = area.height <= 1;
    let flat_y = area.width <= 1;

    match direction {
        GradientDirection::Horizontal => hx,
        GradientDirection::Vertical => vy,
        GradientDirection::DiagonalDown => match (flat_x, flat_y) {
            (true, true) => 0.0,
            (true, false) => hx,
            (false, true) => vy,
            (false, false) => (hx + vy) / 2.0,
        },
        GradientDirection::DiagonalUp => match (flat_x, flat_y) {
            (true, true) => 0.0,
            (true, false) => hx,
            (false, true) => 1.0 - vy,
            (false, false) => (hx + (1.0 - vy)) / 2.0,
        },
        GradientDirection::Perimeter => perimeter_position(area, x, y),
    }
}

/// Position along the clockwise outer ring of `area`, starting at its top-left
/// corner. Degenerate rects fall back to their natural single-line direction.
fn perimeter_position(area: Rect, x: u16, y: u16) -> f32 {
    if area.width <= 1 && area.height <= 1 {
        return 0.0;
    }
    if area.height <= 1 {
        return (x.saturating_sub(area.x)) as f32 / (area.width - 1) as f32;
    }
    if area.width <= 1 {
        return (y.saturating_sub(area.y)) as f32 / (area.height - 1) as f32;
    }

    let w = area.width as u32;
    let h = area.height as u32;
    let dx = x.saturating_sub(area.x) as u32;
    let dy = y.saturating_sub(area.y) as u32;
    let last_x = w - 1;
    let last_y = h - 1;

    let index = if dy == 0 {
        dx
    } else if dx == last_x {
        last_x + dy
    } else if dy == last_y {
        last_x + last_y + (last_x - dx)
    } else if dx == 0 {
        2 * last_x + last_y + (last_y - dy)
    } else {
        // Interior cells are not on the ring; anchor them at the start so a
        // caller that paints too much still gets a defined colour.
        0
    };
    let length = 2 * w + 2 * h - 4;
    index as f32 / (length - 1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_id_accepts_only_v1_filename_characters() {
        for valid in ["default", "high-contrast", "aqua_2", "fire9"] {
            assert!(ThemeId::parse(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "Aqua", "two words", "../aqua", "aqua.toml"] {
            assert!(ThemeId::parse(invalid).is_err(), "{invalid}");
        }
    }
}
