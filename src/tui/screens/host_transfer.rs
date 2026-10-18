//! Dual-pane host transfer screen (`m` in the profile manager's transfer
//! dialog): two bordered panes, the active profile on the left and the other
//! profile on the right, a queue strip and a hint line. Modelled on the SFTP
//! browser, and reusing its theme roles.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::Clear;

use crate::app::host_transfer::{HostTransferScreen, Pane, PaneRow, Side};
use crate::app::App;
use crate::theme::catalog::StyleRole;
use crate::theme::model::ResolvedTheme;
use crate::tui::text::ellipsize;
use crate::tui::widgets::panel_box::{render_panel_box, SFTP_PANEL};

/// Queue rows shown before the strip folds the rest into a count.
const QUEUE_ROWS: usize = 4;

const HINT: &str = "↑↓ move · Tab pane · → push · ← pull · Enter stage · a all shown · / filter · u unstage · x clear · c continue · Esc cancel";

pub fn render_host_transfer(frame: &mut Frame, app: &App) {
    let Some(screen) = app
        .pending_transfer
        .as_ref()
        .and_then(|pending| pending.screen.as_ref())
    else {
        return;
    };
    let area = frame.area();
    if area.height < 8 || area.width < 24 {
        return;
    }
    frame.render_widget(Clear, area);
    let theme = app.theme();
    let buf = frame.buffer_mut();

    let queue_h = if screen.queue.is_empty() {
        1
    } else {
        screen.queue.len().min(QUEUE_ROWS) as u16 + 1
    };
    let foot_h = queue_h + 1; // queue strip + hint line
    let panes_y = area.y + 1;
    let panes_h = area.height.saturating_sub(foot_h + 1).max(3);

    buf.set_string(
        area.x + 1,
        area.y,
        ellipsize(
            &format!(
                "Transfer hosts  {}  ⇄  {}",
                screen.left.profile, screen.right.profile
            ),
            area.width.saturating_sub(2) as usize,
        ),
        theme.style(StyleRole::SftpQueueHeader),
    );

    let half = area.width / 2;
    let left = Rect::new(area.x, panes_y, half, panes_h);
    let right = Rect::new(area.x + half, panes_y, area.width - half, panes_h);
    render_pane(buf, left, screen, Side::Left, StyleRole::SftpLocal, theme);
    render_pane(
        buf,
        right,
        screen,
        Side::Right,
        StyleRole::SftpRemote,
        theme,
    );

    let queue_y = panes_y + panes_h;
    render_queue(buf, area.x, queue_y, area.width, screen, theme);

    let hint_y = area.y + area.height - 1;
    let (text, style) = match &screen.notice {
        Some(notice) => (format!("⚠ {notice}"), theme.style(StyleRole::SftpNotice)),
        None => (HINT.to_string(), theme.style(StyleRole::TextDim)),
    };
    buf.set_string(
        area.x + 1,
        hint_y,
        ellipsize(&text, area.width.saturating_sub(2) as usize),
        style,
    );
}

