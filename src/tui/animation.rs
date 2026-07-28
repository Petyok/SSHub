use std::time::{Duration, Instant};

use ratatui::prelude::*;
use ratatui::widgets::Clear;

use crate::theme::catalog::{PaintRole, StyleRole};
use crate::theme::model::ResolvedTheme;

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

/// When each hub glyph takes over, and the glyph itself. The *style* is no
/// longer part of the table: it comes from the active theme, so the two
/// constants that survive here are timing and typography.
const HUB_STAGES: [(f64, &str); 4] = [
    (2.50, "\u{00B7}"), // ·
    (3.30, "+"),        // +
    (3.90, "\u{25C6}"), // ◆
    (4.40, "\u{25C9}"), // ◉
];

/// The style of each hub stage under `theme`.
///
/// The first two stages are the hub still assembling (`hub_early`), the last
/// two the assembled hub (`hub_ready`). `animation.hub_flash` is deliberately
/// *not* in this table: it is the pulsing `hub` **label**, which was amber
/// where these glyphs were bright, and folding it in here would have recoloured
/// the finished glyph.
fn hub_stages(theme: &ResolvedTheme) -> [Style; 4] {
    [
        theme.style(StyleRole::AnimationHubEarly),
        theme.style(StyleRole::AnimationHubEarly),
        theme.style(StyleRole::AnimationHubReady),
        theme.style(StyleRole::AnimationHubReady),
    ]
}

// ── Wordmark / tagline / quip ───────────────────────────
const WORDMARK: &str = "\u{2500} S S H u b \u{2500}";
const WORDMARK_ROW: u16 = 18;
const WORDMARK_COL: u16 = 34;
const WORDMARK_START: f64 = 5.30;
const WORDMARK_CPS: f64 = 9.0;

const TAGLINE: &str = "secure shell x undefined behavior";
const TAGLINE_ROW: u16 = 20;
const TAGLINE_COL: u16 = 24;
const TAGLINE_START: f64 = 7.00;
const TAGLINE_CPS: f64 = 22.0;

const QUIP_ROW: u16 = 21;
const QUIP_START: f64 = 8.70;
const QUIP_CPS: f64 = 22.0;

const QUIPS: &[&str] = &[
    "vibe-coded slop",
    "i love pizza",
    "anyone who read this owes me $10",
    "works on my machine",
    "ssh but make it fancy",
    "no warranty express or implied",
    "your ~/.ssh is in good hands probably",
];

fn pick_quip() -> &'static str {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    QUIPS[seed % QUIPS.len()]
}

