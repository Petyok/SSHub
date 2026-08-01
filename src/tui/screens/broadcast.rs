//! Broadcast mode (issue #3) render layer — pure draw functions.
//!
//! Two families of surfaces:
//!  * pre-run overlay stages (`render_pick_target` / `render_command_prompt` /
//!    `render_preview`) — modal popups over the hosts dashboard, mirroring the
//!    `render_sftp_prompt_popup` idiom (Clear + Block/Paragraph);
//!  * the live docked panel (`render_broadcast_panel`) plus its full-screen
//!    zoomed view (`render_broadcast_zoomed`) and the countdown gauge
//!    (`render_countdown_bar`), driven by the #18 panel focus/zoom machinery.
//!
//! Nothing here mutates `App` except the `zoom_window` scroll write-back that
//! issue #18 panels already use.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use std::time::Instant;

use crate::app::App;
use crate::broadcast::{done_count, failures_first, HostState, DISMISS, TOAST_ANIM, TOAST_TTL};
use crate::theme::catalog::{ColorRole, PaintRole, StyleRole};
use crate::theme::model::ResolvedTheme;
use crate::tui::blit;
use crate::tui::tween::ease_out;
use crate::tui::widgets::panel_box::{put_clamped, render_panel_box, zoom_window, BROADCAST_PANEL};

// ── Per-host row presentation ───────────────────────────────

/// Glyph + style + short status label for one host state. Single-cell glyphs
/// keep the row columns aligned across terminals (no emoji double-width).
fn state_row(state: &HostState, theme: &ResolvedTheme) -> (&'static str, Style, String) {
    // The glyph and the word both differ per state, so a terminal that reduces
    // the theme's RGB to the nearest ANSI colour still tells them apart.
    let color = |role: ColorRole| Style::default().fg(theme.color(role));
    match state {
        HostState::Pending => (
            "\u{25cb}",
            color(ColorRole::BroadcastPending),
            "pending".to_string(),
        ),
        HostState::Running => (
            "\u{25cf}",
            color(ColorRole::BroadcastRunning),
            "running".to_string(),
        ),
        HostState::Done { exit: 0 } => (
            "\u{2713}",
            color(ColorRole::BroadcastSuccess),
            "exit 0".to_string(),
        ),
        HostState::Done { exit } => (
            "\u{2717}",
            color(ColorRole::BroadcastError),
            format!("exit {exit}"),
        ),
        HostState::Failed { .. } => (
            "\u{2717}",
            color(ColorRole::BroadcastError),
            "failed".to_string(),
        ),
    }
}

