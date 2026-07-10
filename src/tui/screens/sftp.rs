//! SFTP tab body renderer.
//!
//! Two sub-states, mirroring `app.sftp`:
//! - `None` → **picker**: reuse the grouped hosts panel + a "connect" hint.
//! - `Some(state)` → **browser**: two bordered columns (left local / right
//!   remote), a queue strip, and a progress line while a run is in flight.
//!
//! Signature matches the other screens (`render_<name>(frame, area, app)`);
//! `tui/mod.rs` wires it through a local `render_sftp_body` wrapper.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::*;

use crate::app::App;
use crate::sftp::model::{Direction, Focus, Pane, Phase, SftpState};
use crate::tui::text::ellipsize;
use crate::tui::theme;
use crate::tui::widgets::panel_box::render_panel_box;

pub fn render_sftp(frame: &mut Frame, area: Rect, app: &App) {
    if area.height < 3 || area.width < 8 {
        return;
    }
    match app.sftp.as_ref() {
        None => render_picker(frame, area, app),
        Some(state) => render_browser(frame, area, state),
    }
}

// ── Picker sub-state ─────────────────────────────────────────

fn render_picker(frame: &mut Frame, area: Rect, app: &App) {
    let list_h = area.height.saturating_sub(1);
    let list_area = Rect::new(area.x, area.y, area.width, list_h);
    // Reuse the dashboard hosts panel so the picker shows the full grouped tree
    // with collapse arrows (▸/▾) and scrolling, identical to the hosts tab.
    crate::tui::widgets::hosts_panel::render_hosts_panel(frame, list_area, app);

    let hint_y = area.y + area.height.saturating_sub(1);
    frame.buffer_mut().set_string(
        area.x + 2,
        hint_y,
        "Enter connect (on a group: fold) · Esc back",
        theme::dim(),
    );
}

// ── Browser sub-state ────────────────────────────────────────

fn render_browser(frame: &mut Frame, area: Rect, state: &SftpState) {
    let progress_h: u16 = if state.phase == Phase::Running { 1 } else { 0 };
    let queue_h: u16 = if state.queue.is_empty() {
        1
    } else {
        (state.queue.len().min(4) as u16) + 1
    };
    let foot_h = progress_h + queue_h;
    let panes_h = area.height.saturating_sub(foot_h).max(2);

    // Left = local (your machine), right = remote (the server).
    let half = area.width / 2;
    let local_rect = Rect::new(area.x, area.y, half, panes_h);
    let remote_rect = Rect::new(area.x + half, area.y, area.width - half, panes_h);

    let buf = frame.buffer_mut();
    render_pane(
        buf,
        local_rect,
        &state.local,
        "local",
        state.focus == Focus::Local,
    );
    render_pane(
        buf,
        remote_rect,
        &state.remote,
        "remote",
        state.focus == Focus::Remote,
    );

    let queue_y = area.y + panes_h;
    render_queue(buf, area.x, queue_y, area.width, &state.queue);

    if progress_h > 0 {
        let py = area.y + area.height.saturating_sub(1);
        render_progress(buf, area.x, py, area.width, state);
    }
}

fn render_pane(buf: &mut Buffer, rect: Rect, pane: &Pane, title: &str, focused: bool) {
    if rect.width < 6 || rect.height < 2 {
        return;
    }
    let count = format!("{} · {}", pane.cwd.display(), pane.entries.len());
    render_panel_box(buf, rect, title, Some(&count));

    let inner_x = rect.x + 2;
    let inner_w = rect.width.saturating_sub(4) as usize;
    let top = rect.y + 1;
    let rows = rect.height.saturating_sub(2) as usize;
    if rows == 0 {
        return;
    }

    if pane.entries.is_empty() {
        buf.set_string(inner_x, top, "(empty)", theme::dim());
        return;
    }

    let scroll = if pane.selected >= rows {
        pane.selected - rows + 1
    } else {
        0
    };

    for (i, entry) in pane.entries.iter().skip(scroll).take(rows).enumerate() {
        let idx = scroll + i;
        let y = top + i as u16;
        let is_sel = idx == pane.selected;
        let active = is_sel && focused;

        // Highlight the whole selected row of the focused pane.
        if active {
            for cx in (rect.x + 1)..(rect.x + rect.width - 1) {
                if let Some(c) = buf.cell_mut((cx, y)) {
                    c.set_style(theme::selected());
                    c.set_symbol(" ");
                }
            }
        }

        let marker = if active { "▸ " } else { "  " };
        let size_str = if entry.is_dir {
            "<dir>".to_string()
        } else {
            human_size(entry.size)
        };
        let name_budget = inner_w
            .saturating_sub(marker.chars().count() + size_str.chars().count() + 1)
            .max(1);
        let name = ellipsize(&entry.name, name_budget);
        let line = format!("{marker}{name}");

        let name_style = if active {
            theme::selected()
        } else if entry.is_dir {
            theme::cyan()
        } else {
            theme::text()
        };
        buf.set_string(inner_x, y, &line, name_style);

        let size_w = size_str.chars().count() as u16;
        let size_x = (rect.x + rect.width).saturating_sub(size_w + 2);
        let size_style = if active {
            theme::selected()
        } else {
            theme::dim()
        };
        buf.set_string(size_x, y, &size_str, size_style);
    }
}

fn render_queue(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    w: u16,
    queue: &[crate::sftp::model::QueuedTransfer],
) {
    if queue.is_empty() {
        buf.set_string(
            x + 2,
            y,
            "queue: empty  (← download · → upload · u remove · c run)",
            theme::dim(),
        );
        return;
    }
    buf.set_string(
        x + 2,
        y,
        format!("queue ({})  c=run  u=remove", queue.len()),
        theme::heading(),
    );
    for (i, t) in queue.iter().take(4).enumerate() {
        let yy = y + 1 + i as u16;
        let (arrow, label, style) = match t.direction {
            Direction::Download => ("←", "download", theme::green()),
            Direction::Upload => ("→", "upload", theme::amber()),
        };
        let s = format!("{arrow} {label}  {}", t.name);
        let clamped = ellipsize(&s, w.saturating_sub(6) as usize);
        buf.set_string(x + 4, yy, clamped, style);
    }
}

fn render_progress(buf: &mut Buffer, x: u16, y: u16, w: u16, state: &SftpState) {
    let s = if let Some(p) = state.progress {
        let pct = if p.size > 0 {
            (p.transferred as f64 / p.size as f64 * 100.0) as u32
        } else {
            0
        };
        format!(
            "running {}/{}  {pct}%  {} / {}",
            p.index + 1,
            p.total,
            human_size(p.transferred),
            human_size(p.size),
        )
    } else {
        "running…".to_string()
    };
    let clamped = ellipsize(&s, w.saturating_sub(4) as usize);
    buf.set_string(x + 2, y, clamped, theme::amber());
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes}{}", UNITS[0])
    } else {
        format!("{v:.1}{}", UNITS[i])
    }
}
