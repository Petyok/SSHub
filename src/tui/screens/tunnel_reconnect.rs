use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::{App, TUNNEL_RECONNECT_FIELDS};
use crate::tui::theme;

/// Keep-alive reconnect settings overlay (Tunnels tab). `+`/`-` adjust the
/// highlighted row; changes persist to `config.toml` immediately.
pub fn render_tunnel_reconnect_settings(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let popup_w = 58u16.min(area.width.saturating_sub(2));
    let popup_h = (TUNNEL_RECONNECT_FIELDS.len() as u16 + 7).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Tunnel reconnect ", theme::heading()))
            .border_style(theme::popup_border()),
        popup,
    );

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let val_x = popup.x + 28;
    let inner_w = popup.width.saturating_sub(4) as usize;

    buf.set_string(
        row_x,
        popup.y + 1,
        crate::tui::text::ellipsize(
            "Keep-alive tunnels only (per-tunnel toggle in form)",
            inner_w,
        ),
        theme::dim(),
    );

    for (i, (label, _hint)) in TUNNEL_RECONNECT_FIELDS.iter().enumerate() {
        let ry = popup.y + 3 + i as u16;
        if ry >= popup.y + popup.height - 3 {
            break;
        }
        let is_sel = i == app.tunnel_reconnect_selected;
        if is_sel {
            let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
            buf.set_string(popup.x + 1, ry, &blank, theme::selected());
        }
        let label_style = if is_sel {
            theme::white().bg(theme::SEL_BG)
        } else {
            theme::text()
        };
        let marker = if is_sel { "> " } else { "  " };
        let label_avail = (val_x.saturating_sub(row_x + 1)) as usize;
        buf.set_string(
            row_x,
            ry,
            crate::tui::text::ellipsize(&format!("{marker}{label}"), label_avail),
            label_style,
        );

        let value = app.config.tunnel_reconnect.display_field(i);
        let val_style = if is_sel {
            theme::green().bg(theme::SEL_BG)
        } else {
            theme::mute()
        };
        let avail = popup
            .x
            .saturating_add(popup.width)
            .saturating_sub(val_x + 1) as usize;
        buf.set_string(
            val_x,
            ry,
            crate::tui::text::ellipsize(&value, avail),
            val_style,
        );
    }

    let hint = TUNNEL_RECONNECT_FIELDS
        .get(app.tunnel_reconnect_selected)
        .map(|(_, h)| *h)
        .unwrap_or("");
    let hint_y = popup.y + popup.height - 3;
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize(hint, inner_w),
        theme::dim(),
    );
    let legend = "+/- adjust  * reset row  Esc close";
    let legend_y = popup.y + popup.height - 2;
    buf.set_string(
        row_x,
        legend_y,
        crate::tui::text::ellipsize(legend, inner_w),
        theme::mute(),
    );
}