/// Trailing detail (failure reason / stderr snippet) shown after the status
/// label. Empty for healthy/pending/running rows.
fn state_detail(result: &crate::broadcast::HostResult) -> String {
    match &result.state {
        HostState::Failed { reason } => reason.clone(),
        HostState::Done { exit } if *exit != 0 => result
            .stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

/// Draw a single host row into `buf` at row `y`, inside `[inner_x, right_lim)`.
fn draw_host_row(
    buf: &mut Buffer,
    y: u16,
    inner_x: u16,
    right_lim: u16,
    result: &crate::broadcast::HostResult,
    theme: &ResolvedTheme,
) {
    if inner_x >= right_lim {
        return;
    }
    let inner_w = (right_lim - inner_x) as usize;
    let (glyph, gstyle, label) = state_row(&result.state, theme);
    let body = theme.style(StyleRole::TextPrimary);

    // Glyph column (1 cell + space).
    buf.set_string(inner_x, y, glyph, gstyle);
    let mut col = inner_x + 2;
    if col >= right_lim {
        return;
    }

    // Host name — left column, roughly a third of the width, min 8.
    let name_w = (inner_w / 3).clamp(8, 22).min((right_lim - col) as usize);
    col += put_clamped(buf, col, y, &result.host_name, body, name_w);
    // Pad the name column so the status labels line up.
    while col < inner_x + 2 + name_w as u16 && col < right_lim {
        buf.set_string(col, y, " ", body);
        col += 1;
    }
    col += 1;
    if col >= right_lim {
        return;
    }

    // Status label, coloured like the glyph.
    col += put_clamped(buf, col, y, &label, gstyle, (right_lim - col) as usize);
    col += 1;

    // Optional failure detail in dim.
    let detail = state_detail(result);
    if !detail.is_empty() && col < right_lim {
        put_clamped(
            buf,
            col,
            y,
            &detail,
            theme.style(StyleRole::BroadcastDetail),
            (right_lim - col) as usize,
        );
    }
}

/// Header title + count badge shared by the docked and zoomed views.
fn header_parts(bc: &crate::app::BroadcastState) -> (String, String) {
    let title = format!("cast: {} \u{00b7} {}", bc.command, bc.target_label);
    let fails = crate::broadcast::failure_count(&bc.results);
    let badge = if fails > 0 {
        // Surface the failure count right in the title bar so errors are obvious
        // at a glance even when the failing rows scroll out of a short panel.
        format!(
            "{}/{} \u{00b7} {}\u{2717}",
            done_count(&bc.results),
            bc.results.len(),
            fails
        )
    } else {
        format!("{}/{}", done_count(&bc.results), bc.results.len())
    };
    (title, badge)
}

// ── Live docked panel ───────────────────────────────────────

/// Live docked panel drawn into `area` (already positioned/animated by the
/// caller). `focused` drives the accent border (issue #18). Header
/// `cast: <cmd> · <target> · N/total`, per-host rows (`failures_first`), and a
/// countdown gauge along the bottom when Settling.
pub fn render_broadcast_panel(frame: &mut Frame, area: Rect, app: &App, focused: bool) {
    let Some(bc) = app.broadcast.as_ref() else {
        return;
    };
    if area.width < 4 || area.height < 3 {
        return;
    }

    // Float overlay: wipe the cells beneath first so the dashboard grid behind
    // the docked panel never bleeds through empty interior rows / trailing cells.
    frame.render_widget(Clear, area);
    let (title, badge) = header_parts(bc);
    render_panel_box(
        frame.buffer_mut(),
        area,
        &title,
        BROADCAST_PANEL.with_badge(&badge),
        focused,
        app.theme(),
    );

    let inner_x = area.x + 2;
    let right_lim = area.x + area.width - 1; // last col is the border
    let bottom = area.y + area.height - 1; // border row

    // When settling, reserve the last inner row for the countdown gauge.
    let settling_frac = match &bc.phase {
        crate::app::BroadcastPhase::Settling { done_at } => {
            let elapsed = done_at.elapsed().as_secs_f32();
            Some((elapsed / DISMISS.as_secs_f32()).clamp(0.0, 1.0))
        }
        _ => None,
    };
    let rows_bottom = if settling_frac.is_some() {
        bottom.saturating_sub(1)
    } else {
        bottom
    };

    let order = failures_first(&bc.results);
    let capacity = rows_bottom.saturating_sub(area.y + 1) as usize;
    for (y, &idx) in (area.y + 1..rows_bottom).zip(order.iter()) {
        draw_host_row(
            frame.buffer_mut(),
            y,
            inner_x,
            right_lim,
            &bc.results[idx],
            app.theme(),
        );
    }
    // More hosts than rows — replace the last visible row with an overflow marker.
    if order.len() > capacity && rows_bottom > area.y + 1 {
        put_clamped(
            frame.buffer_mut(),
            inner_x,
            rows_bottom - 1,
            "\u{2026}",
            app.theme().style(StyleRole::BroadcastDetail),
            (right_lim - inner_x) as usize,
        );
    }

    if let Some(frac) = settling_frac {
        let bar = Rect::new(inner_x, bottom.saturating_sub(1), right_lim - inner_x, 1);
        render_countdown_bar(frame, bar, frac, app.theme());
    }
}

// ── Zoomed full-body view (issue #18) ───────────────────────

/// Zoomed full-body view: a selectable failures-first host list on top, and the
/// selected host's stdout/stderr in a detail pane below. Scroll via
/// `zoom_window(app, len, visible)`.
pub fn render_broadcast_zoomed(frame: &mut Frame, area: Rect, app: &App) {
    let Some(bc) = app.broadcast.as_ref() else {
        return;
    };
    if area.width < 6 || area.height < 4 {
        return;
    }

    frame.render_widget(Clear, area);
    let (title, badge) = header_parts(bc);
    render_panel_box(
        frame.buffer_mut(),
        area,
        &title,
        BROADCAST_PANEL.with_badge(&badge),
        true,
        app.theme(),
    );

    let inner_x = area.x + 2;
    let right_lim = area.x + area.width - 1;
    let inner_w = (right_lim - inner_x) as usize;
    let bottom = area.y + area.height - 1;

    let order = failures_first(&bc.results);
    let len = order.len();

    // Split the inner body: ~55% for the list, the rest for the detail pane.
    let inner_h = bottom.saturating_sub(area.y + 1) as usize;
    let list_h = ((inner_h * 55) / 100).clamp(1, inner_h.max(1));
    let visible = list_h.max(1);

    let (first, sel) = zoom_window(app, len, visible);

    // ── Host list ───────────────────────────────────────────
    for (row, &idx) in order.iter().enumerate().skip(first).take(visible) {
        let y = area.y + 1 + (row - first) as u16;
        if y >= area.y + 1 + list_h as u16 || y >= bottom {
            break;
        }
        draw_host_row(
            frame.buffer_mut(),
            y,
            inner_x,
            right_lim,
            &bc.results[idx],
            app.theme(),
        );
        if row == sel {
            for col in inner_x..right_lim {
                if let Some(cell) = frame.buffer_mut().cell_mut((col, y)) {
                    cell.modifier.insert(Modifier::REVERSED);
                }
            }
        }
    }

    // ── Divider ─────────────────────────────────────────────
    let div_y = area.y + 1 + list_h as u16;
    if div_y < bottom {
        // The inner divider between the host list and the output pane. It has
        // its own rect so a gradient runs across the rule alone; the zoomed
        // broadcast body is never drawn over the remote PTY.
        let rule = Rect::new(inner_x, div_y, inner_w as u16, 1);
        let theme = app.theme();
        let sep: String = "\u{2500}".repeat(inner_w);
        frame.buffer_mut().set_string(
            inner_x,
            div_y,
            &sep,
            Style::default().fg(blit::line_color(theme, PaintRole::SeparatorSecondary, rule)),
        );
        blit::paint_line(
            frame.buffer_mut(),
            rule,
            theme,
            PaintRole::SeparatorSecondary,
        );
    }

    // ── Detail pane for the selected host ───────────────────
    if len == 0 {
        return;
    }
    let selected = &bc.results[order[sel.min(len - 1)]];
    let mut dy = div_y + 1;
    if dy >= bottom {
        return;
    }

    let head = format!("{} \u{2014} output", selected.host_name);
    put_clamped(
        frame.buffer_mut(),
        inner_x,
        dy,
        &head,
        app.theme().style(StyleRole::TextBright),
        inner_w,
    );
    dy += 1;

    let body_style = app.theme().style(StyleRole::TextPrimary);
    let push_block = |frame: &mut Frame, dy: &mut u16, tag: &str, body: &str, tag_style: Style| {
        if body.trim().is_empty() || *dy >= bottom {
            return;
        }
        put_clamped(frame.buffer_mut(), inner_x, *dy, tag, tag_style, inner_w);
        *dy += 1;
        for line in body.lines() {
            if *dy >= bottom {
                break;
            }
            put_clamped(
                frame.buffer_mut(),
                inner_x + 2,
                *dy,
                line,
                body_style,
                inner_w.saturating_sub(2),
            );
            *dy += 1;
        }
    };

    push_block(
        frame,
        &mut dy,
        "stdout:",
        &selected.stdout,
        app.theme().style(StyleRole::BroadcastStdout),
    );
    push_block(
        frame,
        &mut dy,
        "stderr:",
        &selected.stderr,
        app.theme().style(StyleRole::BroadcastStderr),
    );

    if selected.stdout.trim().is_empty() && selected.stderr.trim().is_empty() && dy < bottom {
        put_clamped(
            frame.buffer_mut(),
            inner_x,
            dy,
            "(no output)",
            app.theme().style(StyleRole::BroadcastDetail),
            inner_w,
        );
    }
}

// ── Countdown gauge ─────────────────────────────────────────

/// Thin countdown gauge along a 1-row `area`. `frac` in [0,1] = elapsed/DISMISS;
/// the filled portion depletes as the countdown runs out.
pub fn render_countdown_bar(frame: &mut Frame, area: Rect, frac: f32, theme: &ResolvedTheme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let frac = frac.clamp(0.0, 1.0);
    let remaining_secs = (DISMISS.as_secs_f32() * (1.0 - frac)).ceil() as u32;
    let label = format!(" dismiss {remaining_secs}s ");
    let label_w = label.chars().count() as u16;

    let total = area.width;
    let bar_w = total.saturating_sub(label_w);
    let filled = ((bar_w as f32) * (1.0 - frac)).round() as u16;

    let buf = frame.buffer_mut();
    let y = area.y;
    // Filled (remaining) portion in the countdown role, spent portion muted.
    // Heavy vs light rules, so the split reads without colour too.
    let left = theme.style(StyleRole::BroadcastCountdown);
    let spent = theme.style(StyleRole::TextDim);
    for i in 0..bar_w {
        let ch = if i < filled { "\u{2501}" } else { "\u{2500}" };
        buf.set_string(area.x + i, y, ch, if i < filled { left } else { spent });
    }
    if label_w > 0 && bar_w < total {
        buf.set_string(area.x + bar_w, y, &label, theme.style(StyleRole::TextMuted));
    }
}

// ── Docked / spawn geometry (shared with the orchestrator) ──

/// Resting docked rect: bottom-right corner of the dashboard body.
pub fn docked_rect(body: Rect) -> Rect {
    let w = 54u16.min(body.width.saturating_sub(2)).max(20);
    let h = 13u16.min(body.height.saturating_sub(2)).max(6);
    let x = body.x + body.width.saturating_sub(w).saturating_sub(1);
    let y = body.y + body.height.saturating_sub(h).saturating_sub(1);
    Rect::new(x, y, w, h)
}

/// Entry-slide start rect: centered over the body (same size as `docked_rect`).
pub fn spawn_rect(body: Rect) -> Rect {
    let d = docked_rect(body);
    let x = body.x + body.width.saturating_sub(d.width) / 2;
    let y = body.y + body.height.saturating_sub(d.height) / 2;
    Rect::new(x, y, d.width, d.height)
}

/// Max wrapped text lines a toast shows (older content is clipped by the box).
const MAX_TOAST_LINES: usize = 6;

/// Error toasts (issue #3): one popup per failed host, sliding in from the right
/// and out again after `TOAST_TTL`. They stack **up from just above the docked
/// panel** while it's on screen, and **down into the vacated bottom-right** once
/// the panel is gone. Each box is sized to wrap its full error text (capped).
pub fn render_broadcast_toasts(frame: &mut Frame, body: Rect, app: &App) {
    if app.broadcast_toasts.is_empty() {
        return;
    }
    let dock = docked_rect(body);
    let w = dock.width;
    let inner_w = w.saturating_sub(2) as usize; // inside the borders
    let target_x = dock.x;
    let off_right = body.x + body.width; // fully off the right edge
    let now = Instant::now();
    let motion = app.motion_enabled();

    // Anchor: stack grows upward from `stack_bottom`. With the panel present that
    // is just above it (dock.y); once it's gone, the toasts fall down into the
    // freed space (dock.y + dock.height). The transition animates over TOAST_ANIM
    // from the moment the panel was dismissed (skipped under reduced motion).
    let top_anchor = dock.y;
    let low_anchor = dock.y + dock.height;
    let anchor = if app.broadcast.is_some() {
        top_anchor
    } else if let (true, Some(gone)) = (motion, app.broadcast_panel_gone_at) {
        let t = now.saturating_duration_since(gone).as_secs_f32() / TOAST_ANIM.as_secs_f32();
        lerp_u16(top_anchor, low_anchor, ease_out(t.clamp(0.0, 1.0)))
    } else {
        low_anchor
    };
    let mut cur_bottom = anchor;

    for toast in app.broadcast_toasts.iter().rev() {
        let lines = wrap_line_count(&toast.text, inner_w).clamp(1, MAX_TOAST_LINES);
        let height = lines as u16 + 2; // borders
        if cur_bottom < body.y + height {
            break; // no room left above
        }
        let y = cur_bottom - height;
        cur_bottom = y; // the next (older) toast sits above this one

        // Slide progress from `born`: in for the first TOAST_ANIM, hold, then out
        // once past TOAST_TTL. No stored state — all derived from elapsed time.
        // Under reduced motion the toast just sits at rest and blinks out at TTL.
        let elapsed = now.saturating_duration_since(toast.born);
        let x = if !motion {
            target_x
        } else if elapsed >= TOAST_TTL {
            let t = (elapsed - TOAST_TTL).as_secs_f32() / TOAST_ANIM.as_secs_f32();
            lerp_u16(target_x, off_right, ease_out(t.clamp(0.0, 1.0)))
        } else {
            let t = elapsed.as_secs_f32() / TOAST_ANIM.as_secs_f32();
            lerp_u16(off_right, target_x, ease_out(t.clamp(0.0, 1.0)))
        };
        if x >= off_right {
            continue; // fully off-screen this frame
        }
        let vis_w = w.min(off_right - x);
        if vis_w < 6 {
            continue;
        }
        let rect = Rect::new(x, y, vis_w, height);

        crate::tui::open_popup(frame, rect, app.theme());
        let title =
            crate::tui::text::ellipsize(&format!(" \u{2717} {} ", toast.host), vis_w as usize);
        // A failure toast is the broadcast error state, not generic popup
        // chrome: it is the same red as the row that produced it.
        let failed = Style::default().fg(app.theme().color(ColorRole::BroadcastError));
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(failed)
            .title(Span::styled(title, failed));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let para = Paragraph::new(toast.text.as_str())
            .style(app.theme().style(StyleRole::BroadcastDetail))
            .wrap(Wrap { trim: true });
        frame.render_widget(para, inner);
    }
}

/// Round a horizontal lerp between two columns.
fn lerp_u16(a: u16, b: u16, t: f32) -> u16 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u16
}

/// Greedy word-wrap line count for `text` at `width` (matches `Wrap{trim}` well
/// enough to size a toast box). Blank input still counts as one line.
fn wrap_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    let mut lines = 0usize;
    for para in text.split('\n') {
        let mut col = 0usize;
        for word in para.split_whitespace() {
            let wl = word.chars().count();
            if col == 0 {
                col = wl;
            } else if col + 1 + wl <= width {
                col += 1 + wl;
            } else {
                lines += 1;
                col = wl;
            }
        }
        lines += 1; // the paragraph's final (or only/empty) line
    }
    lines.max(1)
}

