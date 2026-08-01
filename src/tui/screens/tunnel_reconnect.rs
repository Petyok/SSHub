use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders};

use crate::app::{App, TUNNEL_RECONNECT_FIELDS};
use crate::theme::catalog::{ColorRole, StyleRole};

/// Keep-alive reconnect settings overlay (Tunnels tab). `+`/`-` adjust the
/// highlighted row; changes persist to `config.toml` immediately.
pub fn render_tunnel_reconnect_settings(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let popup_w = 58u16.min(area.width.saturating_sub(2));
    let popup_h = (TUNNEL_RECONNECT_FIELDS.len() as u16 + 7).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    let popup = crate::tui::popup_open_rect(popup, app);

    let theme = app.theme();
    let selection = theme.style(StyleRole::SettingsRowSelected);
    let hint_style = theme.style(StyleRole::PopupHint);
    // As in the keybind editor: the marker wore the highlighted row's style.
    let focus = theme.style(StyleRole::SettingsMarker);

    crate::tui::open_popup(frame, popup, theme);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " Tunnel reconnect ",
                theme.style(StyleRole::PopupTitle),
            ))
            .border_style(crate::tui::popup_border_style(theme, popup)),
        popup,
    );
    crate::tui::paint_popup_border(frame, popup, theme);

    // Everything below writes into the buffer directly. `set_string` clips
    // columns on its own, but an out-of-range *row* panics — and `fit_popup`
    // only keeps the outer box legal, not its inner rows.
    if popup.width < 4 || popup.height < 4 {
        return;
    }

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
        hint_style,
    );

    for (i, (label, _hint)) in TUNNEL_RECONNECT_FIELDS.iter().enumerate() {
        let ry = popup.y + 3 + i as u16;
        if ry >= popup.y + popup.height.saturating_sub(3) {
            break;
        }
        let is_sel = i == app.tunnel_reconnect_selected;
        if is_sel {
            let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
            buf.set_string(popup.x + 1, ry, &blank, selection);
        }
        let label_style = if is_sel {
            selection
        } else {
            theme.style(StyleRole::TableRow)
        };
        let label_avail = (val_x.saturating_sub(row_x + 1)) as usize;
        // The controls below keep their own foreground, but the selection bar
        // drawn above owns the background of a selected row.
        let over_bar = |style: Style| {
            if is_sel {
                crate::tui::inherit_background(style, selection)
            } else {
                style
            }
        };
        buf.set_string(
            row_x,
            ry,
            if is_sel { "> " } else { "  " },
            if is_sel { over_bar(focus) } else { label_style },
        );
        buf.set_string(
            row_x + 2,
            ry,
            crate::tui::text::ellipsize(label, label_avail.saturating_sub(2)),
            label_style,
        );

        let value = app.config.tunnel_reconnect.display_field(i);
        let val_style = over_bar(if is_sel {
            Style::default().fg(theme.color(ColorRole::StatusSuccess))
        } else {
            theme.style(StyleRole::PopupLegend)
        });
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
    let hint_y = popup.y + popup.height.saturating_sub(3);
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize(hint, inner_w),
        hint_style,
    );
    let legend = "+/- adjust  * reset row  Esc close";
    let legend_y = popup.y + popup.height.saturating_sub(2);
    buf.set_string(
        row_x,
        legend_y,
        crate::tui::text::ellipsize(legend, inner_w),
        theme.style(StyleRole::PopupLegend),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::RoleMarker;
    use crate::test_support::{fg, fg_bg, frame_at, marker, role_marker_theme, themed_app};

    const SELECTION_FG: u32 = 0xb2_0001;
    const SELECTION_BG: u32 = 0xb2_0101;
    const MARKER_FG: u32 = 0xb2_0002;
    const MARKER_BG: u32 = 0xb2_0102;
    const SUCCESS_FG: u32 = 0xb2_0003;

    /// `status.success` is a colour role, so it has no background of its own to
    /// mark — which is exactly why the value used to punch a hole in the bar.
    const MARKERS: &[RoleMarker] = &[
        fg_bg(
            "components.settings.row_selected",
            SELECTION_FG,
            SELECTION_BG,
        ),
        fg_bg("components.settings.marker", MARKER_FG, MARKER_BG),
        fg("components.status.success", SUCCESS_FG),
    ];

    /// The selection bar's background must survive under the focus marker and
    /// the status value, while both keep their own foreground.
    #[test]
    fn selected_row_controls_keep_their_foreground_over_the_selection_background() {
        let mut app = themed_app(role_marker_theme("tunnel-reconnect", MARKERS));
        app.mode = crate::app::AppMode::TunnelReconnectSettings;
        app.tunnel_reconnect_selected = 0;

        let area = Rect::new(0, 0, 80, 24);
        let buf = frame_at(area, |f| render_tunnel_reconnect_settings(f, &app));
        let popup = app.last_popup_rect.get().expect("the popup was laid out");
        let ry = popup.y + 3;
        let sel_bg = marker(SELECTION_BG);

        let marker_cell = buf.cell((popup.x + 2, ry)).unwrap();
        assert_eq!(marker_cell.symbol(), ">", "the focus marker");
        assert_eq!(marker_cell.fg, marker(MARKER_FG), "marker foreground");
        assert_eq!(marker_cell.bg, sel_bg, "marker background");

        let value_cell = buf.cell((popup.x + 28, ry)).unwrap();
        assert_eq!(value_cell.fg, marker(SUCCESS_FG), "status value foreground");
        assert_eq!(value_cell.bg, sel_bg, "status value background");
    }
}
