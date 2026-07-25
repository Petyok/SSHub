//! SFTP tab body renderer.
//!
//! Three sub-states, mirroring `app.sftp`:
//! - `None` → **picker**: reuse the grouped hosts panel + a "connect" hint.
//! - `Some(state)` with `connecting` → **placeholder**: a centered
//!   "connecting…" line while the worker handshakes.
//! - `Some(state)` → **browser**: two bordered columns (left local / right
//!   remote), a queue strip, and a progress line while a run is in flight.
//!
//! Swapping between them slides (#35): the placeholder rides in and out on the
//! right edge, the two browser panes meet in the middle and part again toward
//! their own edges. See [`render_slide`].
//!
//! Signature matches the other screens (`render_<name>(frame, area, app)`);
//! `tui/mod.rs` wires it through a local `render_sftp_body` wrapper.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::*;

use crate::app::{App, SftpAnim};
use crate::sftp::model::{Direction, Focus, Pane, Phase, SftpState};
use crate::tui::text::ellipsize;
use crate::tui::theme;
use crate::tui::tween;
use crate::tui::widgets::panel_box::render_panel_box;
use crate::tui::SFTP_ANIM;

pub fn render_sftp(frame: &mut Frame, area: Rect, app: &App) {
    if area.height < 3 || area.width < 8 {
        return;
    }
    // A sub-state slide (#35) owns the body for its duration; the resting frames
    // also leave a snapshot behind so the next slide *out* has cells to carry.
    if let Some((kind, at)) = app.sftp_anim.filter(|_| app.motion_enabled()) {
        let p = tween::progress(at, SFTP_ANIM, std::time::Instant::now());
        if p < 1.0 {
            render_slide(frame, area, app, kind, p);
            return;
        }
    }
    render_rest(frame, area, app);
    if app.sftp.is_some() && app.motion_enabled() {
        *app.sftp_snapshot.borrow_mut() = Some(snapshot_area(frame.buffer_mut(), area));
    }
}

/// The SFTP body at rest: picker, "connecting…" placeholder, or live browser.
fn render_rest(frame: &mut Frame, area: Rect, app: &App) {
    match app.sftp.as_ref() {
        None => render_picker(frame, area, app),
        // Still handshaking: show a "connecting…" line, not empty panes. An
        // unreachable host fails into the Notice popup (never a blank browser).
        Some(state) if state.connecting => {
            render_connecting(frame.buffer_mut(), area, app.sftp_host.as_deref())
        }
        Some(state) => render_browser(frame.buffer_mut(), area, state),
    }
}

// ── Sub-state slides (#35) ───────────────────────────────────

/// Draw one frame of an SFTP sub-state slide.
///
/// The layer that is arriving (or leaving) is rendered into a standalone buffer
/// — or read back from the snapshot, when its state is already gone — and then
/// blitted at an eased offset over the layer it replaces. `ConnectIn` /
/// `ConnectOut` move the whole "connecting…" placeholder along the right edge;
/// `PanesIn` / `PanesOut` split at the pane boundary so each half travels to its
/// own edge, and the two panels meet in the middle.
fn render_slide(frame: &mut Frame, area: Rect, app: &App, kind: SftpAnim, p: f32) {
    let e = tween::ease_out(p);
    match kind {
        SftpAnim::ConnectIn => {
            render_picker(frame, area, app);
            let mut layer = Buffer::empty(area);
            render_connecting(&mut layer, area, app.sftp_host.as_deref());
            let dx = ((1.0 - e) * area.width as f32).round() as i32;
            blit_shifted(frame.buffer_mut(), area, area, &layer, dx);
            // Keep the resting layer around: a host that fails before the slide
            // even lands still has cells to carry back out.
            *app.sftp_snapshot.borrow_mut() = Some(layer);
        }
        SftpAnim::ConnectOut => {
            render_picker(frame, area, app);
            if let Some(src) = app.sftp_snapshot.borrow().as_ref() {
                let dx = (e * area.width as f32).round() as i32;
                blit_shifted(frame.buffer_mut(), area, area, src, dx);
            }
        }
        SftpAnim::PanesIn => {
            let Some(state) = app.sftp.as_ref() else {
                return;
            };
            // The placeholder stays put underneath, so the panes close over the
            // very line that was reporting the handshake.
            render_connecting(frame.buffer_mut(), area, app.sftp_host.as_deref());
            let mut layer = Buffer::empty(area);
            render_browser(&mut layer, area, state);
            blit_panes(frame.buffer_mut(), area, &layer, 1.0 - e);
            // Same for an Esc landing mid-slide: the panes part from the rest
            // position rather than snapping to it first.
            *app.sftp_snapshot.borrow_mut() = Some(layer);
        }
        SftpAnim::PanesOut => {
            render_picker(frame, area, app);
            if let Some(src) = app.sftp_snapshot.borrow().as_ref() {
                blit_panes(frame.buffer_mut(), area, src, e);
            }
        }
    }
}

