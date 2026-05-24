use std::time::{Duration, Instant};

use ratatui::prelude::*;
use ratatui::widgets::Clear;

use crate::tui::theme;

const TOTAL_DURATION: Duration = Duration::from_millis(9950);

// ── Hub center ──────────────────────────────────────────
const HUB_X: u16 = 40;
const HUB_Y: u16 = 9;

// ── Host nodes ──────────────────────────────────────────
#[derive(Clone, Copy)]
struct HostNode {
    col: u16,
    row: u16,
    label: &'static str,
    label_col: u16,
    dot_time: f64,
    label_time: f64,
    spoke_start: f64,
}

const HOSTS: [HostNode; 6] = [
    HostNode {
        col: 14,
        row: 2,
        label: "api-1",
        label_col: 8,
        dot_time: 0.20,
        label_time: 0.35,
        spoke_start: 2.00,
    },
    HostNode {
        col: 66,
        row: 2,
        label: "db-01",
        label_col: 68,
        dot_time: 0.45,
        label_time: 0.60,
        spoke_start: 2.15,
    },
    HostNode {
        col: 10,
        row: 9,
        label: "cache-a",
        label_col: 2,
        dot_time: 0.70,
        label_time: 0.85,
        spoke_start: 2.30,
    },
    HostNode {
        col: 70,
        row: 9,
        label: "bastion",
        label_col: 72,
        dot_time: 0.95,
        label_time: 1.10,
        spoke_start: 2.45,
    },
    HostNode {
        col: 14,
        row: 15,
        label: "edge-fra",
        label_col: 5,
        dot_time: 1.20,
        label_time: 1.35,
        spoke_start: 2.60,
    },
    HostNode {
        col: 66,
        row: 15,
        label: "worker-1",
        label_col: 68,
        dot_time: 1.45,
        label_time: 1.60,
        spoke_start: 2.75,
    },
];

const SPOKE_DURATION: f64 = 0.85;

// ── Hub evolution ───────────────────────────────────────
const HUB_STAGES: [(f64, &str, Style); 4] = [
    (2.50, "\u{00B7}", Style::new().fg(theme::GREEN)), // ·
    (3.30, "+", Style::new().fg(theme::GREEN)),        // +
    (
        3.90,
        "\u{25C6}",
        Style::new().fg(theme::BRIGHT).add_modifier(Modifier::BOLD),
    ), // ◆
    (
        4.40,
        "\u{25C9}",
        Style::new().fg(theme::BRIGHT).add_modifier(Modifier::BOLD),
    ), // ◉
];

// ── Wordmark / tagline / parenthetical ──────────────────
const WORDMARK: &str = "\u{2500} S S H u b \u{2500}";
const WORDMARK_ROW: u16 = 18;
const WORDMARK_COL: u16 = 34;
const WORDMARK_START: f64 = 5.30;
const WORDMARK_CPS: f64 = 9.0;

const TAGLINE: &str = "secure shell  \u{00B7}  undefined behavior";
const TAGLINE_ROW: u16 = 20;
const TAGLINE_COL: u16 = 22;
const TAGLINE_START: f64 = 7.00;
const TAGLINE_CPS: f64 = 22.0;

const PAREN: &str = "(we don\u{2019}t ship that one)";
const PAREN_ROW: u16 = 21;
const PAREN_COL: u16 = 28;
const PAREN_START: f64 = 8.70;
const PAREN_CPS: f64 = 22.0;

const PROMPT_TIME: f64 = 9.90;
const PROMPT_ROW: u16 = 23;
const PROMPT_COL: u16 = 28;

const HUB_LABEL_TIME: f64 = 4.70;
const ANIMATION_DONE: f64 = 9.95;

// ── Bresenham spoke cells ───────────────────────────────

#[derive(Clone)]
struct SpokeCell {
    x: u16,
    y: u16,
    glyph: char,
}

fn bresenham_cells(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<SpokeCell> {
    let dx = (x1 - x0).unsigned_abs() as i32;
    let dy = (y1 - y0).unsigned_abs() as i32;
    let sx: i32 = if x0 < x1 { 1 } else { -1 };
    let sy: i32 = if y0 < y1 { 1 } else { -1 };
    let mut err = dx - dy;
    let mut x = x0;
    let mut y = y0;
    let mut out = Vec::new();

    loop {
        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            x += sx;
        }
        if e2 < dx {
            err += dx;
            y += sy;
        }
        if x == x1 && y == y1 {
            break;
        }
        let glyph = if dy == 0 {
            '\u{2500}' // ─
        } else if dx == 0 {
            '\u{2502}' // │
        } else if (sx > 0 && sy > 0) || (sx < 0 && sy < 0) {
            '\u{2572}' // ╲
        } else {
            '\u{2571}' // ╱
        };
        out.push(SpokeCell {
            x: x as u16,
            y: y as u16,
            glyph,
        });
    }
    out
}

// ── AnimationState ──────────────────────────────────────

pub struct AnimationState {
    start: Instant,
    spokes: Vec<Vec<SpokeCell>>,
    too_small: bool,
}