// ── Pre-run overlay stages ──────────────────────────────────

/// Centered popup rect helper, clamped to the frame.
fn popup_rect(frame: &Frame, w_pct: u16, min_w: u16, h: u16) -> Rect {
    let area = frame.area();
    // The percentage is computed in `u32`: `area.width * w_pct` overflows a
    // `u16` on a wide terminal long before the division brings it back down.
    // `fit_popup` then keeps `min_w` subordinate to the width actually there.
    let desired_w =
        (u32::from(area.width) * u32::from(w_pct) / 100).min(u32::from(u16::MAX)) as u16;
    let w = crate::tui::fit_popup(desired_w, min_w, area.width);
    let h = crate::tui::fit_popup(h, 1, area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

/// "Broadcast to:" menu — pick a group or tag as the target set.
pub fn render_pick_target(frame: &mut Frame, app: &App) {
    let Some(setup) = app.broadcast_setup.as_ref() else {
        return;
    };

    let list_rows = setup.options.len().clamp(1, 12) as u16;
    let popup = popup_rect(frame, 60, 40, list_rows + 4);

    let popup = crate::tui::popup_open_rect(popup, app);

    let theme = app.theme();
    crate::tui::open_popup(frame, popup, theme);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " Broadcast to ",
                theme.style(StyleRole::PopupTitle),
            ))
            .border_style(crate::tui::popup_border_style(theme, popup)),
        popup,
    );
    crate::tui::paint_popup_border(frame, popup, theme);

    // Everything below writes into the buffer directly. `set_string` clips
    // columns on its own, but an out-of-range *row* panics — and the popup
    // rect being legal says nothing about the rows inside it.
    if popup.width < 4 || popup.height < 3 {
        return;
    }

    let row_x = popup.x + 2;
    let inner_w = popup.width.saturating_sub(4) as usize;
    let list_top = popup.y + 1;
    let visible = popup.height.saturating_sub(3) as usize;

    let buf = frame.buffer_mut();
    if setup.options.is_empty() {
        buf.set_string(
            row_x,
            list_top,
            "(no groups or tags)",
            theme.style(StyleRole::PopupLegend),
        );
    } else {
        let scroll = setup
            .menu_selected
            .saturating_sub(visible.saturating_sub(1));
        for (i, opt) in setup.options.iter().skip(scroll).take(visible).enumerate() {
            let idx = scroll + i;
            let ry = list_top + i as u16;
            let is_sel = idx == setup.menu_selected;
            let text = match opt {
                crate::app::BroadcastTarget::Group { label, .. } => {
                    format!("group: {label}")
                }
                crate::app::BroadcastTarget::Tag { name } => format!("#{name}"),
            };
            let style = theme.style(if is_sel {
                StyleRole::PickerRowSelected
            } else {
                StyleRole::PickerRow
            });
            if is_sel {
                let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
                buf.set_string(popup.x + 1, ry, &blank, style);
            }
            let marker = if is_sel { "\u{203a} " } else { "  " };
            buf.set_string(
                row_x,
                ry,
                crate::tui::text::ellipsize(&format!("{marker}{text}"), inner_w),
                style,
            );
        }
    }

    let hint_y = popup.y + popup.height.saturating_sub(2);
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize(
            "\u{2191}/\u{2193} select \u{00b7} Enter \u{00b7} Esc cancel",
            inner_w,
        ),
        theme.style(StyleRole::PopupLegend),
    );
}