fn quip_col(quip: &str) -> u16 {
    (80u16.saturating_sub(quip.chars().count() as u16)) / 2
}

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
    quip: &'static str,
    quip_col: u16,
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

        let quip = pick_quip();
        Self {
            start: Instant::now(),
            spokes,
            too_small,
            quip_col: quip_col(quip),
            quip,
        }
    }

    /// The same state, wound forward to `t` seconds in. Test-only: the render
    /// path reads the wall clock, and a role that only appears at 9.9s is not
    /// otherwise reachable from a test.
    #[cfg(test)]
    fn at(width: u16, height: u16, t: f64) -> Self {
        let mut state = Self::new(width, height);
        state.start = Instant::now() - Duration::from_secs_f64(t);
        state
    }

    pub fn is_complete(&self) -> bool {
        self.start.elapsed() >= TOTAL_DURATION
    }

    pub fn render(&self, frame: &mut Frame, theme: &ResolvedTheme) {
        let area = frame.area();
        frame.render_widget(Clear, area);
        // `Clear` only resets the cells; the theme's own animation ground is
        // laid down over them, exactly as `open_popup` does for overlays. Under
        // `default` the role is transparent, so this stays a plain clear.
        crate::tui::blit::fill_paint(
            frame.buffer_mut(),
            area,
            theme,
            PaintRole::AnimationBackground,
        );

        if self.too_small {
            self.render_compact(frame, theme);
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
                    theme.style(StyleRole::AnimationNode),
                );
            }
            if t >= host.label_time {
                set_str(
                    frame,
                    host.label_col + ox,
                    host.row + oy,
                    host.label,
                    theme.style(StyleRole::AnimationNodeLabel),
                );
            }
        }

        // ── Spokes ─────────────────────────────────────
        let dim_style = theme.style(StyleRole::AnimationSpoke);
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
                let bg_style = Style::default().bg(halo_color(theme));
                set_str(frame, HUB_X - 1 + ox, HUB_Y + oy, " ", bg_style);
                set_str(frame, HUB_X + 1 + ox, HUB_Y + oy, " ", bg_style);
                // The hub glyph itself will overwrite (HUB_X, HUB_Y)
            }
        }

        let stages = hub_stages(theme);
        let mut hub_glyph: Option<(&str, Style)> = None;
        for (i, &(time, glyph)) in HUB_STAGES.iter().enumerate().rev() {
            if t >= time {
                hub_glyph = Some((glyph, stages[i]));
                break;
            }
        }
        if let Some((glyph, style)) = hub_glyph {
            // If halo is on, add selBg background
            let final_style = if (4.40..ANIMATION_DONE).contains(&t) {
                let halo_on = ((t * 2.0) as u32).is_multiple_of(2);
                if halo_on {
                    style.bg(halo_color(theme))
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
                        theme.style(StyleRole::AnimationHubFlash),
                    );
                }
            } else {
                set_str(
                    frame,
                    39 + ox,
                    10 + oy,
                    "hub",
                    theme.style(StyleRole::AnimationHubLabel),
                );
            }
        }

        // ── Wordmark ───────────────────────────────────
        if t >= WORDMARK_START {
            render_typing_wordmark(frame, theme, t, ox, oy);
        }

        // ── Tagline ────────────────────────────────────
        if t >= TAGLINE_START {
            render_typing_tagline(frame, theme, t, ox, oy);
        }

        // ── Random quip ──────────────────────────────
        if t >= QUIP_START {
            let chars_visible = ((t - QUIP_START) * QUIP_CPS).floor() as usize;
            let visible: String = self.quip.chars().take(chars_visible).collect();
            set_str(
                frame,
                self.quip_col + ox,
                QUIP_ROW + oy,
                &visible,
                theme.style(StyleRole::AnimationQuip),
            );
        }

        // ── Prompt ─────────────────────────────────────
        if t >= PROMPT_TIME {
            render_prompt(frame, theme, t, ox, oy);
        }
    }

    fn render_compact(&self, frame: &mut Frame, theme: &ResolvedTheme) {
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
                theme.style(StyleRole::AnimationWordmark),
            );
        }
        let hint = "press Enter";
        let hx = cx.saturating_sub(hint.len() as u16 / 2);
        let hy = cy + 2;
        if hy < area.height {
            set_str(
                frame,
                hx,
                hy,
                hint,
                theme.style(StyleRole::AnimationPromptText),
            );
        }
    }
}

// ── Typed text helpers ──────────────────────────────────

fn render_typing_wordmark(frame: &mut Frame, theme: &ResolvedTheme, t: f64, ox: u16, oy: u16) {
    let chars_visible = ((t - WORDMARK_START) * WORDMARK_CPS).floor() as usize;
    // Wordmark: "─ S S H u b ─"
    // Indices:   0123456789...
    // Chars 0-7 ("─ S S H ") = bright bold
    // Chars 8-10 ("u b") = amber bold
    // Chars 11-12 (" ─") = bright bold
    let bright_bold = theme.style(StyleRole::AnimationWordmark);
    let amber_bold = theme.style(StyleRole::AnimationWordmarkAccent);

    let wordmark_chars: Vec<char> = WORDMARK.chars().collect();

    for (i, &ch) in wordmark_chars.iter().enumerate().take(chars_visible) {
        let style = if i <= 7 {
            bright_bold
        } else if i <= 10 {
            amber_bold
        } else {
            bright_bold
        };
        let col = WORDMARK_COL + ox + i as u16;
        let s = format!("{}", ch);
        set_str(frame, col, WORDMARK_ROW + oy, &s, style);
    }
}

