//! Compact "your devices" summary in the dashboard's right column, carved out of
//! the Recent panel. Drawn only once a ring is configured; it reuses the auth
//! panel's roles (no roles of its own) and turns entirely to the error colour
//! when the ring is compromised.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::App;
use crate::theme::catalog::{ColorRole, StyleRole};
use crate::tui::widgets::panel_box::{put_clamped, render_panel_box, AUTH_PANEL};

/// Rows the panel takes: borders plus one line.
pub const SYNC_H: u16 = 3;

pub fn render_sync_panel(buf: &mut Buffer, area: Rect, app: &App) {
    let theme = app.theme();
    let sync = &app.sync;
    let text = theme.style(StyleRole::TextPrimary);
    let muted = theme.style(StyleRole::TextMuted);
    render_panel_box(buf, area, "your devices", AUTH_PANEL.plain(), false, theme);

    let x = area.x + 2;
    let w = area.width.saturating_sub(4) as usize;
    let right = x + w as u16;
    let row = area.y + 1;

    // One line: a glyph per device, the trust term, the pending counter.
    let mut col = x;
    for d in sync.devices.iter() {
        if col + 2 > right {
            break;
        }
        let (glyph, role) = d.status.glyph();
        buf.set_string(col, row, glyph, Style::default().fg(theme.color(role)));
        col += 2;
    }
    if sync.devices.is_empty() {
        col += put_clamped(buf, col, row, "no devices", muted, w);
    }
    let mut rest = sync
        .trust
        .as_ref()
        .map_or("no trust session".to_string(), |t| {
            format!("trust {}d", t.term_days)
        });
    if sync.pending > 0 {
        rest.push_str(&format!("  \u{23f3}{} waiting", sync.pending));
    }
    if col + 1 < right {
        put_clamped(buf, col + 1, row, &rest, text, (right - col - 1) as usize);
    }

    // A compromised ring turns the whole block red, border included.
    if sync.compromised() {
        let red = theme.color(ColorRole::StatusError);
        for yy in area.y..area.y + area.height {
            for xx in area.x..area.x + area.width {
                if let Some(cell) = buf.cell_mut((xx, yy)) {
                    cell.set_fg(red);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::sync::{sample_sync_view, DeviceStatusView};
    use crate::app::tests::test_app;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn dashboard(app: &crate::app::App) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(132, 38)).unwrap();
        terminal.draw(|f| crate::tui::render(f, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)).collect::<Vec<_>>())
            .map(|(x, y)| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect()
    }

    #[test]
    fn hidden_when_sync_is_not_configured() {
        let app = test_app(vec![]);
        assert!(!text(&dashboard(&app)).contains("your devices"));
    }

    #[test]
    fn shows_devices_cert_term_and_pending() {
        let mut app = test_app(vec![]);
        app.sync = sample_sync_view();
        let s = text(&dashboard(&app));
        for want in ["your devices", "trust 30d", "2 waiting", "recent sessions"] {
            assert!(s.contains(want), "{want}");
        }
    }

    #[test]
    fn the_whole_block_is_red_on_compromise() {
        let mut app = test_app(vec![]);
        app.sync = sample_sync_view();
        app.sync.devices[1].status = DeviceStatusView::Compromised;
        let buf = dashboard(&app);
        let red = app
            .theme()
            .color(crate::theme::catalog::ColorRole::StatusError);
        // Find the panel's title row, then check every non-blank cell of the
        // rows of the box under it.
        let (tx, ty) = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .find(|&(x, y)| {
                (0..12)
                    .map(|i| buf.cell((x + i, y)).map_or("", |c| c.symbol()))
                    .collect::<String>()
                    == "your devices"
            })
            .expect("panel drawn");
        let mut checked = 0;
        for y in ty..ty + 2 {
            for x in tx..tx + 8 {
                let c = buf.cell((x, y)).unwrap();
                if c.symbol() != " " {
                    assert_eq!(c.fg, red, "({x},{y}) {:?}", c.symbol());
                    checked += 1;
                }
            }
        }
        assert!(checked > 8);
    }
}
