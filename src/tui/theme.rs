//! SSHub design token colours — from the canonical design spec.
//!
//! All colours are exact hex from the handoff. Roles map to UI semantics,
//! not colour names. Never invent additional colours.

use ratatui::style::{Color, Modifier, Style};

// ── Palette ──────────────────────────────────────────────

pub const BG: Color = Color::Rgb(0x0b, 0x0d, 0x10);
pub const BG_DEEP: Color = Color::Rgb(0x06, 0x08, 0x0a);
pub const CHROME: Color = Color::Rgb(0x15, 0x18, 0x1c);
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
pub fn footer_key() -> Style {
    Style::default().fg(BRIGHT)
}
pub fn footer_label() -> Style {
    Style::default().fg(MUTE)
}
pub fn inv() -> Style {
    Style::default().fg(BG_DEEP).bg(BRIGHT)
}

/// Status dot colour by status string.
pub fn status_color(status: &str) -> Color {
    match status {
        "ok" | "launched" | "online" | "up" => GREEN,
        "slow" | "idle" | "retry" | "warning" => AMBER,
        "down" | "fail" | "error" | "unreachable" => RED,
        _ => DIM,
    }
}

/// Sparkline glyph ramp.
pub const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Map a value 0.0..=1.0 to a sparkline char.
pub fn spark_char(ratio: f64) -> char {
    let idx = ((ratio * 7.0).round() as usize).min(7);
    SPARK[idx]
}

/// Sparkline colour by ratio of max.
pub fn spark_color(ratio: f64) -> Color {
    if ratio < 0.4 {
        GREEN
    } else if ratio < 0.7 {
        AMBER
    } else {
        RED
    }
}