fn render_typing_tagline(frame: &mut Frame, theme: &ResolvedTheme, t: f64, ox: u16, oy: u16) {
    let chars_visible = ((t - TAGLINE_START) * TAGLINE_CPS).floor() as usize;
    // "secure shell x undefined behavior" — mute prefix, amber from "undefined"
    let tagline_chars: Vec<char> = TAGLINE.chars().collect();
    // Find where "undefined" starts
    let amber_start = TAGLINE.find("undefined").unwrap_or(tagline_chars.len());
    let amber_start_idx = TAGLINE[..amber_start].chars().count();

    let mute_style = theme.style(StyleRole::AnimationTagline);
    let amber_style = theme.style(StyleRole::AnimationTaglineAccent);

    for (i, &ch) in tagline_chars.iter().enumerate().take(chars_visible) {
        let style = if i < amber_start_idx {
            mute_style
        } else {
            amber_style
        };
        let col = TAGLINE_COL + ox + i as u16;
        let s = format!("{}", ch);
        set_str(frame, col, TAGLINE_ROW + oy, &s, style);
    }
}

fn render_prompt(frame: &mut Frame, theme: &ResolvedTheme, t: f64, ox: u16, oy: u16) {
    // "↵ press Enter to continue ▌"
    // ↵ in bright, " press " in mute, "Enter" in bright, " to continue " in mute, ▌ in green (blinking)
    let mute_style = theme.style(StyleRole::AnimationPromptText);
    let bright_style = theme.style(StyleRole::AnimationPromptKey);

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
            theme.style(StyleRole::AnimationCursor),
        );
    }
}

