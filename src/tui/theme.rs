//! What is left of the fixed renderer palette: the mappings that decide *which*
//! role a value belongs to, never what colour that role has.
//!
//! Colours come from `ResolvedTheme` alone; [`legacy`] (`#[cfg(test)]`) is the
//! frozen `default` palette the parity tests compare against.

use crate::theme::catalog::ColorRole;

/// The global status role a status string maps to.
///
/// The mapping is fixed by the spec (`ok|launched|online|up → success`,
/// `slow|idle|retry|warning → warning`, `down|fail|error|unreachable → error`,
/// everything else `unknown`) and lives here so the surfaces that read the
/// global `components.status.*` family cannot drift apart from each other.
///
/// The audit tab deliberately does **not** use this: it owns an identical
/// mapping onto its own `components.audit.*` family, because a theme must be
/// able to retune the audit log without moving every status dot in the app.
pub fn status_role(status: &str) -> ColorRole {
    match status {
        "ok" | "launched" | "online" | "up" => ColorRole::StatusSuccess,
        "slow" | "idle" | "retry" | "warning" => ColorRole::StatusWarning,
        "down" | "fail" | "error" | "unreachable" => ColorRole::StatusError,
        _ => ColorRole::StatusUnknown,
    }
}

/// Sparkline glyph ramp, one eighth of a cell per step.
pub const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// The frozen pre-theme-system palette, kept as a **test-only witness**.
///
/// Nothing productive may use these again — that is the point of the migration.
/// They stay because the `default` parity proofs have to compare against a value
/// that was *not* derived from `ROLE_SPECS`: deriving the expectation from the
/// same table the role resolves from is circular, and hid four real regressions.
/// Hand-transcribing a legacy call is only meaningful while the call it
/// transcribes still exists somewhere to be read.
#[cfg(test)]
pub mod legacy {
    use ratatui::style::{Color, Modifier, Style};

    // ── Palette ──────────────────────────────────────────────

    pub const BG: Color = Color::Rgb(0x0b, 0x0d, 0x10);
    pub const BG_DEEP: Color = Color::Rgb(0x06, 0x08, 0x0a);
    pub const BORDER: Color = Color::Rgb(0x1f, 0x2a, 0x24);
    pub const DIM: Color = Color::Rgb(0x3d, 0x4a, 0x44);
    pub const MUTE: Color = Color::Rgb(0x6a, 0x7a, 0x72);
    pub const TEXT: Color = Color::Rgb(0xd6, 0xe1, 0xd4);
    pub const BRIGHT: Color = Color::Rgb(0xc7, 0xe8, 0xc9);
    pub const WHITE: Color = Color::Rgb(0xf4, 0xf8, 0xf3);
    pub const GREEN: Color = Color::Rgb(0x7c, 0xb9, 0x92);
    pub const ACCENT: Color = Color::Rgb(0x9e, 0xc9, 0x9b);
    pub const AMBER: Color = Color::Rgb(0xd6, 0xa7, 0x6b);
    pub const CYAN: Color = Color::Rgb(0x6f, 0xb3, 0xb8);
    pub const RED: Color = Color::Rgb(0xc9, 0x7a, 0x7a);
    pub const SEL_BG: Color = Color::Rgb(0x18, 0x2b, 0x22);
    pub const SEL_FG: Color = Color::Rgb(0xc7, 0xe8, 0xc9);

    // ── Semantic styles ──────────────────────────────────────

    pub fn text() -> Style {
        Style::default().fg(TEXT)
    }
    pub fn bright() -> Style {
        Style::default().fg(BRIGHT)
    }
    pub fn dim() -> Style {
        Style::default().fg(DIM)
    }
    pub fn mute() -> Style {
        Style::default().fg(MUTE)
    }
    pub fn green() -> Style {
        Style::default().fg(GREEN)
    }
    pub fn amber() -> Style {
        Style::default().fg(AMBER)
    }
    pub fn cyan() -> Style {
        Style::default().fg(CYAN)
    }
    pub fn red() -> Style {
        Style::default().fg(RED)
    }
    pub fn white() -> Style {
        Style::default().fg(WHITE)
    }
    pub fn selected() -> Style {
        Style::default().fg(SEL_FG).bg(SEL_BG)
    }
    pub fn heading() -> Style {
        Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD)
    }
    pub fn border() -> Style {
        Style::default().fg(BORDER)
    }
    /// Border for modal popups. Brighter than [`border`] so an overlay reads as
    /// a distinct framed dialog against the dashboard behind it.
    pub fn popup_border() -> Style {
        Style::default().fg(MUTE)
    }
    pub fn inv() -> Style {
        Style::default().fg(BG_DEEP).bg(BRIGHT)
    }

    /// Status dot colour by status string, as the pre-migration renderers had
    /// it. The productive mapping is [`super::status_role`].
    pub fn status_color(status: &str) -> Color {
        match status {
            "ok" | "launched" | "online" | "up" => GREEN,
            "slow" | "idle" | "retry" | "warning" => AMBER,
            "down" | "fail" | "error" | "unreachable" => RED,
            _ => DIM,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status mapping still bands values exactly as the frozen palette did.
    #[test]
    fn the_surviving_mappings_still_band_values_exactly_as_the_palette_did() {
        for status in [
            "ok",
            "launched",
            "online",
            "up",
            "slow",
            "idle",
            "retry",
            "warning",
            "down",
            "fail",
            "error",
            "unreachable",
            "connecting",
            "",
        ] {
            let expected = match legacy::status_color(status) {
                legacy::GREEN => ColorRole::StatusSuccess,
                legacy::AMBER => ColorRole::StatusWarning,
                legacy::RED => ColorRole::StatusError,
                _ => ColorRole::StatusUnknown,
            };
            assert_eq!(status_role(status), expected, "status_role({status:?})");
        }
    }
}
