use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders};

use crate::app::{App, SettingItem, SETTINGS_ITEMS};
use crate::theme::catalog::{ColorRole, StyleRole};

/// Settings overlay: one row per [`SETTINGS_ITEMS`] entry — the Theme action
/// row showing the active theme id, then the appearance checkboxes. Space/Enter
/// flips the highlighted toggle (persisted immediately); Esc closes.
pub fn render_settings(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let popup_w = 56u16.min(area.width.saturating_sub(2));
    let popup_h = (SETTINGS_ITEMS.len() as u16 + 6).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    let popup = crate::tui::popup_open_rect(popup, app);

    let theme = app.theme();
    let selection = theme.style(StyleRole::SettingsRowSelected);
    let row = theme.style(StyleRole::TableRow);
    let legend = theme.style(StyleRole::PopupLegend);

    crate::tui::open_popup(frame, popup, theme);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " Settings ",
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
    let inner_w = popup.width.saturating_sub(4) as usize;

    // Labels share one column so action and toggle rows line up.
    let label_x = row_x + 4;
    let label_w = inner_w.saturating_sub(4);

    for (i, desc) in SETTINGS_ITEMS.iter().enumerate() {
        let ry = popup.y + 1 + i as u16;
        if ry >= popup.y + popup.height.saturating_sub(2) {
            break;
        }
        let is_sel = i == app.settings_selected;
        if is_sel {
            let blank = " ".repeat(popup.width.saturating_sub(2) as usize);
            buf.set_string(popup.x + 1, ry, &blank, selection);
        }
        let label_style = if is_sel { selection } else { row };
        // The controls below carry their own foreground but must not carry a
        // background onto a selected row: the bar drawn above owns that.
        let over_bar = |style: Style| {
            if is_sel {
                crate::tui::inherit_background(style, selection)
            } else {
                style
            }
        };

        match app.setting_value(desc.item) {
            Some(on) => {
                let check = if on { "[x] " } else { "[ ] " };
                let check_style = over_bar(if on {
                    Style::default().fg(theme.color(ColorRole::StatusSuccess))
                } else {
                    legend
                });
                buf.set_string(row_x, ry, check, check_style);
                buf.set_string(
                    label_x,
                    ry,
                    crate::tui::text::ellipsize(desc.label, label_w),
                    label_style,
                );
            }
            // Action row: the current value stands in for the checkbox.
            None => {
                let label = crate::tui::text::ellipsize(desc.label, label_w);
                buf.set_string(label_x, ry, &label, label_style);
                let used = label.chars().count() + 1;
                let value = crate::tui::text::ellipsize(
                    app.active_theme_id(),
                    label_w.saturating_sub(used),
                );
                buf.set_string(
                    label_x + used as u16,
                    ry,
                    value,
                    over_bar(theme.style(StyleRole::PickerMatch)),
                );
            }
        }
    }

    // Footer: the hint for the highlighted row + key legend.
    let selected = SETTINGS_ITEMS.get(app.settings_selected);
    let hint = selected.map(|d| d.hint).unwrap_or("");
    let hint_y = popup.y + popup.height.saturating_sub(3);
    buf.set_string(
        row_x,
        hint_y,
        crate::tui::text::ellipsize(hint, inner_w),
        theme.style(StyleRole::PopupHint),
    );
    let action = matches!(selected.map(|d| &d.item), Some(SettingItem::Theme));
    let legend_text = if action {
        "Enter choose \u{b7} \u{2191}\u{2193} move \u{b7} Esc close"
    } else {
        "Space toggle \u{b7} \u{2191}\u{2193} move \u{b7} Esc close"
    };
    let legend_y = popup.y + popup.height.saturating_sub(2);
    buf.set_string(
        row_x,
        legend_y,
        crate::tui::text::ellipsize(legend_text, inner_w),
        legend,
    );
}

#[cfg(test)]
mod tests {
    use super::render_settings;
    use crate::app::{App, AppDeps, AppMode, SETTINGS_ITEMS};
    use crate::config::AppConfig;
    use crate::metadata::MetadataDb;
    use crate::ssh::{HostResolver, SshHost};
    use crate::store::LauncherStore;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::sync::Arc;

    struct NoHosts;

