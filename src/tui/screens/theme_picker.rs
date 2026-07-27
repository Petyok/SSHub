//! Theme picker overlay — Task 10 ships the list half of it.
//!
//! Layout is a pure function of the terminal area ([`layout`]), so the key
//! handler can size a page exactly like the renderer draws one and tests can
//! assert on coordinates instead of hunting for substrings. Every buffer write
//! goes through `set_stringn`, which clips to the width it is given, so the
//! screen stays inside `frame.area()` down to `1x1`.
//!
//! Task 11 fills [`PickerLayout::preview`] with the responsive two-box preview;
//! the seam is already carved here so the list does not have to be restructured
//! for it.

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::{App, ThemeRow, ThemeRowStatus};
use crate::tui::text::ellipsize;
use crate::tui::theme;

/// Width of the right-hand status column (`warning` is the longest word).
const STATUS_W: usize = 7;
/// Width of the built-in/user marker column.
const KIND_W: usize = 8;
/// Rows of diagnostics the footer shows at most.
const MAX_DIAGNOSTIC_ROWS: u16 = 2;

/// Where each part of the picker goes, given the full terminal area.
pub(crate) struct PickerLayout {
    pub popup: Rect,
    /// Scrollable theme list. Zero-height on a terminal too small for a row.
    pub list: Rect,
    /// Task 11's two-box preview. Always `None` in Task 10 — the seam exists
    /// so the list geometry above does not have to be restructured for it.
    #[allow(dead_code)]
    pub preview: Option<Rect>,
    /// The user themes directory line.
    pub path: Option<Rect>,
    /// Diagnostics / save errors. May be zero-height.
    pub diagnostics: Rect,
    pub legend: Option<Rect>,
}

/// Popup geometry — 70% x 60% of the terminal, clamped so it never exceeds it.
pub(crate) fn popup_rect(area: Rect) -> Rect {
    let width = (area.width * 70 / 100).max(48).min(area.width);
    let height = (area.height * 60 / 100).max(16).min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

/// Carve the popup into list and footer rows.
///
/// The footer is taken from the bottom in priority order (legend, path,
/// diagnostics) and each part is only claimed while the list keeps at least one
/// row: on a tiny terminal the list stays usable and the chrome disappears,
/// which is the spec's "Liste bleibt bedienbar" rule.
pub(crate) fn layout(area: Rect) -> PickerLayout {
    let popup = popup_rect(area);
    let inner = Rect {
        x: popup.x.saturating_add(1),
        y: popup.y.saturating_add(1),
        width: popup.width.saturating_sub(2),
        height: popup.height.saturating_sub(2),
    };
    let mut remaining = if inner.width == 0 { 0 } else { inner.height };
    let mut bottom = inner.y + remaining;

    let mut take = |rows: u16, remaining: &mut u16| -> Option<Rect> {
        // Keep one row for the list at all times.
        let rows = rows.min(remaining.saturating_sub(1));
        if rows == 0 {
            return None;
        }
        *remaining -= rows;
        bottom -= rows;
        Some(Rect::new(inner.x, bottom, inner.width, rows))
    };

    let legend = take(1, &mut remaining);
    let path = take(1, &mut remaining);
    let diagnostics = take(MAX_DIAGNOSTIC_ROWS, &mut remaining)
        .unwrap_or_else(|| Rect::new(inner.x, bottom, inner.width, 0));

    let list = Rect::new(inner.x, inner.y, inner.width, remaining);
    PickerLayout {
        popup,
        list,
        preview: None,
        path,
        diagnostics,
        legend,
    }
}

/// How many list rows fit — the page step for PageUp/PageDown.
pub(crate) fn visible_rows(area: Rect) -> usize {
    layout(area).list.height as usize
}

/// First visible row index, keeping `selected` on screen.
pub(crate) fn scroll_offset(selected: usize, len: usize, visible: usize) -> usize {
    if visible == 0 || len <= visible {
        return 0;
    }
    let max = len - visible;
    selected.saturating_sub(visible - 1).min(max)
}

/// One list line, already laid out into its three columns.
fn row_line(row: &ThemeRow, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    // Name gets whatever the two fixed right-hand columns leave over.
    let name_w = width.saturating_sub(KIND_W + STATUS_W + 2);
    if name_w == 0 {
        return ellipsize(row.display_name(), width);
    }
    let kind = if row.builtin { "built-in" } else { "user" };
    format!(
        "{:<name_w$} {:<KIND_W$} {:<STATUS_W$}",
        ellipsize(row.display_name(), name_w),
        kind,
        row.status.label(),
    )
}

fn status_style(status: ThemeRowStatus) -> Style {
    match status {
        ThemeRowStatus::Valid => theme::green(),
        ThemeRowStatus::Warning => theme::amber(),
        ThemeRowStatus::Invalid => theme::red(),
    }
}

/// The lines the footer explains the selected row with: a save/reload failure
/// first, then the row's own diagnostics, then directory-level ones.
///
/// The directory list is deliberately not filtered to errors — an unusable file
/// name or the 256-file cut are warnings, and they are exactly what explains a
/// theme *missing* from the list.
fn diagnostic_lines(app: &App, selected: Option<&ThemeRow>) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(error) = app.theme_picker.as_ref().and_then(|s| s.error.as_deref()) {
        lines.push(error.to_string());
    }
    if let Some(row) = selected {
        lines.extend(row.diagnostics.iter().map(|d| d.message.clone()));
    }
    lines.extend(
        app.theme_manager
            .registry()
            .diagnostics()
            .iter()
            .map(|d| d.message.clone()),
    );
    lines
}