impl AnimationState {
    pub fn new(width: u16, height: u16) -> Self {
        let too_small = width < 80 || height < 24;

        let spokes: Vec<Vec<SpokeCell>> = if too_small {
            Vec::new()
        } else {
            HOSTS
                .iter()
                .map(|h| bresenham_cells(h.col as i32, h.row as i32, HUB_X as i32, HUB_Y as i32))
                .collect()
        };

        Self {
            start: Instant::now(),
            spokes,
            too_small,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.start.elapsed() >= TOTAL_DURATION
    }

    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Clear, area);

        if self.too_small {
            self.render_compact(frame);
            return;
        }

        let t = self.start.elapsed().as_secs_f64();

        // Center the 80×24 animation grid in the actual terminal
        let ox = area.width.saturating_sub(80) / 2;
        let oy = area.height.saturating_sub(24) / 2;

        // ── Host dots and labels ───────────────────────
        for host in &HOSTS {
            if t >= host.dot_time {
                set_str(
                    frame,
                    host.col + ox,
                    host.row + oy,
                    "\u{25CF}",
                    Style::default().fg(theme::GREEN),
                );
            }
            if t >= host.label_time {
                set_str(
                    frame,
                    host.label_col + ox,
                    host.row + oy,
                    host.label,
                    Style::default().fg(theme::TEXT),
                );
            }
        }

        // ── Spokes ─────────────────────────────────────
        let dim_style = Style::default().fg(theme::DIM);
        for (i, host) in HOSTS.iter().enumerate() {
            if t < host.spoke_start {
                continue;
            }
            let cells = &self.spokes[i];
            let p = ((t - host.spoke_start) / SPOKE_DURATION).clamp(0.0, 1.0);
            let visible = (cells.len() as f64 * p).ceil() as usize;
            for cell in cells.iter().take(visible) {
                let s = format!("{}", cell.glyph);
                set_str(frame, cell.x + ox, cell.y + oy, &s, dim_style);
            }
        }

        // ── Hub glyph ──────────────────────────────────
        // Halo: between 4.40 and 9.95, toggle at ~2 Hz
        if (4.40..ANIMATION_DONE).contains(&t) {
            let halo_on = ((t * 2.0) as u32).is_multiple_of(2);
            if halo_on {
                let bg_style = Style::default().bg(theme::SEL_BG);
                set_str(frame, HUB_X - 1 + ox, HUB_Y + oy, " ", bg_style);
                set_str(frame, HUB_X + 1 + ox, HUB_Y + oy, " ", bg_style);
                // The hub glyph itself will overwrite (HUB_X, HUB_Y)
            }
        }

        let mut hub_glyph: Option<(&str, Style)> = None;
        for &(time, glyph, style) in HUB_STAGES.iter().rev() {
            if t >= time {
                hub_glyph = Some((glyph, style));
                break;
            }
        }
        if let Some((glyph, style)) = hub_glyph {
            // If halo is on, add selBg background
            let final_style = if (4.40..ANIMATION_DONE).contains(&t) {
                let halo_on = ((t * 2.0) as u32).is_multiple_of(2);
                if halo_on {
                    style.bg(theme::SEL_BG)
                } else {
                    style
                }
            } else {
                style
            };
            set_str(frame, HUB_X + ox, HUB_Y + oy, glyph, final_style);
        }

        // ── Hub label ──────────────────────────────────
        if t >= HUB_LABEL_TIME {
            if t >= ANIMATION_DONE {
                // Flash at 1.2 Hz
                let flash_on = ((t * 1.2) as u32).is_multiple_of(2);
                if flash_on {
                    set_str(
                        frame,
                        39 + ox,
                        10 + oy,
                        "hub",
                        Style::default()
                            .fg(theme::AMBER)
                            .add_modifier(Modifier::BOLD),
                    );
                }
            } else {
                set_str(
                    frame,
                    39 + ox,
                    10 + oy,
                    "hub",
                    Style::default().fg(theme::MUTE),
                );
            }
        }

        // ── Wordmark ───────────────────────────────────
        if t >= WORDMARK_START {
            render_typing_wordmark(frame, t, ox, oy);
        }

        // ── Tagline ────────────────────────────────────
        if t >= TAGLINE_START {
            render_typing_tagline(frame, t, ox, oy);
        }

        // ── Parenthetical ──────────────────────────────
        if t >= PAREN_START {
            let chars_visible = ((t - PAREN_START) * PAREN_CPS).floor() as usize;
            let visible: String = PAREN.chars().take(chars_visible).collect();
            set_str(
                frame,
                PAREN_COL + ox,
                PAREN_ROW + oy,
                &visible,
                Style::default().fg(theme::DIM),
            );
        }