    impl HostResolver for NoHosts {
        fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }
        fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
            Ok(SshHost::new(name))
        }
    }

    /// Popup geometry for an 80x24 screen: `popup_w = 56`, one row per settings
    /// item plus the frame and footer, centered. The row count is read from
    /// [`SETTINGS_ITEMS`] so adding a setting moves these tests with it instead
    /// of breaking them.
    const POPUP_X: u16 = (80 - 56) / 2;
    const POPUP_H: u16 = SETTINGS_ITEMS.len() as u16 + 6;
    const POPUP_Y: u16 = (24 - POPUP_H) / 2;
    const LABEL_X: u16 = POPUP_X + 2 + 4;

    /// One rendered line of the settings popup, as a string.
    fn render_line(selected: usize, y: u16) -> String {
        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(NoHosts),
                metadata: Arc::new(MetadataDb::default()),
                store: Arc::new(LauncherStore::open_in_memory().unwrap()),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.mode = AppMode::Settings;
        app.settings_selected = selected;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| render_settings(f, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..80).map(|x| buffer[(x, y)].symbol()).collect()
    }

    /// The selection bar's background must survive under the two controls a
    /// selected row draws over it — the checkbox and the Theme row's value —
    /// while each keeps its own foreground.
    ///
    /// Both are written after the bar. `status.success` is a colour role with
    /// no background at all and `picker.match` carries its own, so one punched
    /// a `Reset` hole in the bar and the other overwrote it with a foreign
    /// colour.
    #[test]
    fn selected_row_controls_keep_their_foreground_over_the_selection_background() {
        use crate::test_support::RoleMarker;
        use crate::test_support::{fg, fg_bg, frame_at, marker, role_marker_theme, themed_app};
        use ratatui::layout::Rect;

        const SELECTION_FG: u32 = 0xb3_0001;
        const SELECTION_BG: u32 = 0xb3_0101;
        const MATCH_FG: u32 = 0xb3_0002;
        const MATCH_BG: u32 = 0xb3_0102;
        const SUCCESS_FG: u32 = 0xb3_0003;

        const MARKERS: &[RoleMarker] = &[
            fg_bg(
                "components.settings.row_selected",
                SELECTION_FG,
                SELECTION_BG,
            ),
            fg_bg("components.picker.match", MATCH_FG, MATCH_BG),
            fg("components.status.success", SUCCESS_FG),
        ];

        let area = Rect::new(0, 0, 80, 24);
        let sel_bg = marker(SELECTION_BG);

        // Row 0 is the Theme action row: its value stands in for the checkbox.
        let mut app = themed_app(role_marker_theme("settings", MARKERS));
        app.mode = AppMode::Settings;
        app.settings_selected = 0;
        let buf = frame_at(area, |f| render_settings(f, &app));
        let popup = app.last_popup_rect.get().expect("the popup was laid out");
        let (value_x, _) = crate::test_support::find_text(&buf, app.active_theme_id());
        let value_cell = buf.cell((value_x, popup.y + 1)).unwrap();
        assert_eq!(value_cell.fg, marker(MATCH_FG), "theme value foreground");
        assert_eq!(value_cell.bg, sel_bg, "theme value background");

        // Row 1 is a toggle: turn it on so the checkbox wears `status.success`.
        let mut app = themed_app(role_marker_theme("settings", MARKERS));
        app.mode = AppMode::Settings;
        app.settings_selected = 1;
        app.config.appearance.transparent_sshub_background = true;
        let buf = frame_at(area, |f| render_settings(f, &app));
        let popup = app.last_popup_rect.get().expect("the popup was laid out");
        let check_cell = buf.cell((popup.x + 2, popup.y + 2)).unwrap();
        assert_eq!(check_cell.symbol(), "[", "the checkbox");
        assert_eq!(check_cell.fg, marker(SUCCESS_FG), "checkbox foreground");
        assert_eq!(check_cell.bg, sel_bg, "checkbox background");
    }

    /// A gradient `components.popup.border` really reaches the popup frame, and
    /// the popup title on the same top row keeps its own role.
    ///
    /// Settings stands in for all sixteen `popup_border_style` call sites: they
    /// share one contract — render the block in the solid fallback, then run
    /// `paint_popup_border` over it — and the mechanism is proved once in
    /// `blit`. What this adds is that the wiring exists at a real call site.
    #[test]
    fn a_gradient_popup_border_reaches_the_frame_without_recolouring_the_title() {
        use crate::test_support::{find_text, frame_at, resolved_source, themed_app};
        use ratatui::layout::Rect;

        let theme = resolved_source(
            "ringed",
            "schema_version = 1\nname = \"Ringed\"\nextends = \"default\"\n\n\
             [gradients.ring]\ndirection = \"perimeter\"\n\
             stops = [ { at = 0.0, color = \"#ff0000\" }, { at = 0.5, color = \"#0000ff\" }, \
             { at = 1.0, color = \"#ff0000\" } ]\n\n\
             [components.popup]\nborder = { gradient = \"gradients.ring\" }\n\
             title = { foreground = \"#00ff00\" }\n",
        );
        let mut app = themed_app(theme);
        app.mode = AppMode::Settings;

        let buf = frame_at(Rect::new(0, 0, 80, 24), |f| render_settings(f, &app));
        let popup = app.last_popup_rect.get().expect("the popup was laid out");

        let bottom: Vec<_> = (popup.x..popup.right())
            .map(|x| buf.cell((x, popup.bottom() - 1)).unwrap().fg)
            .collect();
        assert!(
            bottom.windows(2).any(|pair| pair[0] != pair[1]),
            "the popup border stayed flat: {bottom:?}"
        );

        let title = find_text(&buf, "Settings");
        assert_eq!(
            buf.cell(title).unwrap().fg,
            ratatui::style::Color::Rgb(0x00, 0xff, 0x00),
            "the ring pass repainted the popup title"
        );
    }

    /// The Theme row is an action: no checkbox, and the active theme id sits
    /// right after the label.
    #[test]
    fn the_theme_row_shows_the_active_theme_instead_of_a_checkbox() {
        let line = render_line(0, POPUP_Y + 1);
        let label_col: String = line.chars().skip(LABEL_X as usize).take(16).collect();
        assert_eq!(label_col, "Theme... default");
        let check_col: String = line.chars().skip(POPUP_X as usize + 2).take(4).collect();
        assert_eq!(check_col, "    ", "an action row must not draw a checkbox");
    }

    /// A toggle row keeps its checkbox in the same column as before.
    #[test]
    fn a_toggle_row_still_renders_its_checkbox() {
        let line = render_line(1, POPUP_Y + 2);
        let check_col: String = line.chars().skip(POPUP_X as usize + 2).take(4).collect();
        assert_eq!(check_col, "[ ] ");
        let label_col: String = line.chars().skip(LABEL_X as usize).take(17).collect();
        assert_eq!(label_col, "SSHub transparent");
    }

    /// Legend follows the kind of the highlighted row.
    #[test]
    fn the_legend_matches_the_selected_row_kind() {
        let legend_y = POPUP_Y + POPUP_H - 2;
        assert!(render_line(0, legend_y).contains("Enter choose"));
        assert!(render_line(1, legend_y).contains("Space toggle"));
    }

    /// Hints render at the popup bottom with `inner_w = 56 - 4` columns.
    /// A hint that needs ellipsizing ends flush at the limit, and any
    /// ambiguous-width char (em dash, middle dot, ellipsis) that a terminal
    /// draws 2 cells wide then pushes the tail onto the popup border. Keep
    /// hints short and ASCII so neither can happen. Labels share the same
    /// constraint: the action row draws its value right after the label.
    #[test]
    fn settings_hints_fit_the_popup_without_ellipsizing() {
        const INNER_W: usize = 56 - 4;
        for desc in &SETTINGS_ITEMS {
            let (label, hint) = (desc.label, desc.hint);
            assert!(
                hint.chars().count() <= INNER_W,
                "hint for '{label}' is {} chars, must be <= {INNER_W}",
                hint.chars().count()
            );
            assert!(
                hint.is_ascii(),
                "hint for '{label}' contains a non-ASCII char that may render double-width"
            );
            assert!(
                label.is_ascii() && label.chars().count() <= INNER_W - 4,
                "label '{label}' must be ASCII and fit the label column"
            );
        }
    }
}