/// Blit the two browser halves of `src` pushed `k` of the way toward their own
/// edge: `k == 0` is at rest (panes met in the middle), `k == 1` fully off both
/// sides.
fn blit_panes(dst: &mut Buffer, area: Rect, src: &Buffer, k: f32) {
    // Same split as `render_browser`, so each half carries exactly its pane.
    let half = area.width / 2;
    let left = Rect::new(area.x, area.y, half, area.height);
    let right = Rect::new(area.x + half, area.y, area.width - half, area.height);
    blit_shifted(dst, left, area, src, -((half as f32 * k).round() as i32));
    blit_shifted(
        dst,
        right,
        area,
        src,
        (right.width as f32 * k).round() as i32,
    );
}

/// Copy the `region` cells of `src` into `dst` shifted `dx` columns, clipped to
/// `clip`. `src` is always a standalone buffer (never `dst` itself), so the copy
/// order doesn't matter.
fn blit_shifted(dst: &mut Buffer, region: Rect, clip: Rect, src: &Buffer, dx: i32) {
    for y in region.top()..region.bottom() {
        for x in region.left()..region.right() {
            let tx = x as i32 + dx;
            if tx < clip.left() as i32 || tx >= clip.right() as i32 {
                continue;
            }
            if let (Some(s), Some(d)) = (src.cell((x, y)), dst.cell_mut((tx as u16, y))) {
                *d = s.clone();
            }
        }
    }
}

/// Clone the `area` cells of `src` into a standalone buffer keeping the same
/// absolute coordinates, so a later frame can blit them while sliding.
fn snapshot_area(src: &Buffer, area: Rect) -> Buffer {
    let mut out = Buffer::empty(area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let (Some(s), Some(d)) = (src.cell((x, y)), out.cell_mut((x, y))) {
                *d = s.clone();
            }
        }
    }
    out
}

/// Centered "connecting to <host>…" placeholder shown while the SFTP worker is
/// still establishing the session.
fn render_connecting(buf: &mut Buffer, area: Rect, host: Option<&str>) {
    let msg = match host {
        Some(h) => format!("\u{27F3} Connecting to {h}\u{2026}"),
        None => "\u{27F3} Connecting\u{2026}".to_string(),
    };
    let w = (msg.chars().count() as u16).min(area.width);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + area.height / 2;
    buf.set_string(x, y, &msg, theme::amber());
}

// ── Picker sub-state ─────────────────────────────────────────

fn render_picker(frame: &mut Frame, area: Rect, app: &App) {
    let list_h = area.height.saturating_sub(1);
    let list_area = Rect::new(area.x, area.y, area.width, list_h);
    // Reuse the dashboard hosts panel so the picker shows the full grouped tree
    // with collapse arrows (▸/▾) and scrolling, identical to the hosts tab.
    crate::tui::widgets::hosts_panel::render_hosts_panel(frame, list_area, app);

    let hint_y = area.y + area.height.saturating_sub(1);
    let hint = if app.sftp_picker_searching {
        format!("search: {}\u{2581}", app.search_query)
    } else {
        "Enter connect (on a group: fold) · / search · Esc back".to_string()
    };
    let style = if app.sftp_picker_searching {
        theme::amber()
    } else {
        theme::dim()
    };
    frame
        .buffer_mut()
        .set_string(area.x + 2, hint_y, &hint, style);
}

// ── Browser sub-state ────────────────────────────────────────

fn render_browser(buf: &mut Buffer, area: Rect, state: &SftpState) {
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

    render_pane(
        buf,
        local_rect,
        &state.local,
        "local",
        state.focus == Focus::Local,
        state.searching && state.focus == Focus::Local,
    );
    render_pane(
        buf,
        remote_rect,
        &state.remote,
        "remote",
        state.focus == Focus::Remote,
        state.searching && state.focus == Focus::Remote,
    );

    let queue_y = area.y + panes_h;
    render_queue(
        buf,
        area.x,
        queue_y,
        area.width,
        &state.queue,
        state.notice.as_deref(),
    );

    if progress_h > 0 {
        let py = area.y + area.height.saturating_sub(1);
        render_progress(buf, area.x, py, area.width, state);
    }
}