pub fn render(frame: &mut Frame, app: &App) {
    let areas = layout(frame.area());
    let popup = crate::tui::popup_open_rect(areas.popup, app);
    if popup.width == 0 || popup.height == 0 {
        return;
    }

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Theme ", theme::heading()))
            .border_style(theme::popup_border()),
        popup,
    );

    let rows = app.theme_picker_rows();
    let selected = app.theme_picker.as_ref().map(|s| s.selected).unwrap_or(0);
    let list = areas.list;
    let inner_w = list.width as usize;
    if inner_w == 0 {
        return;
    }

    let buf = frame.buffer_mut();
    let visible = list.height as usize;
    let offset = scroll_offset(selected, rows.len(), visible);
    for (line, row) in rows.iter().skip(offset).take(visible).enumerate() {
        let y = list.y + line as u16;
        let index = offset + line;
        let is_sel = index == selected;
        let base = if is_sel {
            theme::selected()
        } else {
            theme::text()
        };
        if is_sel {
            buf.set_stringn(list.x, y, " ".repeat(inner_w), inner_w, base);
        }
        buf.set_stringn(list.x, y, row_line(row, inner_w), inner_w, base);
        // The status word is re-stamped in its own colour, over the same cells
        // the line already reserved for it, so the column never shifts.
        if inner_w > KIND_W + STATUS_W + 2 {
            let status_x = list.x + (inner_w - STATUS_W) as u16;
            let mut style = status_style(row.status);
            if is_sel {
                style = style.bg(theme::SEL_BG);
            }
            buf.set_stringn(status_x, y, row.status.label(), STATUS_W, style);
        }
    }

    let selected_row = rows.get(selected);
    if let Some(path) = areas.path {
        // Show where new files go: the selected user theme's own path, or the
        // themes directory itself.
        let text = match selected_row.and_then(|row| row.path.as_ref()) {
            Some(path) => path.display().to_string(),
            None => match app.theme_manager.themes_dir() {
                Some(dir) => dir.display().to_string(),
                None => "no themes directory".to_string(),
            },
        };
        buf.set_stringn(path.x, path.y, text, path.width as usize, theme::dim());
    }

    let diagnostics = areas.diagnostics;
    if diagnostics.height > 0 {
        let error_first = app.theme_picker.as_ref().is_some_and(|s| s.error.is_some());
        for (i, line) in diagnostic_lines(app, selected_row)
            .into_iter()
            .take(diagnostics.height as usize)
            .enumerate()
        {
            let style = if i == 0 && error_first {
                theme::red()
            } else {
                theme::mute()
            };
            buf.set_stringn(
                diagnostics.x,
                diagnostics.y + i as u16,
                line,
                diagnostics.width as usize,
                style,
            );
        }
    }

    if let Some(legend) = areas.legend {
        let text = "Enter save \u{b7} \u{2191}\u{2193} move \u{b7} r reload \u{b7} Esc cancel";
        buf.set_stringn(
            legend.x,
            legend.y,
            text,
            legend.width as usize,
            theme::mute(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppDeps, AppMode};
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

    fn picker_app() -> App {
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
        app.settings_selected = 0;
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::empty(),
        ))
        .unwrap();
        assert_eq!(app.mode, AppMode::ThemePicker);
        app
    }

    fn draw(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn line(buffer: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
        (0..width).map(|x| buffer[(x, y)].symbol()).collect()
    }

    /// The smallest terminal there is. Nothing may panic and nothing may be
    /// written outside the buffer.
    #[test]
    fn renders_at_one_by_one() {
        let app = picker_app();
        let buffer = draw(&app, 1, 1);
        assert_eq!(buffer.area().width, 1);
        assert_eq!(buffer.area().height, 1);
    }

    /// Every degenerate size the spec names must survive, and the layout must
    /// never claim a rect outside the terminal.
    #[test]
    fn tiny_terminals_keep_the_layout_inside_the_area() {
        let app = picker_app();
        for (w, h) in [(1, 1), (1, 24), (80, 1), (2, 3), (5, 5), (49, 17)] {
            let area = Rect::new(0, 0, w, h);
            let areas = layout(area);
            assert!(areas.popup.right() <= area.right(), "{w}x{h} popup width");
            assert!(
                areas.popup.bottom() <= area.bottom(),
                "{w}x{h} popup height"
            );
            assert!(areas.list.right() <= area.right(), "{w}x{h} list width");
            assert!(areas.list.bottom() <= area.bottom(), "{w}x{h} list bottom");
            draw(&app, w, h);
        }
    }

    /// Built-ins first, in their frozen order, at exact coordinates.
    #[test]
    fn the_list_starts_with_the_built_ins_in_order() {
        let app = picker_app();
        let buffer = draw(&app, 80, 24);
        let areas = layout(Rect::new(0, 0, 80, 24));
        let list = areas.list;
        let names = ["Default", "Summer", "Aqua", "Fire", "High Contrast"];
        for (i, _) in names.iter().enumerate() {
            let row = line(&buffer, list.y + i as u16, 80);
            let cell: String = row
                .chars()
                .skip(list.x as usize)
                .take(list.width as usize)
                .collect();
            assert!(
                cell.trim_end().ends_with("built-in valid"),
                "row {i} is not a valid built-in: {cell:?}"
            );
        }
    }

    /// The selected row is the one the picker state points at.
    #[test]
    fn the_selected_row_carries_the_selection_background() {
        let mut app = picker_app();
        app.select_theme_row(2);
        let buffer = draw(&app, 80, 24);
        let list = layout(Rect::new(0, 0, 80, 24)).list;
        assert_eq!(buffer[(list.x, list.y + 2)].bg, theme::SEL_BG);
        assert_ne!(buffer[(list.x, list.y)].bg, theme::SEL_BG);
    }

    /// The legend and the themes-directory line occupy the bottom rows.
    #[test]
    fn the_footer_shows_the_legend_and_the_theme_path() {
        let app = picker_app();
        let areas = layout(Rect::new(0, 0, 80, 24));
        let buffer = draw(&app, 80, 24);
        let legend = line(&buffer, areas.legend.unwrap().y, 80);
        assert!(legend.contains("Enter save"), "{legend:?}");
        assert!(legend.contains("r reload"), "{legend:?}");
        assert!(legend.contains("Esc cancel"), "{legend:?}");
        // The built-ins-only test manager owns no directory, and says so rather
        // than pointing at the working directory.
        let path = line(&buffer, areas.path.unwrap().y, 80);
        assert!(path.contains("no themes directory"), "{path:?}");
    }

    /// `scroll_offset` keeps the selection visible without over-scrolling.
    #[test]
    fn the_list_scrolls_only_as_far_as_the_last_page() {
        assert_eq!(scroll_offset(0, 10, 4), 0);
        assert_eq!(scroll_offset(3, 10, 4), 0);
        assert_eq!(scroll_offset(4, 10, 4), 1);
        assert_eq!(scroll_offset(9, 10, 4), 6);
        assert_eq!(scroll_offset(9, 10, 0), 0);
        assert_eq!(scroll_offset(2, 3, 10), 0);
    }
}
