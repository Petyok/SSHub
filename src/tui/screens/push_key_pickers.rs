//! Searchable host picker and identity dropdown picker for the push key integration.

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders};

use crate::app::App;
use crate::theme::catalog::{PaintRole, StyleRole};

pub fn render_host_picker(frame: &mut Frame, app: &App) {
    let Some(picker) = app.push_key_host_picker.as_ref() else {
        return;
    };
    let matches = app.push_key_host_matches();

    let area = frame.area();
    // `.max(30)` used to grow the popup back past the frame it had just been
    // clamped to, so a narrow terminal got a rect wider than its buffer.
    // `fit_popup` keeps the minimum subordinate to what is actually available.
    let list_rows = matches.len().clamp(1, 8) as u16;
    let popup_w = crate::tui::fit_popup(48, 30, area.width.saturating_sub(4));
    let popup_h = crate::tui::fit_popup(list_rows + 5, 5, area.height.saturating_sub(2));
    if popup_w == 0 || popup_h == 0 {
        return;
    }
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    let theme = app.theme();
    crate::tui::open_popup(frame, popup, theme);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " push public key to host ",
                theme.style(StyleRole::PopupTitle),
            ))
            .border_style(Style::default().fg(crate::tui::blit::line_color(
                theme,
                PaintRole::PickerBorder,
                popup,
            ))),
        popup,
    );
    crate::tui::blit::paint_border(frame.buffer_mut(), popup, theme, PaintRole::PickerBorder);

    // Everything below writes into the buffer directly. `set_string` clips
    // columns on its own, but an out-of-range *row* panics — and `fit_popup`
    // only keeps the outer box legal, not its inner rows. The layout needs a
    // query row, a rule, one list row and the hint row inside the border.
    if popup.width < 4 || popup.height < 5 {
        return;
    }

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let inner_w = popup.width.saturating_sub(3) as usize;

    let query_line = format!("/ {}\u{2588}", picker.query);
    buf.set_string(
        row_x,
        popup.y + 1,
        crate::tui::text::ellipsize(&query_line, inner_w),
        theme.style(StyleRole::PickerQuery),
    );

    let sep: String = std::iter::repeat_n('\u{2500}', inner_w).collect();
    buf.set_string(row_x, popup.y + 2, &sep, theme.style(StyleRole::PopupHint));

    let list_top = popup.y + 3;
    let visible = popup.height.saturating_sub(5) as usize;
    if matches.is_empty() {
        buf.set_string(
            row_x,
            list_top,
            "(no matching hosts)",
            theme.style(StyleRole::PopupLegend),
        );
    } else {
        let scroll = picker.selected.saturating_sub(visible.saturating_sub(1));
        for (i, (_, name)) in matches.iter().skip(scroll).take(visible).enumerate() {
            let idx = scroll + i;
            let ry = list_top + i as u16;
            let is_sel = idx == picker.selected;
            let style = if is_sel {
                theme.style(StyleRole::PickerRowSelected)
            } else {
                theme.style(StyleRole::PickerRow)
            };
            if is_sel {
                let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
                buf.set_string(
                    popup.x + 1,
                    ry,
                    &blank,
                    theme.style(StyleRole::PickerRowSelected),
                );
            }
            let marker = if is_sel { "\u{203a} " } else { "  " };
            buf.set_string(
                row_x,
                ry,
                crate::tui::text::ellipsize(&format!("{marker}{name}"), inner_w),
                style,
            );
        }
    }

    let hint_y = popup.y + popup.height.saturating_sub(2);
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize("type to filter · \u{2191}/\u{2193} · Enter · Esc", inner_w),
        theme.style(StyleRole::PopupHint),
    );
}