/// The halo's colour behind the hub glyph.
///
/// `animation.halo` is a `Paint`, so it could carry a gradient — but the halo
/// is three cells, and a gradient sampled over three cells is indistinguishable
/// from its own midpoint. Its solid value is taken instead, which also keeps
/// this off the ring/line painters entirely.
fn halo_color(theme: &ResolvedTheme) -> Color {
    crate::tui::blit::line_color(
        theme,
        PaintRole::AnimationHalo,
        Rect::new(HUB_X.saturating_sub(1), HUB_Y, 3, 1),
    )
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
    use crate::test_support::{resolved_default, resolved_source};
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
        let theme = resolved_default();
        terminal.draw(|frame| state.render(frame, &theme)).unwrap();
    }

    #[test]
    fn render_does_not_panic_on_small_terminal() {
        let state = AnimationState::new(10, 5);
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = resolved_default();
        terminal.draw(|frame| state.render(frame, &theme)).unwrap();
    }

    /// Wind a full-size animation to `t` and render it with `theme`.
    fn frame_at(theme: &ResolvedTheme, t: f64) -> ratatui::buffer::Buffer {
        let state = AnimationState::at(80, 24, t);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| state.render(frame, theme)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn builtin(id: &str) -> ResolvedTheme {
        resolved_source(id, crate::theme::builtins::source(id).unwrap())
    }

    /// The wordmark is drawn from the runtime theme, not from a constant: two
    /// themes must produce two different cells, and the cell must be exactly
    /// what the theme publishes for that role.
    #[test]
    fn animation_uses_runtime_wordmark_and_halo_roles() {
        // t = 8.0: the wordmark has finished typing. t = 5.0: the halo is on
        // (`((t * 2.0) as u32)` is even) and the hub is at its final glyph.
        let default = builtin("default");
        let fire = builtin("fire");

        // The cell's ground is the animation background, which `fire` paints
        // and `default` leaves transparent, so the wordmark itself is compared
        // on the channels the role owns.
        let default_word = frame_at(&default, 8.0)[(WORDMARK_COL, WORDMARK_ROW)].clone();
        let fire_word = frame_at(&fire, 8.0)[(WORDMARK_COL, WORDMARK_ROW)].clone();
        assert_ne!(default_word.fg, fire_word.fg);
        assert_eq!(
            fire_word.fg,
            fire.style(StyleRole::AnimationWordmark).fg.unwrap()
        );
        assert_eq!(
            default_word.fg,
            default.style(StyleRole::AnimationWordmark).fg.unwrap()
        );

        let default_halo = frame_at(&default, 5.0)[(HUB_X - 1, HUB_Y)].bg;
        let fire_halo = frame_at(&fire, 5.0)[(HUB_X - 1, HUB_Y)].bg;
        assert_ne!(default_halo, fire_halo);
        assert_eq!(fire_halo, halo_color(&fire));
        assert_eq!(default_halo, halo_color(&default));
    }

    /// Every cell of the animation, at the moment it exists, against a marker
    /// no other animation role carries. Constants cannot pass this.
    #[test]
    fn every_animation_cell_wears_its_own_role() {
        use crate::test_support::{fg, marker, role_marker_theme};

        const M_BACKGROUND: u32 = 0xc1_0001;
        const M_NODE: u32 = 0xc1_0002;
        const M_NODE_LABEL: u32 = 0xc1_0003;
        const M_SPOKE: u32 = 0xc1_0004;
        const M_HUB_EARLY: u32 = 0xc1_0005;
        const M_HUB_READY: u32 = 0xc1_0006;
        const M_HUB_FLASH: u32 = 0xc1_0007;
        const M_HUB_LABEL: u32 = 0xc1_0008;
        const M_HALO: u32 = 0xc1_0009;
        const M_WORDMARK: u32 = 0xc1_000a;
        const M_WORDMARK_ACCENT: u32 = 0xc1_000b;
        const M_TAGLINE: u32 = 0xc1_000c;
        const M_TAGLINE_ACCENT: u32 = 0xc1_000d;
        const M_QUIP: u32 = 0xc1_000e;
        const M_PROMPT_KEY: u32 = 0xc1_000f;
        const M_PROMPT_TEXT: u32 = 0xc1_0010;
        const M_CURSOR: u32 = 0xc1_0011;

        let theme = role_marker_theme(
            "animation-markers",
            &[
                fg("components.animation.background", M_BACKGROUND),
                fg("components.animation.node", M_NODE),
                fg("components.animation.node_label", M_NODE_LABEL),
                fg("components.animation.spoke", M_SPOKE),
                fg("components.animation.hub_early", M_HUB_EARLY),
                fg("components.animation.hub_ready", M_HUB_READY),
                fg("components.animation.hub_flash", M_HUB_FLASH),
                fg("components.animation.hub_label", M_HUB_LABEL),
                fg("components.animation.halo", M_HALO),
                fg("components.animation.wordmark", M_WORDMARK),
                fg("components.animation.wordmark_accent", M_WORDMARK_ACCENT),
                fg("components.animation.tagline", M_TAGLINE),
                fg("components.animation.tagline_accent", M_TAGLINE_ACCENT),
                fg("components.animation.quip", M_QUIP),
                fg("components.animation.prompt_key", M_PROMPT_KEY),
                fg("components.animation.prompt_text", M_PROMPT_TEXT),
                fg("components.animation.cursor", M_CURSOR),
            ],
        );

        // t = 1.7: every host dot and label is out, no spoke has started.
        let buf = frame_at(&theme, 1.7);
        assert_eq!(
            buf[(79, 0)].bg,
            marker(M_BACKGROUND),
            "the animation ground"
        );
        for host in &HOSTS {
            assert_eq!(
                buf[(host.col, host.row)].fg,
                marker(M_NODE),
                "the dot of {}",
                host.label
            );
            assert_eq!(
                buf[(host.label_col, host.row)].fg,
                marker(M_NODE_LABEL),
                "the label of {}",
                host.label
            );
        }

        // t = 3.5: the spokes are drawing and the hub is still `+`.
        let buf = frame_at(&theme, 3.5);
        let first_spoke = &bresenham_cells(
            HOSTS[0].col as i32,
            HOSTS[0].row as i32,
            HUB_X as i32,
            HUB_Y as i32,
        )[0];
        assert_eq!(
            buf[(first_spoke.x, first_spoke.y)].fg,
            marker(M_SPOKE),
            "the first spoke cell"
        );
        assert_eq!(buf[(HUB_X, HUB_Y)].symbol(), "+");
        assert_eq!(buf[(HUB_X, HUB_Y)].fg, marker(M_HUB_EARLY), "the `+` hub");

        // t = 5.0: the hub is assembled, the halo is on, the label is quiet.
        let buf = frame_at(&theme, 5.0);
        assert_eq!(buf[(HUB_X, HUB_Y)].symbol(), "\u{25c9}");
        assert_eq!(
            buf[(HUB_X, HUB_Y)].fg,
            marker(M_HUB_READY),
            "the assembled hub"
        );
        assert_eq!(
            buf[(HUB_X, HUB_Y)].bg,
            marker(M_HALO),
            "the hub's own halo cell"
        );
        for x in [HUB_X - 1, HUB_X + 1] {
            assert_eq!(buf[(x, HUB_Y)].bg, marker(M_HALO), "the halo cell at {x}");
        }
        assert_eq!(
            buf[(39, 10)].fg,
            marker(M_HUB_LABEL),
            "the quiet `hub` label"
        );

        // t = 8.0: wordmark and tagline are fully typed.
        let buf = frame_at(&theme, 8.0);
        assert_eq!(
            buf[(WORDMARK_COL, WORDMARK_ROW)].fg,
            marker(M_WORDMARK),
            "the wordmark's leading rule"
        );
        assert_eq!(
            buf[(WORDMARK_COL + 8, WORDMARK_ROW)].symbol(),
            "u",
            "the accent run starts at index 8"
        );
        assert_eq!(
            buf[(WORDMARK_COL + 8, WORDMARK_ROW)].fg,
            marker(M_WORDMARK_ACCENT),
            "the `u b` of the wordmark"
        );
        assert_eq!(buf[(TAGLINE_COL, TAGLINE_ROW)].fg, marker(M_TAGLINE));
        let amber_at = TAGLINE.find("undefined").unwrap() as u16;
        assert_eq!(
            buf[(TAGLINE_COL + amber_at, TAGLINE_ROW)].fg,
            marker(M_TAGLINE_ACCENT),
            "the tagline from `undefined` on"
        );

        // t = 10.0: the animation is done — the label pulses on, the prompt is
        // up and its cursor is in its visible half.
        let state = AnimationState::at(80, 24, 10.0);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| state.render(frame, &theme)).unwrap();
        let buf = terminal.backend().buffer().clone();
        assert_eq!(
            buf[(39, 10)].fg,
            marker(M_HUB_FLASH),
            "the pulsing `hub` label"
        );
        assert_eq!(buf[(state.quip_col, QUIP_ROW)].fg, marker(M_QUIP));
        assert_eq!(buf[(PROMPT_COL, PROMPT_ROW)].symbol(), "\u{21b5}");
        assert_eq!(buf[(PROMPT_COL, PROMPT_ROW)].fg, marker(M_PROMPT_KEY));
        assert_eq!(buf[(PROMPT_COL + 1, PROMPT_ROW)].fg, marker(M_PROMPT_TEXT));
        assert_eq!(
            buf[(PROMPT_COL + 8, PROMPT_ROW)].fg,
            marker(M_PROMPT_KEY),
            "`Enter`"
        );
        assert_eq!(buf[(PROMPT_COL + 26, PROMPT_ROW)].symbol(), "\u{258c}");
        assert_eq!(buf[(PROMPT_COL + 26, PROMPT_ROW)].fg, marker(M_CURSOR));
    }

    /// The compact fallback for a terminal under 80x24 reads the same two
    /// roles the full animation uses for the same two pieces of text.
    #[test]
    fn the_compact_animation_wears_the_wordmark_and_prompt_roles() {
        use crate::test_support::{fg, marker, role_marker_theme};

        const M_WORDMARK: u32 = 0xc2_0001;
        const M_PROMPT_TEXT: u32 = 0xc2_0002;
        let theme = role_marker_theme(
            "compact-markers",
            &[
                fg("components.animation.wordmark", M_WORDMARK),
                fg("components.animation.prompt_text", M_PROMPT_TEXT),
            ],
        );

        let state = AnimationState::at(20, 9, 0.0);
        let backend = TestBackend::new(20, 9);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| state.render(frame, &theme)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let (x, y) = crate::test_support::find_text(&buf, "SSHub");
        assert_eq!(buf[(x, y)].fg, marker(M_WORDMARK));
        let (hx, hy) = crate::test_support::find_text(&buf, "press Enter");
        assert_eq!(buf[(hx, hy)].fg, marker(M_PROMPT_TEXT));
    }

    /// Legacy parity, hand-transcribed from the `crate::tui::theme` calls this
    /// screen used to make. Nothing here is derived from `ROLE_SPECS`.
    #[test]
    fn the_animation_reproduces_its_legacy_cells_under_default() {
        use crate::tui::theme::legacy;
        use ratatui::style::Modifier;

        let theme = resolved_default();

        let buf = frame_at(&theme, 1.7);
        assert_eq!(buf[(HOSTS[0].col, HOSTS[0].row)].fg, legacy::GREEN);
        assert_eq!(buf[(HOSTS[0].label_col, HOSTS[0].row)].fg, legacy::TEXT);

        let buf = frame_at(&theme, 3.5);
        assert_eq!(buf[(HUB_X, HUB_Y)].fg, legacy::GREEN, "the `+` hub");
        let spoke = &bresenham_cells(
            HOSTS[0].col as i32,
            HOSTS[0].row as i32,
            HUB_X as i32,
            HUB_Y as i32,
        )[0];
        assert_eq!(buf[(spoke.x, spoke.y)].fg, legacy::DIM);

        let buf = frame_at(&theme, 5.0);
        let hub = buf[(HUB_X, HUB_Y)].clone();
        assert_eq!(hub.fg, legacy::BRIGHT);
        assert!(hub.modifier.contains(Modifier::BOLD), "the hub was bold");
        assert_eq!(buf[(HUB_X - 1, HUB_Y)].bg, legacy::SEL_BG, "the halo");
        assert_eq!(buf[(39, 10)].fg, legacy::MUTE, "the quiet `hub` label");

        let buf = frame_at(&theme, 8.0);
        let word = buf[(WORDMARK_COL, WORDMARK_ROW)].clone();
        assert_eq!(word.fg, legacy::BRIGHT);
        assert!(word.modifier.contains(Modifier::BOLD));
        let accent = buf[(WORDMARK_COL + 8, WORDMARK_ROW)].clone();
        assert_eq!(accent.fg, legacy::AMBER);
        assert!(accent.modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(TAGLINE_COL, TAGLINE_ROW)].fg, legacy::MUTE);
        let amber_at = TAGLINE.find("undefined").unwrap() as u16;
        let tag = buf[(TAGLINE_COL + amber_at, TAGLINE_ROW)].clone();
        assert_eq!(tag.fg, legacy::AMBER);
        assert!(
            !tag.modifier.contains(Modifier::BOLD),
            "the tagline accent was never bold"
        );

        let state = AnimationState::at(80, 24, 10.0);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| state.render(frame, &theme)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let flash = buf[(39, 10)].clone();
        assert_eq!(flash.fg, legacy::AMBER);
        assert!(flash.modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(state.quip_col, QUIP_ROW)].fg, legacy::DIM);
        assert_eq!(buf[(PROMPT_COL, PROMPT_ROW)].fg, legacy::BRIGHT);
        assert_eq!(buf[(PROMPT_COL + 1, PROMPT_ROW)].fg, legacy::MUTE);
        assert_eq!(buf[(PROMPT_COL + 26, PROMPT_ROW)].fg, legacy::GREEN);
    }
}
