use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::App;
use crate::theme::catalog::{ColorRole, StyleRole};

pub fn render_known_hosts(frame: &mut Frame, app: &App) {
    let Some(state) = app.known_hosts.as_ref() else {
        return;
    };

    let area = frame.area();
    let popup_w = 104u16.min(area.width.saturating_sub(2));
    let list_rows = area.height.saturating_sub(9).clamp(8, 20);
    let popup_h = (list_rows + 7).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    let popup = crate::tui::popup_open_rect(popup, app);

    frame.render_widget(Clear, popup);
    if popup.width < 4 || popup.height < 4 {
        return;
    }
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " Known Hosts ",
                app.theme().style(StyleRole::PopupTitle),
            ))
            .border_style(crate::tui::popup_border_style(app.theme(), popup)),
        popup,
    );

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let content_w = popup.width.saturating_sub(4) as usize;

    let query_line = format!("\u{203a} {}\u{2588}", state.query);
    buf.set_string(
        row_x,
        popup.y + 1,
        crate::tui::text::ellipsize(&query_line, content_w),
        app.theme().style(StyleRole::PickerQuery),
    );

    let filtered: Vec<usize> = state.filtered_indices();
    let visible = popup.height.saturating_sub(5) as usize;
    let total = filtered.len();
    let selected = if total == 0 {
        0
    } else {
        state.selected.min(total - 1)
    };
    let scroll = selected
        .saturating_sub(visible.saturating_sub(1))
        .min(total.saturating_sub(visible));

    let host_x = row_x;
    let marker_x = popup.x + 28;
    let type_x = popup.x + 43;
    let fp_x = popup.x + 52;

    for (row, &fi) in filtered.iter().skip(scroll).take(visible).enumerate() {
        let entry = &state.entries[fi];
        let ry = popup.y + 3 + row as u16;
        let is_sel = scroll + row == selected;
        if is_sel {
            let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
            buf.set_string(
                popup.x + 1,
                ry,
                &blank,
                app.theme().style(StyleRole::PickerRowSelected),
            );
        }
        let base = if is_sel {
            app.theme().style(StyleRole::PickerRowSelected)
        } else {
            app.theme().style(StyleRole::PickerRow)
        };
        let dim = if is_sel {
            app.theme().style(StyleRole::PickerRowSelected)
        } else {
            app.theme().style(StyleRole::TextMuted)
        };
        let marker_str = if is_sel { "\u{203a} " } else { "  " };

        let host_col = (marker_x.saturating_sub(host_x + 1)) as usize;
        buf.set_string(
            host_x,
            ry,
            crate::tui::text::ellipsize(&format!("{marker_str}{}", entry.display_host()), host_col),
            base,
        );

        let marker_col = (type_x.saturating_sub(marker_x)) as usize;
        let marker_text = match entry.marker {
            Some(m) => m.to_string(),
            None => String::new(),
        };
        buf.set_string(
            marker_x,
            ry,
            crate::tui::text::ellipsize(&marker_text, marker_col),
            dim,
        );

        let type_col = (fp_x.saturating_sub(type_x)) as usize;
        let type_str = entry.display_type();
        buf.set_string(
            type_x,
            ry,
            crate::tui::text::ellipsize(&type_str, type_col),
            dim,
        );

        let fp_col = popup.x.saturating_add(popup.width).saturating_sub(fp_x + 1) as usize;
        let fp = entry.fingerprint.as_deref().unwrap_or("");
        buf.set_string(fp_x, ry, crate::tui::text::ellipsize(fp, fp_col), dim);
    }

    let footer_y = popup.y + popup.height.saturating_sub(2);
    if state.confirming_delete {
        let sel_idx = filtered.get(selected).copied();
        let msg = if let Some(fi) = sel_idx {
            let e = &state.entries[fi];
            if e.is_hashed() {
                "Cannot delete hashed entry \u{2014} use ssh-keygen -R <host> manually".to_string()
            } else {
                format!("Delete ALL keys for {}? [y] yes  [n] no", e.display_host())
            }
        } else {
            String::new()
        };
        buf.set_string(
            row_x,
            footer_y,
            crate::tui::text::ellipsize(&msg, content_w),
            app.theme().style(StyleRole::PopupWarning),
        );
    } else if let Some(notice) = &state.notice {
        let style = if state.notice_is_error {
            app.theme().style(StyleRole::PopupError)
        } else {
            ratatui::style::Style::default().fg(app.theme().color(ColorRole::StatusSuccess))
        };
        buf.set_string(
            row_x,
            footer_y,
            crate::tui::text::ellipsize(notice, content_w),
            style,
        );
    } else {
        let hint = "\u{2191}\u{2193} move \u{00b7} type to filter \u{00b7} Ctrl+D delete \u{00b7} Ctrl+R refresh \u{00b7} Esc close";
        buf.set_string(
            row_x,
            footer_y,
            crate::tui::text::ellipsize(hint, content_w),
            app.theme().style(StyleRole::PopupHint),
        );
    }

    let count_y = popup.y + popup.height.saturating_sub(1);
    let count = format!(" {total} entries ");
    buf.set_string(
        popup.x + popup.width.saturating_sub(count.len() as u16 + 1),
        count_y,
        &count,
        app.theme().style(StyleRole::TextMuted),
    );
    crate::tui::paint_popup_border(frame, popup, app.theme());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::KnownHostsState;
    use crate::known_hosts::KnownHostEntry;
    use crate::test_support::{frame_at, resolved_default, themed_app};

    fn app_with_known_host() -> App {
        let mut app = themed_app(resolved_default());
        app.known_hosts = Some(KnownHostsState {
            entries: vec![KnownHostEntry {
                marker: None,
                hosts: "server.example".into(),
                key_type: "ssh-ed25519".into(),
                fingerprint: Some("SHA256:example".into()),
            }],
            selected: 0,
            query: String::new(),
            confirming_delete: false,
            notice: None,
            notice_is_error: false,
        });
        app
    }

    #[test]
    fn known_hosts_uses_runtime_theme_roles_and_survives_tiny_terminals() {
        let app = app_with_known_host();
        for (width, height) in [(1, 1), (3, 2), (8, 4), (20, 6), (80, 24)] {
            let area = Rect::new(0, 0, width, height);
            let buffer = frame_at(area, |frame| render_known_hosts(frame, &app));
            assert_eq!(buffer.area, area);
        }
    }
}
