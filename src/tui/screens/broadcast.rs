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
use crate::tui::theme;
use crate::tui::tween::ease_out;
use crate::tui::widgets::panel_box::{put_clamped, render_panel_box, zoom_window};

// ── Per-host row presentation ───────────────────────────────

/// Glyph + style + short status label for one host state. Single-cell glyphs
/// keep the row columns aligned across terminals (no emoji double-width).
fn state_row(state: &HostState) -> (&'static str, Style, String) {
    match state {
        HostState::Pending => ("\u{25cb}", theme::mute(), "pending".to_string()),
        HostState::Running => ("\u{25cf}", theme::amber(), "running".to_string()),
        HostState::Done { exit: 0 } => ("\u{2713}", theme::green(), "exit 0".to_string()),
        HostState::Done { exit } => ("\u{2717}", theme::red(), format!("exit {exit}")),
        HostState::Failed { .. } => ("\u{2717}", theme::red(), "failed".to_string()),
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
) {
    if inner_x >= right_lim {
        return;
    }
    let inner_w = (right_lim - inner_x) as usize;
    let (glyph, gstyle, label) = state_row(&result.state);

    // Glyph column (1 cell + space).
    buf.set_string(inner_x, y, glyph, gstyle);
    let mut col = inner_x + 2;
    if col >= right_lim {
        return;
    }

    // Host name — left column, roughly a third of the width, min 8.
    let name_w = (inner_w / 3).clamp(8, 22).min((right_lim - col) as usize);
    col += put_clamped(buf, col, y, &result.host_name, theme::text(), name_w);
    // Pad the name column so the status labels line up.
    while col < inner_x + 2 + name_w as u16 && col < right_lim {
        buf.set_string(col, y, " ", theme::text());
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
            theme::dim(),
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
    render_panel_box(frame.buffer_mut(), area, &title, Some(&badge), focused);

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
        draw_host_row(frame.buffer_mut(), y, inner_x, right_lim, &bc.results[idx]);
    }
    // More hosts than rows — replace the last visible row with an overflow marker.
    if order.len() > capacity && rows_bottom > area.y + 1 {
        put_clamped(
            frame.buffer_mut(),
            inner_x,
            rows_bottom - 1,
            "\u{2026}",
            theme::dim(),
            (right_lim - inner_x) as usize,
        );
    }

    if let Some(frac) = settling_frac {
        let bar = Rect::new(inner_x, bottom.saturating_sub(1), right_lim - inner_x, 1);
        render_countdown_bar(frame, bar, frac);
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
    render_panel_box(frame.buffer_mut(), area, &title, Some(&badge), true);

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
        draw_host_row(frame.buffer_mut(), y, inner_x, right_lim, &bc.results[idx]);
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
        let sep: String = "\u{2500}".repeat(inner_w);
        frame
            .buffer_mut()
            .set_string(inner_x, div_y, &sep, theme::dim());
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
        theme::bright(),
        inner_w,
    );
    dy += 1;

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
                theme::text(),
                inner_w.saturating_sub(2),
            );
            *dy += 1;
        }
    };

    push_block(frame, &mut dy, "stdout:", &selected.stdout, theme::mute());
    push_block(frame, &mut dy, "stderr:", &selected.stderr, theme::red());

    if selected.stdout.trim().is_empty() && selected.stderr.trim().is_empty() && dy < bottom {
        put_clamped(
            frame.buffer_mut(),
            inner_x,
            dy,
            "(no output)",
            theme::dim(),
            inner_w,
        );
    }
}

// ── Countdown gauge ─────────────────────────────────────────

/// Thin countdown gauge along a 1-row `area`. `frac` in [0,1] = elapsed/DISMISS;
/// the filled portion depletes as the countdown runs out.
pub fn render_countdown_bar(frame: &mut Frame, area: Rect, frac: f32) {
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
    // Filled (remaining) portion in cyan, spent portion in dim.
    for i in 0..bar_w {
        let ch = if i < filled { "\u{2501}" } else { "\u{2500}" };
        let style = if i < filled {
            theme::cyan()
        } else {
            theme::dim()
        };
        buf.set_string(area.x + i, y, ch, style);
    }
    if label_w > 0 && bar_w < total {
        buf.set_string(area.x + bar_w, y, &label, theme::mute());
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

        frame.render_widget(Clear, rect);
        let title =
            crate::tui::text::ellipsize(&format!(" \u{2717} {} ", toast.host), vis_w as usize);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::red())
            .title(Span::styled(title, theme::red()));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let para = Paragraph::new(toast.text.as_str())
            .style(theme::dim())
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
    let w = (area.width * w_pct / 100).max(min_w).min(area.width);
    let h = h.min(area.height);
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

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Broadcast to ", theme::heading()))
            .border_style(theme::popup_border()),
        popup,
    );

    let row_x = popup.x + 2;
    let inner_w = popup.width.saturating_sub(4) as usize;
    let list_top = popup.y + 1;
    let visible = popup.height.saturating_sub(3) as usize;

    let buf = frame.buffer_mut();
    if setup.options.is_empty() {
        buf.set_string(row_x, list_top, "(no groups or tags)", theme::mute());
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
            if is_sel {
                let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
                buf.set_string(popup.x + 1, ry, &blank, theme::selected());
            }
            let marker = if is_sel { "\u{203a} " } else { "  " };
            let style = if is_sel {
                theme::selected()
            } else {
                theme::text()
            };
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
        theme::mute(),
    );
}

/// "cmd>" single-line command prompt for the chosen target.
pub fn render_command_prompt(frame: &mut Frame, app: &App) {
    let Some(setup) = app.broadcast_setup.as_ref() else {
        return;
    };

    let popup = popup_rect(frame, 70, 44, 7);

    let lines = vec![
        Line::from(Span::styled(
            format!("Command to run on {}:", setup.target_label),
            theme::text(),
        )),
        Line::from(Span::styled(
            format!(
                "cmd> {}",
                crate::text_input::with_cursor(&setup.command, setup.cursor)
            ),
            theme::bright(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Enter: preview  \u{2502}  Esc: cancel",
            theme::dim(),
        )),
    ];

    let popup = crate::tui::popup_open_rect(popup, app);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(" Broadcast command ", theme::heading()))
                .border_style(theme::popup_border()),
        ),
        popup,
    );
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

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Broadcast preview ", theme::heading()))
            .border_style(theme::popup_border()),
        popup,
    );

    let row_x = popup.x + 2;
    let inner_w = popup.width.saturating_sub(4) as usize;
    let mut y = popup.y + 1;
    let bottom = popup.y + popup.height - 1;

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
            theme::text(),
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
        buf.set_string(row_x, y, "(no managed hosts in target)", theme::mute());
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
            let name_style = if !cand.selected {
                theme::dim()
            } else if is_cursor {
                theme::selected()
            } else {
                theme::text()
            };
            if is_cursor {
                let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
                buf.set_string(popup.x + 1, ry, &blank, theme::selected());
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
        theme::mute(),
    );
}