/// "cmd>" single-line command prompt for the chosen target.
pub fn render_command_prompt(frame: &mut Frame, app: &App) {
    let Some(setup) = app.broadcast_setup.as_ref() else {
        return;
    };

    let popup = popup_rect(frame, 70, 44, 7);
    let theme = app.theme();

    let lines = vec![
        Line::from(Span::styled(
            format!("Command to run on {}:", setup.target_label),
            theme.style(StyleRole::TextPrimary),
        )),
        Line::from(Span::styled(
            format!(
                "cmd> {}",
                crate::text_input::with_cursor(&setup.command, setup.cursor)
            ),
            theme.style(StyleRole::FormInput),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Enter: preview  \u{2502}  Esc: cancel",
            theme.style(StyleRole::PopupHint),
        )),
    ];

    let popup = crate::tui::popup_open_rect(popup, app);

    crate::tui::open_popup(frame, popup, theme);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " Broadcast command ",
                    theme.style(StyleRole::PopupTitle),
                ))
                .border_style(crate::tui::popup_border_style(theme, popup)),
        ),
        popup,
    );
    crate::tui::paint_popup_border(frame, popup, theme);
}

/// Dry-run preview: the resolved target list + command, with the
/// `[y]` / `[e]` / `[N]` barrier. `[e]` toggles per-host deselect.
pub fn render_preview(frame: &mut Frame, app: &App) {
    let Some(setup) = app.broadcast_setup.as_ref() else {
        return;
    };

    let selected_count = setup.candidates.iter().filter(|c| c.selected).count();
    let list_rows = setup.candidates.len().clamp(1, 14) as u16;
    let popup = popup_rect(frame, 74, 50, list_rows + 6);

    let popup = crate::tui::popup_open_rect(popup, app);

    let theme = app.theme();
    crate::tui::open_popup(frame, popup, theme);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " Broadcast preview ",
                theme.style(StyleRole::PopupTitle),
            ))
            .border_style(crate::tui::popup_border_style(theme, popup)),
        popup,
    );
    crate::tui::paint_popup_border(frame, popup, theme);

    // As in `render_pick_target`: the rows are this renderer's own problem.
    // Five rows is the first height where the summary, one list row and the
    // barrier hint all land inside the border.
    if popup.width < 4 || popup.height < 5 {
        return;
    }

    let row_x = popup.x + 2;
    let inner_w = popup.width.saturating_sub(4) as usize;
    let mut y = popup.y + 1;
    // `popup.y + popup.height - 1` underflowed on a zero-height popup; the
    // bottom row is `bottom()` minus the border, saturating throughout.
    let bottom = popup.bottom().saturating_sub(1);

    // Summary line.
    let summary = format!(
        "Run `{}` on {} host{} ({}):",
        setup.command,
        selected_count,
        if selected_count == 1 { "" } else { "s" },
        setup.target_label,
    );
    {
        let buf = frame.buffer_mut();
        buf.set_string(
            row_x,
            y,
            crate::tui::text::ellipsize(&summary, inner_w),
            theme.style(StyleRole::TextPrimary),
        );
    }
    y += 2;

    // Target list. In edit mode, show checkboxes + highlight the cursor row.
    let list_bottom = bottom.saturating_sub(2);
    let visible = list_bottom.saturating_sub(y) as usize;
    let scroll = if setup.edit_targets {
        setup
            .preview_selected
            .saturating_sub(visible.saturating_sub(1))
    } else {
        0
    };

    let buf = frame.buffer_mut();
    if setup.candidates.is_empty() {
        buf.set_string(
            row_x,
            y,
            "(no managed hosts in target)",
            theme.style(StyleRole::PopupLegend),
        );
    } else {
        for (i, cand) in setup
            .candidates
            .iter()
            .skip(scroll)
            .take(visible)
            .enumerate()
        {
            let idx = scroll + i;
            let ry = y + i as u16;
            if ry >= list_bottom {
                break;
            }
            let is_cursor = setup.edit_targets && idx == setup.preview_selected;
            let checkbox = if setup.edit_targets {
                if cand.selected {
                    "[\u{2713}] "
                } else {
                    "[ ] "
                }
            } else {
                "\u{00b7} "
            };
            // A deselected host stays dim even under the cursor bar, so the
            // `[ ]` rows read as excluded rather than merely unfocused.
            let name_style = theme.style(if !cand.selected {
                StyleRole::TextDim
            } else if is_cursor {
                StyleRole::PickerRowSelected
            } else {
                StyleRole::PickerRow
            });
            if is_cursor {
                let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
                buf.set_string(
                    popup.x + 1,
                    ry,
                    &blank,
                    theme.style(StyleRole::PickerRowSelected),
                );
            }
            buf.set_string(
                row_x,
                ry,
                crate::tui::text::ellipsize(&format!("{checkbox}{}", cand.host_name), inner_w),
                name_style,
            );
        }
    }

    // Barrier hint.
    let hint = if setup.edit_targets {
        "\u{2191}/\u{2193} move \u{00b7} Space toggle \u{00b7} Enter done \u{00b7} Esc cancel"
    } else {
        "[y] confirm   [e] edit targets   [c] edit command   [N] cancel"
    };
    let hint_y = bottom.saturating_sub(1);
    frame.buffer_mut().set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize(hint, inner_w),
        theme.style(StyleRole::PopupLegend),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        assert_panel_wears, frame_at, panel_marker_theme, themed_app, PanelFamily, PanelProof,
    };

    /// A live run with one pending host — enough for both panels to draw.
    fn broadcast_state() -> crate::app::BroadcastState {
        use crate::app::BroadcastPhase;
        use crate::broadcast::BroadcastTask;

        let tasks = vec![BroadcastTask {
            host_id: 1,
            host_name: "web-prod".into(),
            argv: vec!["ssh".into(), "web-prod".into()],
            secret: None,
        }];
        let (_tx, rx) = std::sync::mpsc::channel();
        crate::app::BroadcastState {
            target_label: "group: prod".into(),
            command: "uptime".into(),
            results: crate::broadcast::seed_results(&tasks),
            rx,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            concurrency: 2,
            phase: BroadcastPhase::Running,
            anim: None,
            audit_written: false,
        }
    }

    /// The docked panel wears `broadcast.panel` in **both** focus states.
    ///
    /// `render_broadcast_panel` takes `focused` from its caller, so the
    /// unfocused branch is just as productive as the focused one and needs its
    /// own proof.
    #[test]
    fn the_docked_broadcast_panel_wears_its_roles_in_both_focus_states() {
        let mut app = themed_app(panel_marker_theme());
        app.broadcast = Some(broadcast_state());
        let area = Rect::new(0, 0, 60, 12);

        for focused in [false, true] {
            let buf = frame_at(area, |frame| {
                render_broadcast_panel(frame, area, &app, focused);
            });
            assert_panel_wears(
                &buf,
                area,
                PanelProof {
                    family: PanelFamily::Broadcast,
                    focused,
                    title: "cast",
                    count: Some("0/1"),
                    body: (2, area.height - 2),
                },
            );
        }
    }

    /// The zoomed view is the second call site of the same bundle. It always
    /// draws focused — it only exists while the panel is zoomed *into* — so one
    /// state is the whole productive surface here.
    #[test]
    fn the_zoomed_broadcast_view_wears_the_same_bundle() {
        let mut app = themed_app(panel_marker_theme());
        app.broadcast = Some(broadcast_state());
        let area = Rect::new(0, 0, 60, 12);

        let buf = frame_at(area, |frame| render_broadcast_zoomed(frame, area, &app));
        assert_panel_wears(
            &buf,
            area,
            PanelProof {
                family: PanelFamily::Broadcast,
                focused: true,
                title: "cast",
                count: Some("0/1"),
                body: (2, area.height - 2),
            },
        );
    }

    // ── Role coverage ────────────────────────────────────────

    use crate::broadcast::{HostResult, HostState};
    use crate::test_support::{
        fg, fg_at_text, fg_at_text_from, fg_bg, marker, resolved_default, role_marker_theme,
        RoleMarker,
    };
    use crate::theme::model::ResolvedTheme;

    const PENDING: u32 = 0xa4_0001;
    const RUNNING: u32 = 0xa4_0002;
    const SUCCESS: u32 = 0xa4_0003;
    const ERROR: u32 = 0xa4_0004;
    const STDOUT: u32 = 0xa4_0005;
    const STDERR: u32 = 0xa4_0006;
    const DETAIL: u32 = 0xa4_0007;
    const COUNTDOWN: u32 = 0xa4_0008;
    const RULE: u32 = 0xa4_0009;
    const TEXT: u32 = 0xa4_000a;
    const BRIGHT: u32 = 0xa4_000b;
    const DIM: u32 = 0xa4_000c;
    const MUTED: u32 = 0xa4_000d;
    const POPUP_TITLE: u32 = 0xa4_000e;
    const POPUP_LEGEND: u32 = 0xa4_000f;
    const POPUP_HINT: u32 = 0xa4_0010;
    const PICKER_ROW: u32 = 0xa4_0011;
    const PICKER_SEL: u32 = 0xa4_0012;
    const PICKER_SEL_BG: u32 = 0xa4_0112;
    const FORM_INPUT: u32 = 0xa4_0013;

    const MARKERS: &[RoleMarker] = &[
        fg("components.broadcast.pending", PENDING),
        fg("components.broadcast.running", RUNNING),
        fg("components.broadcast.success", SUCCESS),
        fg("components.broadcast.error", ERROR),
        fg("components.broadcast.stdout", STDOUT),
        fg("components.broadcast.stderr", STDERR),
        fg("components.broadcast.detail", DETAIL),
        fg("components.broadcast.countdown", COUNTDOWN),
        fg("components.separator.secondary", RULE),
        fg("components.text.primary", TEXT),
        fg("components.text.bright", BRIGHT),
        fg("components.text.dim", DIM),
        fg("components.text.muted", MUTED),
        fg("components.popup.title", POPUP_TITLE),
        fg("components.popup.legend", POPUP_LEGEND),
        fg("components.popup.hint", POPUP_HINT),
        fg("components.picker.row", PICKER_ROW),
        fg_bg("components.picker.row_selected", PICKER_SEL, PICKER_SEL_BG),
        fg("components.form.input", FORM_INPUT),
    ];

    fn marked() -> ResolvedTheme {
        role_marker_theme("broadcast", MARKERS)
    }

    fn result(name: &str, state: HostState) -> HostResult {
        HostResult {
            host_id: 1,
            host_name: name.into(),
            state,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    /// A run holding one host in each of the four states.
    fn four_states() -> crate::app::BroadcastState {
        let mut bc = broadcast_state();
        bc.results = vec![
            result("pend-host", HostState::Pending),
            result("run-host", HostState::Running),
            result("ok-host", HostState::Done { exit: 0 }),
            result(
                "bad-host",
                HostState::Failed {
                    reason: "no route".into(),
                },
            ),
        ];
        bc
    }

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 60,
        height: 12,
    };

    /// All four host states, glyph and word, through the docked panel.
    #[test]
    fn every_broadcast_state_wears_its_own_colour() {
        let mut app = themed_app(marked());
        app.broadcast = Some(four_states());
        let buf = frame_at(AREA, |f| render_broadcast_panel(f, AREA, &app, true));

        for (host, word, colour) in [
            ("pend-host", "pending", PENDING),
            ("run-host", "running", RUNNING),
            ("ok-host", "exit 0", SUCCESS),
            ("bad-host", "failed", ERROR),
        ] {
            assert_eq!(
                fg_at_text_from(&buf, host, AREA.y + 1),
                marker(TEXT),
                "{host}: the host name is body text"
            );
            assert_eq!(
                fg_at_text_from(&buf, word, AREA.y + 1),
                marker(colour),
                "{host}: the status word"
            );
        }
        // The failure reason trails the row in its own role.
        assert_eq!(
            fg_at_text_from(&buf, "no route", AREA.y + 1),
            marker(DETAIL),
            "the failure detail"
        );
    }

    /// The zoomed view's divider and its detail head.
    #[test]
    fn the_zoomed_broadcast_view_wears_its_output_roles() {
        let mut app = themed_app(marked());
        let mut bc = four_states();
        bc.results[0].stdout = "all good\n".into();
        bc.results[0].stderr = "went wrong\n".into();
        app.broadcast = Some(bc);
        let area = Rect::new(0, 0, 60, 20);
        let buf = frame_at(area, |f| render_broadcast_zoomed(f, area, &app));

        assert_eq!(
            // From below the panel's own top border, which is the same glyph.
            fg_at_text_from(&buf, "\u{2500}\u{2500}\u{2500}", area.y + 1),
            marker(RULE),
            "the inner divider between the list and the output"
        );
        // `failures_first` puts the failed host at the top, so it is selected.
        assert_eq!(
            fg_at_text_from(&buf, "\u{2014} output", area.y + 1),
            marker(BRIGHT),
            "the detail head"
        );
    }

    /// stdout and stderr must stay distinct even when a host produced both.
    #[test]
    fn broadcast_stdout_and_stderr_never_share_a_role() {
        let mut app = themed_app(marked());
        let mut bc = broadcast_state();
        bc.results = vec![{
            let mut r = result("web-prod", HostState::Done { exit: 1 });
            r.stdout = "all good\n".into();
            r.stderr = "went wrong\n".into();
            r
        }];
        app.broadcast = Some(bc);
        let area = Rect::new(0, 0, 60, 20);
        let buf = frame_at(area, |f| render_broadcast_zoomed(f, area, &app));

        assert_eq!(fg_at_text(&buf, "stdout:"), marker(STDOUT));
        assert_eq!(fg_at_text(&buf, "stderr:"), marker(STDERR));
        assert_eq!(
            fg_at_text(&buf, "all good"),
            marker(TEXT),
            "the output body is plain text"
        );
    }

    #[test]
    fn a_silent_host_reports_no_output_in_the_detail_role() {
        let mut app = themed_app(marked());
        app.broadcast = Some(four_states());
        let area = Rect::new(0, 0, 60, 20);
        let buf = frame_at(area, |f| render_broadcast_zoomed(f, area, &app));
        assert_eq!(fg_at_text(&buf, "(no output)"), marker(DETAIL));
    }

    #[test]
    fn the_countdown_gauge_wears_its_two_roles() {
        let theme = marked();
        let area = Rect::new(0, 0, 40, 1);
        let buf = frame_at(area, |f| render_countdown_bar(f, area, 0.5, &theme));

        assert_eq!(
            fg_at_text(&buf, "\u{2501}"),
            marker(COUNTDOWN),
            "the remaining portion"
        );
        assert_eq!(
            fg_at_text(&buf, "\u{2500}"),
            marker(DIM),
            "the spent portion"
        );
        assert_eq!(fg_at_text(&buf, "dismiss"), marker(MUTED), "the label");
    }

    /// A failure toast is the broadcast error colour, not generic popup chrome.
    #[test]
    fn a_broadcast_toast_wears_the_error_state_colour() {
        let mut app = themed_app(marked());
        app.broadcast_toasts = vec![crate::app::BroadcastToast {
            host: "bad-host".into(),
            text: "no route to host".into(),
            // At rest: a toast born this instant is still off the right edge.
            born: Instant::now()
                .checked_sub(TOAST_ANIM)
                .expect("the test clock is past the epoch"),
        }];
        let body = Rect::new(0, 0, 80, 24);
        let buf = frame_at(body, |f| render_broadcast_toasts(f, body, &app));

        assert_eq!(fg_at_text(&buf, "bad-host"), marker(ERROR), "the title");
        assert_eq!(
            fg_at_text(&buf, "no route"),
            marker(DETAIL),
            "the toast body"
        );
    }

    fn setup(edit_targets: bool) -> crate::app::BroadcastSetup {
        use crate::app::{BroadcastCandidate, BroadcastTarget};
        crate::app::BroadcastSetup {
            options: vec![
                BroadcastTarget::Group {
                    id: 1,
                    label: "prod".into(),
                },
                BroadcastTarget::Tag {
                    name: "edge".into(),
                },
            ],
            menu_selected: 0,
            target_label: "group: prod".into(),
            command: "uptime".into(),
            cursor: 6,
            candidates: vec![
                BroadcastCandidate {
                    host_id: 1,
                    host_name: "keeper".into(),
                    argv: vec![],
                    secret: None,
                    selected: true,
                },
                BroadcastCandidate {
                    host_id: 2,
                    host_name: "dropped".into(),
                    argv: vec![],
                    secret: None,
                    selected: false,
                },
            ],
            preview_selected: 0,
            edit_targets,
        }
    }

    /// The three pre-run popups wear the overlay roles Task 14 established,
    /// each in both of its row states.
    #[test]
    fn the_broadcast_wizard_popups_wear_the_overlay_roles() {
        let mut app = themed_app(marked());
        app.broadcast_setup = Some(setup(false));
        let area = Rect::new(0, 0, 80, 24);

        let buf = frame_at(area, |f| render_pick_target(f, &app));
        assert_eq!(fg_at_text(&buf, "Broadcast to"), marker(POPUP_TITLE));
        assert_eq!(
            fg_at_text(&buf, "group: prod"),
            marker(PICKER_SEL),
            "the selected target row"
        );
        assert_eq!(
            fg_at_text(&buf, "#edge"),
            marker(PICKER_ROW),
            "an unselected target row"
        );
        assert_eq!(fg_at_text(&buf, "Enter"), marker(POPUP_LEGEND), "the hint");

        let buf = frame_at(area, |f| render_command_prompt(f, &app));
        assert_eq!(fg_at_text(&buf, "Broadcast command"), marker(POPUP_TITLE));
        assert_eq!(fg_at_text(&buf, "Command to run"), marker(TEXT));
        assert_eq!(fg_at_text(&buf, "cmd>"), marker(FORM_INPUT));
        assert_eq!(fg_at_text(&buf, "Enter: preview"), marker(POPUP_HINT));

        app.broadcast_setup = Some(setup(true));
        let buf = frame_at(area, |f| render_preview(f, &app));
        assert_eq!(fg_at_text(&buf, "Broadcast preview"), marker(POPUP_TITLE));
        assert_eq!(fg_at_text(&buf, "Run `uptime`"), marker(TEXT));
        assert_eq!(
            fg_at_text(&buf, "keeper"),
            marker(PICKER_SEL),
            "the cursor row"
        );
        assert_eq!(
            fg_at_text(&buf, "dropped"),
            marker(DIM),
            "a deselected host stays dim under the cursor bar"
        );
        assert_eq!(fg_at_text(&buf, "Space toggle"), marker(POPUP_LEGEND));
    }

    /// Every popup renderer has to survive a terminal too small to hold it.
    /// The first four sizes are the matrix the maintainer specified; how much
    /// of the layout each one leaves room for differs per renderer. `20x1` and
    /// `40x2` are the sizes that actually reproduced `render_preview`'s panic:
    /// below `10` columns the inner width collapses to an empty string and
    /// `set_string` never indexes the out-of-range row.
    const TINY: &[(u16, u16)] = &[(1, 1), (3, 2), (8, 4), (20, 6), (20, 1), (40, 2)];

    /// The three pre-run popups must clip rather than panic on a tiny terminal.
    #[test]
    fn the_broadcast_wizard_popups_survive_a_tiny_terminal() {
        /// One wizard stage: its label and its real renderer.
        type Stage = (&'static str, fn(&mut Frame, &App));
        let renderers: &[Stage] = &[
            ("pick target", render_pick_target),
            ("command prompt", render_command_prompt),
            ("preview", render_preview),
        ];
        for edit_targets in [false, true] {
            let mut app = themed_app(resolved_default());
            app.broadcast_setup = Some(setup(edit_targets));
            for (name, render) in renderers {
                for (w, h) in TINY {
                    let area = Rect::new(0, 0, *w, *h);
                    // Reaching the assert at all is the proof: `frame_at` drives
                    // the real renderer, and an out-of-range row would have
                    // panicked before it returned a buffer.
                    let buf = frame_at(area, |frame| render(frame, &app));
                    assert_eq!(buf.area, area, "{name} at {w}x{h} drew outside the frame");
                }
            }
        }
    }

    /// Legacy parity, hand-transcribed from the `crate::tui::theme` calls this
    /// screen made before the migration.
    #[test]
    fn the_broadcast_surfaces_reproduce_their_legacy_cells_under_default() {
        use crate::tui::theme::legacy;
        let theme = resolved_default();

        let mut app = themed_app(resolved_default());
        app.broadcast = Some(four_states());
        let buf = frame_at(AREA, |f| render_broadcast_panel(f, AREA, &app, true));
        for (word, expected) in [
            ("pending", legacy::MUTE),
            ("running", legacy::AMBER),
            ("exit 0", legacy::GREEN),
            ("failed", legacy::RED),
        ] {
            assert_eq!(
                fg_at_text_from(&buf, word, AREA.y + 1),
                expected,
                "the `{word}` row"
            );
        }
        assert_eq!(
            fg_at_text_from(&buf, "no route", AREA.y + 1),
            legacy::DIM,
            "the failure detail was theme::dim()"
        );
        assert_eq!(fg_at_text_from(&buf, "pend-host", AREA.y + 1), legacy::TEXT);

        let area = Rect::new(0, 0, 60, 20);
        let mut app = themed_app(resolved_default());
        let mut bc = broadcast_state();
        bc.results = vec![{
            let mut r = result("web-prod", HostState::Done { exit: 1 });
            r.stdout = "all good\n".into();
            r.stderr = "went wrong\n".into();
            r
        }];
        app.broadcast = Some(bc);
        let buf = frame_at(area, |f| render_broadcast_zoomed(f, area, &app));
        assert_eq!(fg_at_text(&buf, "stdout:"), legacy::MUTE);
        assert_eq!(fg_at_text(&buf, "stderr:"), legacy::RED);
        assert_eq!(fg_at_text(&buf, "all good"), legacy::TEXT);
        assert_eq!(
            fg_at_text_from(&buf, "\u{2500}\u{2500}\u{2500}", area.y + 1),
            legacy::DIM,
            "the divider was theme::dim()"
        );
        assert_eq!(
            fg_at_text_from(&buf, "\u{2014} output", area.y + 1),
            legacy::BRIGHT
        );

        let bar = Rect::new(0, 0, 40, 1);
        let buf = frame_at(bar, |f| render_countdown_bar(f, bar, 0.5, &theme));
        assert_eq!(fg_at_text(&buf, "\u{2501}"), legacy::CYAN);
        assert_eq!(fg_at_text(&buf, "\u{2500}"), legacy::DIM);
        assert_eq!(fg_at_text(&buf, "dismiss"), legacy::MUTE);

        let mut app = themed_app(resolved_default());
        app.broadcast_setup = Some(setup(true));
        let popup = Rect::new(0, 0, 80, 24);
        let buf = frame_at(popup, |f| render_preview(f, &app));
        let title = crate::test_support::find_text(&buf, "Broadcast preview");
        assert_eq!(buf.cell(title).unwrap().fg, legacy::BRIGHT);
        assert!(
            buf.cell(title).unwrap().modifier.contains(Modifier::BOLD),
            "the popup title kept `theme::heading()`'s weight"
        );
        assert_eq!(fg_at_text(&buf, "keeper"), legacy::SEL_FG);
        assert_eq!(fg_at_text(&buf, "dropped"), legacy::DIM);
        assert_eq!(fg_at_text(&buf, "Space toggle"), legacy::MUTE);
    }
}