fn render_pane(
    buf: &mut Buffer,
    rect: Rect,
    pane: &Pane,
    title: &str,
    focused: bool,
    searching: bool,
) {
    if rect.width < 6 || rect.height < 2 {
        return;
    }
    let total = pane.entries.len();
    let vis = pane.visible_indices();
    let vis_n = vis.len();
    // Subtitle makes an *applied* filter obvious even when not actively typing.
    let count = if pane.filter.is_empty() {
        format!("{} · {}", pane.cwd.display(), total)
    } else {
        format!("filter: {} ({}/{})", pane.filter, vis_n, total)
    };
    render_panel_box(buf, rect, title, Some(&count), false);

    let inner_x = rect.x + 2;
    let inner_w = rect.width.saturating_sub(4) as usize;
    let mut top = rect.y + 1;
    let mut rows = rect.height.saturating_sub(2) as usize;
    if rows == 0 {
        return;
    }

    // Prominent search bar on the top inner row while this focused pane is being
    // typed into, so it's unmistakable that keystrokes are filtering (not lost).
    if searching {
        let bar = Style::default().bg(theme::AMBER).fg(Color::Black);
        for cx in (rect.x + 1)..(rect.x + rect.width - 1) {
            if let Some(c) = buf.cell_mut((cx, top)) {
                c.set_style(bar);
                c.set_symbol(" ");
            }
        }
        let prompt = format!(" search: {}\u{2581}   Esc clear · Enter keep", pane.filter);
        buf.set_string(inner_x, top, ellipsize(&prompt, inner_w), bar);
        top += 1;
        rows = rows.saturating_sub(1);
        if rows == 0 {
            return;
        }
    }

    if vis.is_empty() {
        let msg = if pane.filter.is_empty() {
            "(empty)"
        } else {
            "(no matches)"
        };
        buf.set_string(inner_x, top, msg, theme::dim());
        return;
    }

    // Keep the selection roughly centred (a "camera" that follows), clamped to
    // the list bounds — mirrors the hosts panel's `host_scroll_offset`. Avoids
    // the selection sticking to the bottom edge when scrolling back up.
    let count_len = vis.len();
    let scroll = pane
        .selected
        .saturating_sub(rows / 2)
        .min(count_len.saturating_sub(rows));

    for (i, &entry_idx) in vis.iter().skip(scroll).take(rows).enumerate() {
        let entry = &pane.entries[entry_idx];
        let pos = scroll + i; // position within the visible list
        let y = top + i as u16;
        let is_sel = pos == pane.selected;
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
    notice: Option<&str>,
) {
    if queue.is_empty() {
        let (text, style) = match notice {
            Some(n) => (format!("⚠ {n}"), theme::amber()),
            None => (
                "queue: empty  (← download · → upload · u remove · c run)".to_string(),
                theme::dim(),
            ),
        };
        buf.set_string(
            x + 2,
            y,
            ellipsize(&text, w.saturating_sub(4) as usize),
            style,
        );
        return;
    }
    let header = match notice {
        Some(n) => format!("queue ({})  c=run  u=remove   ⚠ {n}", queue.len()),
        None => format!("queue ({})  c=run  u=remove", queue.len()),
    };
    buf.set_string(
        x + 2,
        y,
        ellipsize(&header, w.saturating_sub(4) as usize),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::new(2, 1, 8, 2)
    }

    /// A source buffer whose every cell carries its own column as a symbol, so a
    /// blit's offset is readable straight off the destination row.
    fn ruler(area: Rect) -> Buffer {
        let mut b = Buffer::empty(area);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                b.cell_mut((x, y))
                    .unwrap()
                    .set_symbol(&format!("{}", x % 10));
            }
        }
        b
    }

    fn row(buf: &Buffer, area: Rect, y: u16) -> String {
        (area.left()..area.right())
            .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect()
    }

    #[test]
    fn blit_shifted_moves_cells_and_clips_at_the_edges() {
        let a = area();
        let src = ruler(a);
        let mut dst = Buffer::empty(a);
        blit_shifted(&mut dst, a, a, &src, 3);
        // Columns 2..4 slid in from off the left edge (nothing wrote them);
        // the rest carry the source columns three to the right.
        assert_eq!(row(&dst, a, 1), "   23456");
        // The three source columns pushed past the right edge are dropped, not
        // wrapped or written out of bounds.
        assert_eq!(row(&dst, a, 2), "   23456");
    }

    #[test]
    fn blit_shifted_zero_offset_is_a_plain_copy() {
        let a = area();
        let src = ruler(a);
        let mut dst = Buffer::empty(a);
        blit_shifted(&mut dst, a, a, &src, 0);
        assert_eq!(row(&dst, a, 1), row(&src, a, 1));
    }

    #[test]
    fn blit_panes_at_rest_reproduces_the_source() {
        let a = area();
        let src = ruler(a);
        let mut dst = Buffer::empty(a);
        blit_panes(&mut dst, a, &src, 0.0);
        assert_eq!(row(&dst, a, 1), row(&src, a, 1));
    }

    #[test]
    fn blit_panes_fully_pushed_leaves_nothing_behind() {
        let a = area();
        let src = ruler(a);
        let mut dst = Buffer::empty(a);
        blit_panes(&mut dst, a, &src, 1.0);
        // Both halves have travelled their own width to opposite edges, so every
        // cell inside the body is still the untouched (blank) destination.
        assert_eq!(row(&dst, a, 1), " ".repeat(a.width as usize));
    }

    #[test]
    fn blit_panes_halves_move_in_opposite_directions() {
        let a = area();
        let src = ruler(a);
        let mut dst = Buffer::empty(a);
        // Half of the way out: the left pane (4 wide) is 2 columns left of rest
        // and the right pane 2 columns right, so half of each has already left
        // the body and the gap they open sits in the middle.
        blit_panes(&mut dst, a, &src, 0.5);
        assert_eq!(row(&dst, a, 1), "45    67");
    }

    #[test]
    fn snapshot_area_copies_the_body_verbatim() {
        let a = area();
        let src = ruler(a);
        let snap = snapshot_area(&src, a);
        assert_eq!(row(&snap, a, 1), row(&src, a, 1));
        assert_eq!(row(&snap, a, 2), row(&src, a, 2));
    }
}