fn render_pane(
    buf: &mut Buffer,
    rect: Rect,
    screen: &HostTransferScreen,
    side: Side,
    group_role: StyleRole,
    theme: &ResolvedTheme,
) {
    let pane: &Pane = screen.pane(side);
    let focused = screen.focus == side;
    let count = if pane.filter.is_empty() {
        format!("{} hosts", pane.hosts.len())
    } else {
        format!(
            "filter: {} ({}/{})",
            pane.filter,
            pane.shown(),
            pane.hosts.len()
        )
    };
    render_panel_box(
        buf,
        rect,
        &pane.profile,
        SFTP_PANEL.with_badge(&count),
        focused,
        theme,
    );
    if rect.width < 6 || rect.height < 3 {
        return;
    }
    let inner_x = rect.x + 2;
    let inner_w = rect.width.saturating_sub(4) as usize;
    let mut top = rect.y + 1;
    let mut rows = rect.height.saturating_sub(2) as usize;

    if pane.searching {
        let bar = theme.style(StyleRole::SftpSearch);
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
    }
    if rows == 0 {
        return;
    }

    let list = pane.rows();
    if list.is_empty() {
        let msg = if pane.filter.is_empty() {
            "(no hosts)"
        } else {
            "(no matches)"
        };
        buf.set_string(inner_x, top, msg, theme.style(StyleRole::TextDim));
        return;
    }
    // Same "camera that follows the cursor" as the SFTP panes.
    let scroll = pane
        .selected
        .saturating_sub(rows / 2)
        .min(list.len().saturating_sub(rows));
    for (i, row) in list.iter().skip(scroll).take(rows).enumerate() {
        let pos = scroll + i;
        let y = top + i as u16;
        let is_sel = pos == pane.selected;
        let active = is_sel && focused;
        let bar = is_sel.then(|| {
            theme.style(if active {
                StyleRole::SftpSelection
            } else {
                StyleRole::SelectionInactive
            })
        });
        if let Some(bar) = bar {
            for cx in (rect.x + 1)..(rect.x + rect.width - 1) {
                if let Some(c) = buf.cell_mut((cx, y)) {
                    c.set_style(bar);
                    c.set_symbol(" ");
                }
            }
        }
        let marker = if active { "▸ " } else { "  " };
        match row {
            PaneRow::Group { name, count } => {
                let line = format!("{marker}{name} ({count})");
                let style = bar.unwrap_or_else(|| theme.style(group_role));
                buf.set_string(inner_x, y, ellipsize(&line, inner_w), style);
            }
            PaneRow::Host(i) => {
                let host = &pane.hosts[*i];
                let staged = screen.is_staged(side, &host.name);
                let badge = match (staged, side) {
                    (true, Side::Left) => " →",
                    (true, Side::Right) => " ←",
                    _ => "",
                };
                let budget = inner_w.saturating_sub(badge.chars().count());
                let line = format!("{marker}  {}", host.name);
                let style = bar.unwrap_or_else(|| theme.style(StyleRole::TextPrimary));
                buf.set_string(inner_x, y, ellipsize(&line, budget), style);
                if !badge.is_empty() {
                    let bx = (rect.x + rect.width).saturating_sub(badge.chars().count() as u16 + 2);
                    let role = match side {
                        Side::Left => StyleRole::SftpQueueUpload,
                        Side::Right => StyleRole::SftpQueueDownload,
                    };
                    buf.set_string(bx, y, badge, bar.unwrap_or_else(|| theme.style(role)));
                }
            }
        }
    }
}

fn render_queue(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    screen: &HostTransferScreen,
    theme: &ResolvedTheme,
) {
    let budget = width.saturating_sub(2) as usize;
    if screen.queue.is_empty() {
        buf.set_string(
            x + 1,
            y,
            ellipsize(
                "queue: empty  (→ pushes the left selection, ← pulls the right one, c continues)",
                budget,
            ),
            theme.style(StyleRole::TextDim),
        );
        return;
    }
    let pushes = screen.queue.iter().filter(|s| !s.pull).count();
    let header = format!(
        "queue: {} to push → {}  ·  {} to pull ← {}",
        pushes,
        screen.right.profile,
        screen.queue.len() - pushes,
        screen.left.profile
    );
    buf.set_string(
        x + 1,
        y,
        ellipsize(&header, budget),
        theme.style(StyleRole::SftpQueueHeader),
    );
    for (i, staged) in screen.queue.iter().take(QUEUE_ROWS).enumerate() {
        let (arrow, style) = if staged.pull {
            ("←", theme.style(StyleRole::SftpQueueDownload))
        } else {
            ("→", theme.style(StyleRole::SftpQueueUpload))
        };
        buf.set_string(
            x + 1,
            y + 1 + i as u16,
            ellipsize(&format!("{arrow} {}", staged.host), budget),
            style,
        );
    }
    if screen.queue.len() > QUEUE_ROWS {
        // The last visible row tells what is not drawn.
        let more = screen.queue.len() - (QUEUE_ROWS - 1);
        let row = y + QUEUE_ROWS as u16;
        buf.set_string(
            x + 1,
            row,
            ellipsize(&format!("… {more} more"), budget),
            theme.style(StyleRole::TextDim),
        );
    }
}