pub fn render_identity_picker(frame: &mut Frame, app: &App) {
    let Some(picker) = app.push_key_identity_picker.as_ref() else {
        return;
    };
    let identities = app.push_key_identities();

    let area = frame.area();
    // Same clamp-then-grow bug as the host picker: see `render_host_picker`.
    let list_rows = identities.len().clamp(1, 8) as u16;
    let popup_w = crate::tui::fit_popup(48, 30, area.width.saturating_sub(4));
    let popup_h = crate::tui::fit_popup(list_rows + 4, 4, area.height.saturating_sub(2));
    if popup_w == 0 || popup_h == 0 {
        return;
    }
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    let theme = app.theme();
    crate::tui::open_popup(frame, popup, theme);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " select public key to push ",
                theme.style(StyleRole::PopupTitle),
            ))
            .border_style(Style::default().fg(crate::tui::blit::line_color(
                theme,
                PaintRole::PickerBorder,
                popup,
            ))),
        popup,
    );
    crate::tui::blit::paint_border(frame.buffer_mut(), popup, theme, PaintRole::PickerBorder);

    // As in `render_host_picker`: the outer box being legal says nothing about
    // the rows. This layout needs one list row and the hint row inside the
    // border.
    if popup.width < 4 || popup.height < 4 {
        return;
    }

    let buf = frame.buffer_mut();
    let row_x = popup.x + 2;
    let inner_w = popup.width.saturating_sub(3) as usize;

    let list_top = popup.y + 2;
    let visible = popup.height.saturating_sub(4) as usize;
    if identities.is_empty() {
        buf.set_string(
            row_x,
            list_top,
            "(no key identities)",
            theme.style(StyleRole::PopupLegend),
        );
    } else {
        let scroll = picker.selected.saturating_sub(visible.saturating_sub(1));
        for (i, identity) in identities.iter().skip(scroll).take(visible).enumerate() {
            let idx = scroll + i;
            let ry = list_top + i as u16;
            let is_sel = idx == picker.selected;
            let style = if is_sel {
                theme.style(StyleRole::PickerRowSelected)
            } else {
                theme.style(StyleRole::PickerRow)
            };
            if is_sel {
                let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
                buf.set_string(
                    popup.x + 1,
                    ry,
                    &blank,
                    theme.style(StyleRole::PickerRowSelected),
                );
            }
            let marker = if is_sel { "\u{203a} " } else { "  " };
            buf.set_string(
                row_x,
                ry,
                crate::tui::text::ellipsize(&format!("{marker}{}", identity.name), inner_w),
                style,
            );
        }
    }

    let hint_y = popup.y + popup.height.saturating_sub(2);
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize("\u{2191}/\u{2193} · Enter · Esc", inner_w),
        theme.style(StyleRole::PopupHint),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppMode, PushKeyHostPicker, PushKeyIdentityPicker};
    use crate::store::Identity;
    use crate::test_support::{frame_at, resolved_default, themed_app};

    /// Every popup renderer has to survive a terminal too small to hold it.
    /// The four sizes are the matrix the maintainer specified; how much of the
    /// layout each one leaves room for differs per picker, and that is the
    /// point — whatever a renderer decides to draw, it must not panic.
    const TINY: &[(u16, u16)] = &[(1, 1), (3, 2), (8, 4), (20, 6)];

    fn host_picker_app() -> App {
        let mut app = themed_app(resolved_default());
        app.push_key_host_picker = Some(PushKeyHostPicker {
            query: String::new(),
            selected: 0,
        });
        app.mode = AppMode::PushKeyHostPicker;
        app
    }

    fn identity_picker_app() -> App {
        let mut app = themed_app(resolved_default());
        app.identities = vec![Identity {
            id: 1,
            name: "id_ed25519".into(),
            username: Some("ubuntu".into()),
            private_key: Some(std::path::PathBuf::from("/home/u/.ssh/id_ed25519")),
            certificate: None,
            has_password: false,
        }];
        app.push_key_identity_picker = Some(PushKeyIdentityPicker { selected: 0 });
        app.mode = AppMode::PushKeyIdentityPicker;
        app
    }

    #[test]
    fn pickers_survive_a_tiny_terminal() {
        /// One picker: its label, the state it needs, and its real renderer.
        type Case = (&'static str, fn() -> App, fn(&mut Frame, &App));
        let cases: &[Case] = &[
            ("host picker", host_picker_app, render_host_picker),
            (
                "identity picker",
                identity_picker_app,
                render_identity_picker,
            ),
        ];
        for (name, build, render) in cases {
            let app = build();
            for (w, h) in TINY {
                let area = Rect::new(0, 0, *w, *h);
                // Reaching the assert at all is the proof: `frame_at` drives the
                // real renderer, and an out-of-range row would have panicked.
                let buf = frame_at(area, |frame| render(frame, &app));
                assert_eq!(buf.area, area, "{name} at {w}x{h} drew outside the frame");
            }
        }
    }
}
