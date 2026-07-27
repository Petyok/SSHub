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
use crate::theme::model::ResolvedTheme;
use crate::tui::blit;
use crate::tui::text::ellipsize;
use crate::tui::theme;
use crate::tui::tween;
use crate::tui::widgets::panel_box::{render_panel_box, SFTP_PANEL};
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
        *app.sftp_snapshot.borrow_mut() = Some(blit::snapshot(frame.buffer_mut(), area));
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
        Some(state) => {
            let fill = app.sftp_progress_advance(progress_fraction(state));
            render_browser(
                frame.buffer_mut(),
                area,
                state,
                fill,
                staged_fly_in(app),
                nav_offsets(app, area),
                app.theme(),
            )
        }
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
            blit::blit(frame.buffer_mut(), area, area, &layer, dx, 0);
            // Keep the resting layer around: a host that fails before the slide
            // even lands still has cells to carry back out.
            *app.sftp_snapshot.borrow_mut() = Some(layer);
        }
        SftpAnim::ConnectOut => {
            render_picker(frame, area, app);
            if let Some(src) = app.sftp_snapshot.borrow().as_ref() {
                let dx = (e * area.width as f32).round() as i32;
                blit::blit(frame.buffer_mut(), area, area, src, dx, 0);
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
            render_browser(
                &mut layer,
                area,
                state,
                app.sftp_progress_advance(progress_fraction(state)),
                staged_fly_in(app),
                nav_offsets(app, area),
                app.theme(),
            );
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
    blit::blit(dst, left, area, src, -((half as f32 * k).round() as i32), 0);
    blit::blit(
        dst,
        right,
        area,
        src,
        (right.width as f32 * k).round() as i32,
        0,
    );
}

/// Centered "connecting to <host>…" placeholder shown while the SFTP worker is
/// still establishing the session.
fn render_connecting(buf: &mut Buffer, area: Rect, host: Option<&str>) {
    // Same braille spinner the session connect screen turns, so a handshake
    // reads as in flight rather than hung.
    let spin = tween::spinner_frame_now();
    let msg = match host {
        Some(h) => format!("{spin} Connecting to {h}\u{2026}"),
        None => format!("{spin} Connecting\u{2026}"),
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

fn render_browser(
    buf: &mut Buffer,
    area: Rect,
    state: &SftpState,
    fill: f32,
    staged: f32,
    nav: [i32; 2],
    theme: &ResolvedTheme,
) {
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

    // The left pane is the local filesystem by default, but can be pointed at
    // a second server -- in which case it carries that host's name.
    let left_title = state.left_host.as_deref().unwrap_or("local");
    render_pane(
        buf,
        local_rect,
        &state.local,
        left_title,
        state.focus == Focus::Local,
        state.searching && state.focus == Focus::Local,
        theme,
    );
    if state.left_connecting {
        render_connecting(buf, local_rect, state.left_host.as_deref());
    }
    render_pane(
        buf,
        remote_rect,
        &state.remote,
        "remote",
        state.focus == Focus::Remote,
        state.searching && state.focus == Focus::Remote,
        theme,
    );

    // Slide each pane's listing to its new directory, inside its own border so
    // the box itself stays put (#35).
    slide_listing(buf, local_rect, nav[0]);
    slide_listing(buf, remote_rect, nav[1]);

    let queue_y = area.y + panes_h;
    render_queue(
        buf,
        area.x,
        queue_y,
        area.width,
        &state.queue,
        state.notice.as_deref(),
        staged,
    );

    if progress_h > 0 {
        let py = area.y + area.height.saturating_sub(1);
        render_progress(buf, area.x, py, area.width, state, fill);
    }
}

fn render_pane(
    buf: &mut Buffer,
    rect: Rect,
    pane: &Pane,
    title: &str,
    focused: bool,
    searching: bool,
    theme: &ResolvedTheme,
) {
    if rect.width < 6 || rect.height < 2 {
        return;
    }
    let total = pane.entries.len();
    let vis = pane.visible_indices();
    let vis_n = vis.len();
    // The count has to be of what's on screen: counting entries the dotfile
    // filter is holding back would leave the pane claiming rows it isn't
    // drawing. Say how many are held back instead, since the setting persists
    // and would otherwise be invisible on a later run.
    let hidden = pane.hidden_len();
    let hidden_note = if hidden > 0 {
        format!(" · {hidden} hidden")
    } else {
        String::new()
    };
    // Subtitle makes an *applied* filter obvious even when not actively typing.
    let count = if pane.filter.is_empty() {
        format!("{} · {}{}", pane.cwd.display(), vis_n, hidden_note)
    } else {
        format!(
            "filter: {} ({}/{}){}",
            pane.filter, vis_n, total, hidden_note
        )
    };
    render_panel_box(
        buf,
        rect,
        title,
        SFTP_PANEL.badge(&count),
        focused,
        theme,
        SFTP_PANEL,
    );

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
        // "(empty)" would be a lie when the only entries are dotfiles being
        // filtered out -- which is exactly what a root directory of dotfiles
        // looks like, since it has no ".." row to fall back on.
        let msg = match (pane.filter.is_empty(), hidden > 0) {
            (true, true) => "(all hidden — press . to show)".to_string(),
            (true, false) => "(empty)".to_string(),
            // A search can come up empty *because* of the dotfile filter; say
            // so rather than letting the user conclude the entry isn't there.
            (false, true) => format!("(no matches — {hidden} hidden, . to show)"),
            (false, false) => "(no matches)".to_string(),
        };
        buf.set_string(inner_x, top, &msg, theme::dim());
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
        // The ".." row is a way out, not a listing entry: no size badge.
        let size_str = if entry.is_parent() {
            String::new()
        } else if entry.is_dir {
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
    staged: f32,
) {
    if queue.is_empty() {
        let (text, style) = match notice {
            Some(n) => (format!("⚠ {n}"), theme::amber()),
            None => (
                "queue: empty  (← download · → upload · c run · . hidden · o second host)"
                    .to_string(),
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
    let last = queue.len().saturating_sub(1);
    for (i, t) in queue.iter().take(4).enumerate() {
        let yy = y + 1 + i as u16;
        let (arrow, label, style) = match t.direction {
            Direction::Download => ("←", "download", theme::green()),
            Direction::Upload => ("→", "upload", theme::amber()),
        };
        let s = format!("{arrow} {label}  {}", t.name);
        let clamped = ellipsize(&s, w.saturating_sub(6) as usize);
        // A just-staged row flies in from the side the file is coming from:
        // a download off the remote pane on the right, an upload off the local
        // pane on the left (#35).
        if i == last && staged < 1.0 {
            let travel = (w as f32 * (1.0 - staged)).round() as i32;
            let dx = match t.direction {
                Direction::Download => travel,
                Direction::Upload => -travel,
            };
            let strip = Rect::new(x, yy, w, 1);
            let mut layer = Buffer::empty(strip);
            layer.set_string(x + 4, yy, clamped, style);
            blit::blit(buf, strip, strip, &layer, dx, 0);
        } else {
            buf.set_string(x + 4, yy, clamped, style);
        }
    }
}

/// Shift a pane's listing by `dx` columns within its border, leaving whatever
/// it vacates blank, so a new directory rides in from the side it came from.
fn slide_listing(buf: &mut Buffer, pane: Rect, dx: i32) {
    if dx == 0 || pane.width < 3 || pane.height < 3 {
        return;
    }
    let inner = Rect::new(pane.x + 1, pane.y + 1, pane.width - 2, pane.height - 2);
    let layer = blit::snapshot(buf, inner);
    for y in inner.top()..inner.bottom() {
        for x in inner.left()..inner.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.reset();
            }
        }
    }
    blit::blit(buf, inner, inner, &layer, dx, 0);
}

/// Column offsets for the two panes' directory slides (#35): the listing
/// starts a pane-width to one side and eases home. Descending into a child
/// comes in from the right, stepping back out from the left.
fn nav_offsets(app: &App, area: Rect) -> [i32; 2] {
    let half = (area.width / 2) as f32;
    let now = std::time::Instant::now();
    std::array::from_fn(|i| {
        let Some((deeper, at)) = app.sftp_nav[i] else {
            return 0;
        };
        let p = tween::progress(at, crate::tui::SFTP_NAV_ANIM, now);
        if p >= 1.0 {
            return 0;
        }
        let travel = (1.0 - tween::ease_out(p)) * half;
        let dx = travel.round() as i32;
        if deeper {
            dx
        } else {
            -dx
        }
    })
}

/// How far the newest queue row has flown in, `0.0` to `1.0` (#35). `1.0` at
/// rest, so a settled queue draws in place.
fn staged_fly_in(app: &App) -> f32 {
    if !app.motion_enabled() {
        return 1.0;
    }
    match app.sftp_queue_at {
        Some(at) => tween::ease_out(tween::progress(
            at,
            crate::tui::SFTP_QUEUE_ANIM,
            std::time::Instant::now(),
        )),
        None => 1.0,
    }
}

/// How far the running transfer has got, `0.0` to `1.0`. Zero when nothing is
/// running or the size is unknown, which the bar draws as empty.
fn progress_fraction(state: &SftpState) -> f32 {
    match state.progress {
        Some(p) if p.size > 0 => (p.transferred as f32 / p.size as f32).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

/// Eighth-width block glyphs, for a bar that can end part-way through a cell.
const BAR_PARTIALS: [&str; 7] = ["▏", "▎", "▍", "▌", "▋", "▊", "▉"];

/// Progress line for the running queue: a filled bar under the numbers, drawn
/// at `fill` (the smoothed figure, #35) so it sweeps between the worker's
/// chunked updates instead of stepping.
fn render_progress(buf: &mut Buffer, x: u16, y: u16, w: u16, state: &SftpState, fill: f32) {
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
    let label_w = clamped.chars().count() as u16;
    buf.set_string(x + 2, y, clamped, theme::amber());

    // Bar in whatever is left of the line, right of the numbers.
    let bar_x = x + 3 + label_w;
    let bar_w = (x + w).saturating_sub(bar_x + 2);
    if bar_w < 4 {
        return;
    }
    let units = (bar_w as f32 * 8.0 * fill.clamp(0.0, 1.0)).round() as u16;
    let full = units / 8;
    let rem = (units % 8) as usize;
    for i in 0..bar_w {
        let cell_x = bar_x + i;
        let (glyph, style) = if i < full {
            ("█", theme::green())
        } else if i == full && rem > 0 {
            (BAR_PARTIALS[rem - 1], theme::green())
        } else {
            ("░", theme::dim())
        };
        buf.set_string(cell_x, y, glyph, style);
    }
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

    /// Both SFTP panes wear `sftp.panel` in **both** focus states, badge
    /// included.
    ///
    /// `render_browser` used to compute the focus correctly and `render_pane`
    /// threw it away, so `border_focused` could never fire. The count marker is
    /// checked too: SFTP is one of only three families whose productive caller
    /// really passes a badge.
    #[test]
    fn both_sftp_panes_wear_the_panel_roles_in_both_focus_states() {
        use crate::test_support::{
            assert_panel_wears, buffer_at, panel_marker_theme, PanelFamily, PanelProof,
        };

        let theme = panel_marker_theme();
        let area = Rect::new(0, 0, 60, 12);
        // `render_browser` splits at `area.width / 2`, so these are the two
        // pane rects.
        let local = Rect::new(area.x, area.y, area.width / 2, area.height);
        let remote = Rect::new(
            area.x + area.width / 2,
            area.y,
            area.width - area.width / 2,
            area.height,
        );

        for (focus, local_focused) in [(Focus::Local, true), (Focus::Remote, false)] {
            let mut state = SftpState::new("/remote", "/local");
            state.focus = focus;
            let buf = buffer_at(area, |buf| {
                render_browser(buf, area, &state, 0.0, 1.0, [0, 0], &theme);
            });

            // Both panes get the full five-role assertion. The titles
            // ("local" / "remote") and the badges (each pane's own cwd) are
            // unique strings in the shared buffer, so searching it finds the
            // right pane either way.
            assert_panel_wears(
                &buf,
                local,
                PanelProof {
                    family: PanelFamily::Sftp,
                    focused: local_focused,
                    title: "local",
                    count: Some("/local"),
                    body: (local.x + 2, local.y + 1),
                },
            );
            assert_panel_wears(
                &buf,
                remote,
                PanelProof {
                    family: PanelFamily::Sftp,
                    focused: !local_focused,
                    title: "remote",
                    count: Some("/remote"),
                    body: (remote.x + 2, remote.y + 1),
                },
            );
        }
    }
}
