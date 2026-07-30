use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::App;
use crate::tui::theme;

pub fn render_known_hosts(frame: &mut Frame, app: &App) {
    let Some(state) = app.known_hosts.as_ref() else {
        return;
    };

    let area = frame.area();
    let popup_w = 78u16.min(area.width.saturating_sub(2));
    let list_rows = area.height.saturating_sub(9).clamp(8, 20);
    let popup_h = (list_rows + 7).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    let popup = crate::tui::popup_open_rect(popup, app);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Known Hosts ", theme::heading()))
            .border_style(theme::popup_border()),
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
        theme::bright(),
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
            buf.set_string(popup.x + 1, ry, &blank, theme::selected());
        }
        let base = if is_sel {
            theme::white().bg(theme::SEL_BG)
        } else {
            theme::text()
        };
        let dim = if is_sel {
            theme::mute().bg(theme::SEL_BG)
        } else {
            theme::mute()
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
        buf.set_string(
            type_x,
            ry,
            crate::tui::text::ellipsize(&entry.display_type().to_uppercase(), type_col),
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
            theme::amber(),
        );
    } else if let Some(notice) = &state.notice {
        buf.set_string(
            row_x,
            footer_y,
            crate::tui::text::ellipsize(notice, content_w),
            theme::red(),
        );
    } else {
        let hint = "\u{2191}\u{2193} move \u{00b7} type to filter \u{00b7} d delete \u{00b7} r refresh \u{00b7} Esc close";
        buf.set_string(
            row_x,
            footer_y,
            crate::tui::text::ellipsize(hint, content_w),
            theme::mute(),
        );
    }

    let count_y = popup.y + popup.height.saturating_sub(1);
    let count = format!(" {total} entries ");
    buf.set_string(
        popup.x + popup.width.saturating_sub(count.len() as u16 + 1),
        count_y,
        &count,
        theme::mute(),
    );
}