        // ── Prompt ─────────────────────────────────────
        if t >= PROMPT_TIME {
            render_prompt(frame, t, ox, oy);
        }
    }

    fn render_compact(&self, frame: &mut Frame) {
        let area = frame.area();
        let cx = area.width / 2;
        let cy = area.height / 2;

        let label = "SSHub";
        let x = cx.saturating_sub(label.len() as u16 / 2);
        if cy < area.height {
            set_str(
                frame,
                x,
                cy,
                label,
                Style::default()
                    .fg(theme::BRIGHT)
                    .add_modifier(Modifier::BOLD),
            );
        }
        let hint = "press Enter";
        let hx = cx.saturating_sub(hint.len() as u16 / 2);
        let hy = cy + 2;
        if hy < area.height {
            set_str(frame, hx, hy, hint, Style::default().fg(theme::MUTE));
        }
    }
}

// ── Typed text helpers ──────────────────────────────────

fn render_typing_wordmark(frame: &mut Frame, t: f64, ox: u16, oy: u16) {
    let chars_visible = ((t - WORDMARK_START) * WORDMARK_CPS).floor() as usize;
    // Wordmark: "─ S S H u b ─"
    // Indices:   0123456789...
    // Chars 0-7 ("─ S S H ") = bright bold
    // Chars 8-10 ("u b") = amber bold
    // Chars 11-12 (" ─") = bright bold
    let bright_bold = Style::default()
        .fg(theme::BRIGHT)
        .add_modifier(Modifier::BOLD);
    let amber_bold = Style::default()
        .fg(theme::AMBER)
        .add_modifier(Modifier::BOLD);

    let wordmark_chars: Vec<char> = WORDMARK.chars().collect();
    let mut col = WORDMARK_COL + ox;

    for (i, &ch) in wordmark_chars.iter().enumerate() {
        if i >= chars_visible {
            break;
        }
        let style = if i <= 7 {
            bright_bold
        } else if i <= 10 {
            amber_bold
        } else {
            bright_bold
        };
        let s = format!("{}", ch);
        set_str(frame, col, WORDMARK_ROW + oy, &s, style);
        col += 1;
    }
}

fn render_typing_tagline(frame: &mut Frame, t: f64, ox: u16, oy: u16) {
    let chars_visible = ((t - TAGLINE_START) * TAGLINE_CPS).floor() as usize;
    // "secure shell  ·  undefined behavior"
    //  0..15 = "secure shell  · " (16 chars including trailing space) -> mute
    //  Wait, let's count precisely:
    //  s e c u r e   s h e l l     ·     u n d e f i n e d   b e h a v i o r
    //  The mute portion is "secure shell  ·  " (17 chars, indices 0-16)
    //  The amber portion is "undefined behavior" (18 chars, indices 17-34)
    let tagline_chars: Vec<char> = TAGLINE.chars().collect();
    // Find where "undefined" starts
    let amber_start = TAGLINE.find("undefined").unwrap_or(tagline_chars.len());
    let amber_start_idx = TAGLINE[..amber_start].chars().count();

    let mute_style = Style::default().fg(theme::MUTE);
    let amber_style = Style::default().fg(theme::AMBER);

    let mut col = TAGLINE_COL + ox;
    for (i, &ch) in tagline_chars.iter().enumerate() {
        if i >= chars_visible {
            break;
        }
        let style = if i < amber_start_idx {
            mute_style
        } else {
            amber_style
        };
        let s = format!("{}", ch);
        set_str(frame, col, TAGLINE_ROW + oy, &s, style);
        col += 1;
    }
}

fn render_prompt(frame: &mut Frame, t: f64, ox: u16, oy: u16) {
    // "↵ press Enter to continue ▌"
    // ↵ in bright, " press " in mute, "Enter" in bright, " to continue " in mute, ▌ in green (blinking)
    let mute_style = Style::default().fg(theme::MUTE);
    let bright_style = Style::default().fg(theme::BRIGHT);

    let mut col = PROMPT_COL + ox;
    let row = PROMPT_ROW + oy;

    // ↵
    set_str(frame, col, row, "\u{21B5}", bright_style);
    col += 1;

    // " press "
    set_str(frame, col, row, " press ", mute_style);
    col += 7;

    // "Enter"
    set_str(frame, col, row, "Enter", bright_style);
    col += 5;

    // " to continue "
    set_str(frame, col, row, " to continue ", mute_style);
    col += 13;

    // ▌ blinking cursor at 1.6 Hz
    let cursor_on = ((t * 1.6) as u32).is_multiple_of(2);
    if cursor_on {
        set_str(
            frame,
            col,
            row,
            "\u{258C}",
            Style::default().fg(theme::GREEN),
        );
    }
}

// ── Buffer helper ───────────────────────────────────────

fn set_str(frame: &mut Frame, x: u16, y: u16, text: &str, style: Style) {
    let area = frame.area();
    if y >= area.height || x >= area.width {
        return;
    }
    let buf = frame.buffer_mut();
    // Use the buffer's set_string method — it handles clipping
    buf.set_string(x, y, text, style);
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn animation_not_complete_immediately() {
        let state = AnimationState::new(80, 24);
        assert!(!state.is_complete());
    }

    #[test]
    fn render_does_not_panic_on_test_backend() {
        let state = AnimationState::new(80, 24);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| state.render(frame)).unwrap();
    }

    #[test]
    fn render_does_not_panic_on_small_terminal() {
        let state = AnimationState::new(10, 5);
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| state.render(frame)).unwrap();
    }
}
