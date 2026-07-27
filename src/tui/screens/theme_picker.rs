//! Theme picker overlay — the list, the live two-box preview and the footer.
//!
//! Layout is a pure function of the terminal area
//! ([`plan_theme_picker_layout`]), so the key handler can size a page exactly
//! like the renderer draws one and tests can assert on coordinates instead of
//! hunting for substrings. Every buffer write goes through `set_stringn`, which
//! clips to the width it is given, so the screen stays inside `frame.area()`
//! down to `1x1`.
//!
//! The preview is drawn against the theme that is *live* while the picker is
//! open (`app.theme()`): navigation already previews the selected theme on the
//! whole TUI, so re-resolving here could only ever disagree with what the rest
//! of the screen shows.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

use crate::app::{App, ThemeRow, ThemeRowStatus};
use crate::theme::catalog::{ColorRole, PaintRole, StyleRole};
use crate::theme::gradient::{
    paint_gradient_area, paint_gradient_line, paint_gradient_ring, CellSelection, PaintChannel,
};
use crate::theme::model::{ResolvedPaint, ResolvedTheme};
use crate::tui::text::ellipsize;
use crate::tui::theme;

/// Width of the right-hand status column (`warning` is the longest word).
const STATUS_W: usize = 7;
/// Width of the built-in/user marker column.
const KIND_W: usize = 8;
/// Rows of diagnostics the footer shows at most.
const MAX_DIAGNOSTIC_ROWS: u16 = 2;

/// Narrowest preview that still fits every role the spec enumerates: a frame,
/// one padding column on each side and the 24-column text sampler.
const MIN_PREVIEW_W: u16 = 34;
/// Narrowest list that still shows a name next to its two fixed columns.
const MIN_LIST_W: u16 = 24;
/// Rows the preview needs: the top box, one background strip, the bottom box.
const MIN_PREVIEW_H: u16 = TOP_BOX_H + 1 + BOTTOM_BOX_H;
/// Rows the list keeps for itself before *any* preview may appear.
///
/// One, the same rule the footer chrome is carved with. A higher floor here
/// would make the stacked (narrow) regime cost more vertical space than the
/// side-by-side (wide) one, so the fallback for a cramped terminal would be the
/// hardest layout to reach — see [`PREVIEW_POPUP_H`], which is where a *usable*
/// list is asked for instead.
const MIN_STACKED_LIST_H: u16 = 1;
/// List rows the popup grows to make room for once a preview fits at all.
const PREFERRED_LIST_H: u16 = 3;
/// Rows the footer chrome claims: legend, path and the diagnostics area.
const FOOTER_H: u16 = 2 + MAX_DIAGNOSTIC_ROWS;
/// How tall the popup asks to be so that the preview can actually appear.
///
/// The picker is an overlay — nothing else competes for these rows — and a
/// preview nobody ever sees is the one outcome this screen cannot afford. A
/// flat 60% of the terminal put the two-box preview out of reach on an 80x24
/// terminal, which is the size most people still run.
const PREVIEW_POPUP_H: u16 = 2 + FOOTER_H + PREFERRED_LIST_H + MIN_PREVIEW_H;
/// One blank column between list and preview so the two never touch.
const PREVIEW_GAP: u16 = 1;
/// Frame plus four content rows: text sampler, divider, tabs, statuses.
const TOP_BOX_H: u16 = 6;
/// Frame plus three content rows: selected host, host, footer key pair.
const BOTTOM_BOX_H: u16 = 5;

/// Which of the spec's three responsive regimes the picker is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThemePickerLayoutMode {
    /// Wide enough for list and preview next to each other.
    SideBySide,
    /// Narrow: the preview moves underneath the list.
    Stacked,
    /// Too small for either — the list keeps working and a hint says why the
    /// preview is gone.
    ListOnly,
}

/// Where each part of the picker goes, given the full terminal area.
pub(crate) struct ThemePickerLayout {
    pub mode: ThemePickerLayoutMode,
    pub popup: Rect,
    /// Scrollable theme list. Zero-height on a terminal too small for a row.
    pub list: Rect,
    /// The two-box live preview, or `None` in [`ThemePickerLayoutMode::ListOnly`].
    pub preview: Option<Rect>,
    /// One row explaining the hidden preview. `None` only when even that row
    /// would have cost the list its last one.
    pub notice: Option<Rect>,
    /// The user themes directory line.
    pub path: Option<Rect>,
    /// Diagnostics / save errors. May be zero-height.
    pub diagnostics: Rect,
    pub legend: Option<Rect>,
}

/// Popup geometry — 70% x 60% of the terminal, but never shorter than the rows
/// the preview needs, and always clamped so it never exceeds the terminal.
pub(crate) fn popup_rect(area: Rect) -> Rect {
    let width = (area.width * 70 / 100).max(48).min(area.width);
    let height = (area.height * 60 / 100)
        .max(16)
        .max(PREVIEW_POPUP_H)
        .min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

/// Carve the popup into list, preview and footer rows.
///
/// The footer is taken from the bottom in priority order (legend, path,
/// diagnostics) and each part is only claimed while the list keeps at least one
/// row: on a tiny terminal the list stays usable and the chrome disappears,
/// which is the spec's "Liste bleibt bedienbar" rule. What is left over is then
/// split between list and preview by [`split_body`].
///
/// This is the single place picker geometry is decided, so `visible_rows` — and
/// with it the PageUp/PageDown step — follows the preview automatically.
pub(crate) fn plan_theme_picker_layout(area: Rect) -> ThemePickerLayout {
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

    let body = Rect::new(inner.x, inner.y, inner.width, remaining);
    let (mode, list, preview, notice) = split_body(body);
    ThemePickerLayout {
        mode,
        popup,
        list,
        preview,
        notice,
        path,
        diagnostics,
        legend,
    }
}

/// Split what the footer left over between the list and the preview.
///
/// The three regimes are the spec's: side by side while both minimum widths
/// fit, stacked while the width alone does, and otherwise list-only with a size
/// hint. Each regime is a hard threshold rather than a shrinking preview,
/// because a preview too small for the roles the spec enumerates would be
/// misleading in a way a hidden one is not.
fn split_body(body: Rect) -> (ThemePickerLayoutMode, Rect, Option<Rect>, Option<Rect>) {
    if body.width >= MIN_LIST_W + PREVIEW_GAP + MIN_PREVIEW_W && body.height >= MIN_PREVIEW_H {
        // 45% for the preview, but never so much that the list drops below its
        // own minimum — the entry condition guarantees the clamp stays >= MIN_PREVIEW_W.
        let preview_w = (body.width * 45 / 100)
            .max(MIN_PREVIEW_W)
            .min(body.width - PREVIEW_GAP - MIN_LIST_W);
        let list_w = body.width - preview_w - PREVIEW_GAP;
        return (
            ThemePickerLayoutMode::SideBySide,
            Rect::new(body.x, body.y, list_w, body.height),
            Some(Rect::new(
                body.x + list_w + PREVIEW_GAP,
                body.y,
                preview_w,
                body.height,
            )),
            None,
        );
    }

    if body.width >= MIN_PREVIEW_W && body.height >= MIN_STACKED_LIST_H + MIN_PREVIEW_H {
        // The preview takes exactly what it needs; every further row is the
        // list's, because that is the half the user is actually navigating.
        let list_h = body.height - MIN_PREVIEW_H;
        return (
            ThemePickerLayoutMode::Stacked,
            Rect::new(body.x, body.y, body.width, list_h),
            Some(Rect::new(
                body.x,
                body.y + list_h,
                body.width,
                MIN_PREVIEW_H,
            )),
            None,
        );
    }

    // The list keeps at least one row, so the hint only gets one once there are
    // two — same rule the footer chrome above is carved with.
    let notice = (body.height >= 2).then(|| Rect::new(body.x, body.bottom() - 1, body.width, 1));
    let list_h = body.height - u16::from(notice.is_some());
    (
        ThemePickerLayoutMode::ListOnly,
        Rect::new(body.x, body.y, body.width, list_h),
        None,
        notice,
    )
}

/// How many list rows fit — the page step for PageUp/PageDown.
pub(crate) fn visible_rows(area: Rect) -> usize {
    plan_theme_picker_layout(area).list.height as usize
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
        app.theme_registry()
            .diagnostics()
            .iter()
            .map(|d| d.message.clone()),
    );
    lines
}

/// Interior of a preview box: inside the frame plus one padding column, which
/// is the cell that shows the box fill colour next to the text.
fn preview_content(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(1),
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
    )
}

/// One content row of a preview box, or `None` when the box is that short.
fn content_row(content: Rect, index: u16) -> Option<Rect> {
    (index < content.height).then(|| Rect::new(content.x, content.y + index, content.width, 1))
}

/// The two framed boxes of the preview.
pub(crate) struct PreviewBoxes {
    pub top: Rect,
    pub bottom: Rect,
}

/// Place the two boxes inside the preview rect.
///
/// The bottom box is anchored to the bottom edge and the top box is capped, so
/// the strip left between them grows with the terminal — and that strip is
/// exactly what shows off the background colour the spec asks the preview to
/// demonstrate.
pub(crate) fn plan_preview_boxes(area: Rect) -> PreviewBoxes {
    let top_h = TOP_BOX_H.min(area.height.saturating_sub(BOTTOM_BOX_H + 1));
    PreviewBoxes {
        top: Rect::new(area.x, area.y, area.width, top_h),
        bottom: Rect::new(
            area.x,
            area.bottom().saturating_sub(BOTTOM_BOX_H),
            area.width,
            BOTTOM_BOX_H.min(area.height),
        ),
    }
}

/// The four text roles the preview must show, with the column each starts in.
///
/// The sample tables are shared by the renderer and its tests: an assertion
/// then cannot drift away from what was actually drawn.
fn text_samples(theme: &ResolvedTheme) -> [(u16, &'static str, Style); 4] {
    [
        (0, "normal", theme.style(StyleRole::TextPrimary)),
        (7, "bright", theme.style(StyleRole::TextBright)),
        (14, "dim", theme.style(StyleRole::TextDim)),
        (18, "marked", theme.style(StyleRole::SelectionActive)),
    ]
}

fn tab_samples(theme: &ResolvedTheme) -> [(u16, &'static str, Style); 2] {
    [
        (0, "session-1", theme.style(StyleRole::TabsActive)),
        (10, "session-2", theme.style(StyleRole::TabsInactive)),
    ]
}

/// `up`/`warning`/`error` as text *and* colour, which is what the spec asks
/// for: a status the user can read even where the colour does not survive.
fn status_samples(theme: &ResolvedTheme) -> [(u16, &'static str, Style); 3] {
    [
        (
            0,
            "up",
            Style::default().fg(theme.color(ColorRole::StatusSuccess)),
        ),
        (
            3,
            "warning",
            Style::default().fg(theme.color(ColorRole::StatusWarning)),
        ),
        (
            11,
            "error",
            Style::default().fg(theme.color(ColorRole::StatusError)),
        ),
    ]
}

/// A bright key next to a dimmer label — the footer's contrast pair.
fn footer_samples(theme: &ResolvedTheme) -> [(u16, &'static str, Style); 2] {
    [
        (0, "^k", theme.style(StyleRole::FooterKey)),
        (3, "keys", theme.style(StyleRole::FooterLabel)),
    ]
}

/// Blank every cell of `area`, optionally with a background colour.
fn blank(buf: &mut Buffer, area: Rect, style: Style) {
    let width = area.width as usize;
    if width == 0 {
        return;
    }
    let spaces = " ".repeat(width);
    for y in area.y..area.bottom() {
        buf.set_stringn(area.x, y, &spaces, width, style);
    }
}

/// Fill `area` with a paint role, gradient included.
///
/// A gradient background goes through [`paint_gradient_area`] so every cell is
/// sampled at its own position; reading one colour with
/// `ResolvedTheme::paint_color_at` would flatten the whole surface to whatever
/// the top-left corner happened to be. Solid roles never reach the painter, so
/// the common case stays a plain blanking pass.
///
/// The painter is handed no exclusions because the picker overlay never
/// overlaps a remote PTY viewport — a background pass that can reach one must
/// pass the shared protected-PTY rect instead.
fn fill_paint(buf: &mut Buffer, area: Rect, theme: &ResolvedTheme, role: PaintRole) {
    match theme.paint(role) {
        ResolvedPaint::Solid(color) => blank(buf, area, Style::default().bg(*color)),
        ResolvedPaint::Gradient(_) => {
            // Blank first: the painter recolours cells, it does not clear them.
            blank(buf, area, Style::default());
            if let Some(gradient) = theme.paint_gradient(role) {
                paint_gradient_area(
                    buf,
                    area,
                    gradient,
                    PaintChannel::Background,
                    CellSelection::All,
                    &[],
                );
            }
        }
    }
}

/// Draw a box frame with `set_stringn` rather than a ratatui `Block`.
///
/// The picker's invariant is that every write is clipped by the width it is
/// given; a widget's own out-of-area handling is not something this module gets
/// to assert on.
fn draw_frame(buf: &mut Buffer, area: Rect, style: Style) {
    let width = area.width as usize;
    if width < 2 || area.height < 2 {
        return;
    }
    let horizontal = "\u{2500}".repeat(width - 2);
    buf.set_stringn(
        area.x,
        area.y,
        format!("\u{250c}{horizontal}\u{2510}"),
        width,
        style,
    );
    buf.set_stringn(
        area.x,
        area.bottom() - 1,
        format!("\u{2514}{horizontal}\u{2518}"),
        width,
        style,
    );
    for y in area.y + 1..area.bottom() - 1 {
        buf.set_stringn(area.x, y, "\u{2502}", 1, style);
        buf.set_stringn(area.right() - 1, y, "\u{2502}", 1, style);
    }
}

fn draw_samples(buf: &mut Buffer, row: Rect, samples: &[(u16, &'static str, Style)]) {
    for (offset, text, style) in samples {
        if *offset >= row.width {
            break;
        }
        let remaining = (row.width - offset) as usize;
        buf.set_stringn(row.x + offset, row.y, text, remaining, *style);
    }
}

/// One framed preview box: background, solid fallback frame, gradient ring,
/// then the title.
///
/// The order is the panel pipeline's, and it is what keeps the title
/// independently styled: the ring runs before it, so it never repaints it.
fn render_preview_box(
    buf: &mut Buffer,
    area: Rect,
    theme: &ResolvedTheme,
    title: &str,
    border: PaintRole,
    background: PaintRole,
    title_style: Style,
) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    fill_paint(buf, area, theme, background);
    let solid = theme.paint_color_at(border, area, area.x, area.y);
    draw_frame(buf, area, Style::default().fg(solid));
    // Solid-colour themes never reach the painter at all — `default` and
    // `high-contrast` have no gradients and must keep the cheap path.
    //
    // A ring takes no exclusions by design; that is safe here only because the
    // picker overlay never overlaps a remote PTY viewport.
    if let Some(gradient) = theme.paint_gradient(border) {
        paint_gradient_ring(buf, area, gradient);
    }
    let width = (area.width as usize).saturating_sub(2);
    buf.set_stringn(area.x + 2, area.y, title, width, title_style);
}

/// The upper box: text roles, the divider and the session tabs.
fn render_preview_chrome(buf: &mut Buffer, content: Rect, theme: &ResolvedTheme) {
    if let Some(row) = content_row(content, 0) {
        draw_samples(buf, row, &text_samples(theme));
    }
    if let Some(row) = content_row(content, 1) {
        let color = theme.paint_color_at(PaintRole::SeparatorPrimary, row, row.x, row.y);
        buf.set_stringn(
            row.x,
            row.y,
            "\u{2500}".repeat(row.width as usize),
            row.width as usize,
            Style::default().fg(color),
        );
        if let Some(gradient) = theme.paint_gradient(PaintRole::SeparatorPrimary) {
            paint_gradient_line(
                buf,
                row,
                gradient,
                PaintChannel::Foreground,
                CellSelection::All,
            );
        }
    }
    if let Some(row) = content_row(content, 2) {
        draw_samples(buf, row, &tab_samples(theme));
    }
    if let Some(row) = content_row(content, 3) {
        draw_samples(buf, row, &status_samples(theme));
    }
}

/// The lower box: a selected host row, a plain one and the footer key pair.
fn render_preview_hosts(buf: &mut Buffer, content: Rect, theme: &ResolvedTheme) {
    if let Some(row) = content_row(content, 0) {
        let selected = theme.style(StyleRole::DashboardHostListHostSelected);
        let width = row.width as usize;
        buf.set_stringn(row.x, row.y, " ".repeat(width), width, selected);
        buf.set_stringn(row.x, row.y, "web-01", width, selected);
    }
    if let Some(row) = content_row(content, 1) {
        draw_samples(
            buf,
            row,
            &[(0, "db-02", theme.style(StyleRole::DashboardHostListHost))],
        );
    }
    if let Some(row) = content_row(content, 2) {
        draw_samples(buf, row, &footer_samples(theme));
    }
}

/// The live two-box preview, drawn against the theme currently painting.
fn render_theme_preview(buf: &mut Buffer, area: Rect, theme: &ResolvedTheme) {
    // The whole surface is the app background; the boxes sit on top of it, so
    // the strip between them is where that colour stays visible.
    fill_paint(buf, area, theme, PaintRole::AppBackground);
    let boxes = plan_preview_boxes(area);
    render_preview_box(
        buf,
        boxes.top,
        theme,
        " Theme preview ",
        PaintRole::DashboardHostListBorderFocused,
        PaintRole::DashboardHostListBackground,
        theme.style(StyleRole::DashboardHostListTitle),
    );
    render_preview_box(
        buf,
        boxes.bottom,
        theme,
        " Host details ",
        PaintRole::DashboardDetailsBorder,
        PaintRole::DashboardDetailsBackground,
        theme.style(StyleRole::DashboardDetailsTitle),
    );
    render_preview_chrome(buf, preview_content(boxes.top), theme);
    render_preview_hosts(buf, preview_content(boxes.bottom), theme);
}

/// The hint that replaces a preview there is no room for.
fn preview_notice() -> String {
    format!("preview hidden \u{2014} needs {MIN_PREVIEW_W}x{MIN_PREVIEW_H}")
}

pub fn render(frame: &mut Frame, app: &App) {
    let areas = plan_theme_picker_layout(frame.area());
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

    // `app.theme()` is the previewed theme: navigation already activated it, so
    // an invalid selection leaves the last valid theme painting here too.
    //
    // The mode is the decision; `preview` and `notice` are only the geometry it
    // produced, and either can still be absent on a terminal with no room left.
    match areas.mode {
        ThemePickerLayoutMode::SideBySide | ThemePickerLayoutMode::Stacked => {
            if let Some(preview) = areas.preview {
                render_theme_preview(buf, preview, app.theme());
            }
        }
        ThemePickerLayoutMode::ListOnly => {
            if let Some(notice) = areas.notice {
                buf.set_stringn(
                    notice.x,
                    notice.y,
                    preview_notice(),
                    notice.width as usize,
                    app.theme().style(StyleRole::PopupHint),
                );
            }
        }
    }

    let selected_row = rows.get(selected);
    if let Some(path) = areas.path {
        // Show where new files go: the selected user theme's own path, or the
        // themes directory itself.
        let text = match selected_row.and_then(|row| row.path.as_ref()) {
            Some(path) => path.display().to_string(),
            None => match app.themes_dir() {
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
    use tempfile::TempDir;

    struct NoHosts;

    impl HostResolver for NoHosts {
        fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }
        fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
            Ok(SshHost::new(name))
        }
    }

    /// An app on the built-ins-only manager, still in Settings.
    fn base_app() -> App {
        App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(NoHosts),
                metadata: Arc::new(MetadataDb::default()),
                store: Arc::new(LauncherStore::open_in_memory().unwrap()),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        )
    }

    /// Walk the real entry point — Enter on the Settings theme row — so the
    /// renderer is never handed a state the app cannot actually produce.
    fn open_picker(app: &mut App) {
        app.mode = AppMode::Settings;
        app.settings_selected = 0;
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::empty(),
        ))
        .unwrap();
        assert_eq!(app.mode, AppMode::ThemePicker);
    }

    fn picker_app() -> App {
        let mut app = base_app();
        open_picker(&mut app);
        app
    }

    /// A picker previewing the built-in theme `id`.
    fn picker_app_previewing(id: &str) -> App {
        let mut app = picker_app();
        let index = app
            .theme_picker_rows()
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("no theme row for `{id}`"));
        app.select_theme_row(index);
        assert_eq!(app.theme().id().as_str(), id, "the preview must be live");
        app
    }

    /// A picker whose selected row is a user theme that is valid but carries an
    /// unknown component role — `warning` in Compatible mode.
    ///
    /// The `TempDir` is returned rather than dropped: a warning record can only
    /// come from a file, and the directory has to outlive the render. Nothing
    /// outside it is touched.
    fn warning_picker_app() -> (App, TempDir) {
        user_theme_app(
            "warned",
            // An unknown role inside a *known* family: the validator reports
            // the full path, which is the string the user has to go and fix.
            "schema_version = 1\nname = \"Warned\"\nextends = \"default\"\n\n\
             [components.footer]\nglow = \"semantic.accent\"\n",
        )
    }

    /// A picker previewing a user theme whose *backgrounds* are gradients.
    ///
    /// None of the five built-ins does that, so this is the only way to catch a
    /// fill that samples one colour instead of running the painter.
    fn gradient_background_app() -> (App, TempDir) {
        let (app, dir) = user_theme_app(
            "washed",
            "schema_version = 1\nname = \"Washed\"\nextends = \"default\"\n\n\
             [gradients.wash]\ndirection = \"horizontal\"\n\
             stops = [ { at = 0.0, color = \"#000000\" }, { at = 1.0, color = \"#ffffff\" } ]\n\n\
             [components.app]\nbackground = { gradient = \"gradients.wash\" }\n\n\
             [components.dashboard.host_list]\nbackground = { gradient = \"gradients.wash\" }\n",
        );
        assert_eq!(
            app.theme().id().as_str(),
            "washed",
            "the preview must be live"
        );
        (app, dir)
    }

    /// A picker with exactly one user theme file, already selected.
    fn user_theme_app(id: &str, body: &str) -> (App, TempDir) {
        let root = tempfile::tempdir().unwrap();
        let themes = root.path().join("themes");
        std::fs::create_dir(&themes).unwrap();
        std::fs::write(themes.join(format!("{id}.toml")), body).unwrap();
        let mut app = base_app();
        app.load_themes_from(&themes);
        open_picker(&mut app);
        let index = app
            .theme_picker_rows()
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("`{id}` is not listed"));
        app.select_theme_row(index);
        (app, root)
    }

    fn draw(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn render_picker(id: &str, width: u16, height: u16) -> ratatui::buffer::Buffer {
        draw(&picker_app_previewing(id), width, height)
    }

    fn line(buffer: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
        (0..width).map(|x| buffer[(x, y)].symbol()).collect()
    }

    /// One row of a rect, read at exactly the columns that rect owns.
    fn row_text(buffer: &ratatui::buffer::Buffer, rect: Rect, y: u16) -> String {
        (rect.x..rect.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    /// The preview rect and its two boxes for a terminal of this size.
    fn preview_of(width: u16, height: u16) -> (Rect, PreviewBoxes) {
        let areas = plan_theme_picker_layout(Rect::new(0, 0, width, height));
        let preview = areas.preview.expect("a preview at this size");
        let boxes = plan_preview_boxes(preview);
        (preview, boxes)
    }

    /// Every rect the layout claims must lie inside the terminal — ratatui
    /// silently drops out-of-area writes, so containment has to be asserted on
    /// the geometry rather than inferred from a draw that did not panic.
    fn assert_rects_inside(areas: &ThemePickerLayout, area: Rect, label: &str) {
        let mut rects = vec![("popup", areas.popup), ("list", areas.list)];
        rects.extend(areas.preview.map(|r| ("preview", r)));
        rects.extend(areas.notice.map(|r| ("notice", r)));
        rects.extend(areas.path.map(|r| ("path", r)));
        rects.push(("diagnostics", areas.diagnostics));
        rects.extend(areas.legend.map(|r| ("legend", r)));
        for (name, rect) in rects {
            assert!(rect.right() <= area.right(), "{label}: {name} right");
            assert!(rect.bottom() <= area.bottom(), "{label}: {name} bottom");
            assert!(
                rect.x >= area.x && rect.y >= area.y,
                "{label}: {name} origin"
            );
        }
        if let Some(preview) = areas.preview {
            let boxes = plan_preview_boxes(preview);
            for (name, rect) in [("top box", boxes.top), ("bottom box", boxes.bottom)] {
                assert!(
                    rect.right() <= preview.right() && rect.bottom() <= preview.bottom(),
                    "{label}: {name} escapes the preview"
                );
            }
        }
    }

    // ---------------------------------------------------------------- geometry

    #[test]
    fn wide_layout_places_list_left_and_preview_right() {
        let layout = plan_theme_picker_layout(Rect::new(0, 0, 120, 36));
        assert_eq!(layout.mode, ThemePickerLayoutMode::SideBySide);
        let preview = layout.preview.expect("wide preview");
        assert!(layout.list.right() <= preview.x);
        assert!(preview.height >= 12);
    }

    #[test]
    fn tiny_layout_keeps_the_list_and_hides_preview() {
        let layout = plan_theme_picker_layout(Rect::new(0, 0, 30, 8));
        assert_eq!(layout.mode, ThemePickerLayoutMode::ListOnly);
        assert!(layout.preview.is_none());
        assert!(layout.notice.is_some());
    }

    /// A terminal too narrow for two columns but tall enough for both stacks
    /// them, and the preview must sit directly under the list.
    #[test]
    fn narrow_layout_stacks_the_preview_under_the_list() {
        let layout = plan_theme_picker_layout(Rect::new(0, 0, 60, 36));
        assert_eq!(layout.mode, ThemePickerLayoutMode::Stacked);
        let preview = layout.preview.expect("stacked preview");
        assert_eq!(preview.x, layout.list.x, "stacked shares the left edge");
        assert_eq!(preview.width, layout.list.width);
        assert_eq!(preview.y, layout.list.bottom(), "no gap, no overlap");
        assert_eq!(preview.height, MIN_PREVIEW_H);
        assert!(layout.list.height >= MIN_STACKED_LIST_H);
        assert!(layout.notice.is_none());
    }

    /// The preview may never eat the footer chrome the list already carved.
    #[test]
    fn the_preview_never_overlaps_the_footer_rows() {
        for (w, h) in [(120, 36), (100, 30), (60, 36)] {
            let areas = plan_theme_picker_layout(Rect::new(0, 0, w, h));
            let preview = areas.preview.expect("a preview at this size");
            assert!(
                preview.bottom() <= areas.diagnostics.y,
                "{w}x{h}: preview runs into the diagnostics"
            );
            for chrome in [areas.path, areas.legend].into_iter().flatten() {
                assert!(preview.bottom() <= chrome.y, "{w}x{h}: preview hits chrome");
            }
        }
    }

    /// `visible_rows` — and with it the PageUp/PageDown step — has to follow the
    /// preview automatically, because both read the same pure function.
    #[test]
    fn the_page_step_follows_the_shrunken_list() {
        let area = Rect::new(0, 0, 60, 36);
        let areas = plan_theme_picker_layout(area);
        assert_eq!(areas.mode, ThemePickerLayoutMode::Stacked);
        assert_eq!(visible_rows(area), areas.list.height as usize);
        assert!(
            visible_rows(area) < visible_rows(Rect::new(0, 0, 120, 36)),
            "a stacked preview must cost list rows"
        );
    }

    /// The smallest terminal there is. Nothing may be planned outside it.
    #[test]
    fn renders_at_one_by_one() {
        let app = picker_app();
        let area = Rect::new(0, 0, 1, 1);
        let areas = plan_theme_picker_layout(area);
        assert_rects_inside(&areas, area, "1x1");
        assert_eq!(areas.mode, ThemePickerLayoutMode::ListOnly);
        assert!(areas.preview.is_none());
        assert!(areas.notice.is_none(), "the list keeps the only row");
        let buffer = draw(&app, 1, 1);
        assert_eq!(buffer.area().width, 1);
        assert_eq!(buffer.area().height, 1);
    }

    /// Every degenerate size the spec names must survive, and the layout must
    /// never claim a rect outside the terminal — preview and notice included.
    #[test]
    fn tiny_terminals_keep_the_layout_inside_the_area() {
        let app = picker_app();
        for (w, h) in [
            (1, 1),
            (1, 24),
            (80, 1),
            (2, 3),
            (5, 5),
            (20, 5),
            (40, 10),
            (49, 17),
            (60, 36),
            (100, 30),
            (120, 36),
        ] {
            let area = Rect::new(0, 0, w, h);
            let areas = plan_theme_picker_layout(area);
            assert_rects_inside(&areas, area, &format!("{w}x{h}"));
            // A popup with any interior at all must keep the list a row; a
            // two-row popup has no interior to give.
            if areas.popup.width > 2 && areas.popup.height > 2 {
                assert!(areas.list.height >= 1, "{w}x{h}: the list must stay usable");
            }
            draw(&app, w, h);
        }
    }

    /// The three sizes the contract names, drawn with a gradient theme so the
    /// painters run too.
    #[test]
    fn the_named_degenerate_sizes_draw_without_writing_outside() {
        let app = picker_app_previewing("fire");
        for (w, h) in [(1, 1), (20, 5), (40, 10)] {
            let area = Rect::new(0, 0, w, h);
            assert_rects_inside(&plan_theme_picker_layout(area), area, &format!("{w}x{h}"));
            let buffer = draw(&app, w, h);
            assert_eq!(buffer.area(), &area);
        }
    }

    /// When the preview is hidden the user is told why, at the exact row the
    /// layout reserved for it.
    #[test]
    fn the_hidden_preview_is_explained_on_its_own_row() {
        let app = picker_app();
        let area = Rect::new(0, 0, 40, 10);
        let areas = plan_theme_picker_layout(area);
        assert_eq!(areas.mode, ThemePickerLayoutMode::ListOnly);
        let notice = areas.notice.expect("a hint row");
        let buffer = draw(&app, 40, 10);
        assert!(
            row_text(&buffer, notice, notice.y).starts_with("preview hidden"),
            "{:?}",
            row_text(&buffer, notice, notice.y)
        );
    }

    // ----------------------------------------------------------------- preview

    #[test]
    fn preview_contains_two_boxes_gradient_and_semantic_statuses() {
        let buffer = render_picker("fire", 100, 30);
        let (_, boxes) = preview_of(100, 30);
        assert!(
            row_text(&buffer, boxes.top, boxes.top.y).contains("Theme preview"),
            "top box title"
        );
        assert!(
            row_text(&buffer, boxes.bottom, boxes.bottom.y).contains("Host details"),
            "bottom box title"
        );

        let statuses = content_row(preview_content(boxes.top), 3).expect("the status row");
        let text = row_text(&buffer, statuses, statuses.y);
        assert!(text.starts_with("up warning error"), "{text:?}");

        assert_gradient_changes_along_top_border(&buffer, boxes.top);
    }

    /// A gradient frame must actually run a ramp along the top border.
    ///
    /// Read past the title, which is drawn *after* the ring on purpose and
    /// therefore carries its own colour.
    fn assert_gradient_changes_along_top_border(buffer: &ratatui::buffer::Buffer, area: Rect) {
        let colors: Vec<_> = (area.x + 18..area.right())
            .map(|x| buffer[(x, area.y)].fg)
            .collect();
        assert!(
            colors.windows(2).any(|pair| pair[0] != pair[1]),
            "the top border is flat: {colors:?}"
        );
    }

    /// A theme without gradients must take the solid path — same code, no ramp.
    #[test]
    fn a_solid_theme_paints_a_flat_frame() {
        let app = picker_app_previewing("default");
        assert!(
            app.theme()
                .paint_gradient(PaintRole::DashboardHostListBorderFocused)
                .is_none(),
            "`default` is the frozen solid baseline"
        );
        let buffer = draw(&app, 100, 30);
        let (_, boxes) = preview_of(100, 30);
        let top = boxes.top;
        let colors: Vec<_> = (top.x + 18..top.right())
            .map(|x| buffer[(x, top.y)].fg)
            .collect();
        assert!(
            colors.windows(2).all(|pair| pair[0] == pair[1]),
            "a solid frame must not ramp: {colors:?}"
        );
    }

    /// normal / bright / dim / marked, each at the column and style the shared
    /// sample table names.
    #[test]
    fn the_preview_shows_the_four_text_roles() {
        let app = picker_app_previewing("summer");
        let buffer = draw(&app, 100, 30);
        let (_, boxes) = preview_of(100, 30);
        let row = content_row(preview_content(boxes.top), 0).expect("the text row");
        for (offset, text, style) in text_samples(app.theme()) {
            let cell = &buffer[(row.x + offset, row.y)];
            assert_eq!(cell.symbol(), &text[0..1], "{text} is not at its column");
            assert_eq!(cell.fg, style.fg.unwrap(), "{text} foreground");
        }
        // The four must be visibly different roles, not four copies of `text`.
        let theme = app.theme();
        let marked = theme.style(StyleRole::SelectionActive);
        assert_ne!(marked.bg, theme.style(StyleRole::TextPrimary).bg);
        assert_ne!(
            theme.style(StyleRole::TextBright).fg,
            theme.style(StyleRole::TextDim).fg
        );
    }

    /// Active and inactive session tabs sit side by side in their own styles.
    #[test]
    fn the_preview_shows_active_and_inactive_session_tabs() {
        let app = picker_app_previewing("aqua");
        let buffer = draw(&app, 100, 30);
        let (_, boxes) = preview_of(100, 30);
        let row = content_row(preview_content(boxes.top), 2).expect("the tab row");
        let text = row_text(&buffer, row, row.y);
        assert!(text.starts_with("session-1 session-2"), "{text:?}");
        let [(active_x, _, active), (inactive_x, _, inactive)] = tab_samples(app.theme());
        assert_eq!(buffer[(row.x + active_x, row.y)].fg, active.fg.unwrap());
        assert_eq!(buffer[(row.x + inactive_x, row.y)].fg, inactive.fg.unwrap());
        assert_ne!(active.fg, inactive.fg, "the two tabs must differ");
    }

    /// A focused frame and an inactive one, each from its own paint role.
    #[test]
    fn the_two_boxes_use_the_focused_and_the_inactive_frame() {
        let app = picker_app_previewing("fire");
        let buffer = draw(&app, 100, 30);
        let (_, boxes) = preview_of(100, 30);
        let theme = app.theme();
        let focused = theme.paint_color_at(
            PaintRole::DashboardHostListBorderFocused,
            boxes.top,
            boxes.top.x,
            boxes.top.y,
        );
        let inactive = theme.paint_color_at(
            PaintRole::DashboardDetailsBorder,
            boxes.bottom,
            boxes.bottom.x,
            boxes.bottom.y,
        );
        assert_eq!(buffer[(boxes.top.x, boxes.top.y)].fg, focused);
        assert_eq!(buffer[(boxes.bottom.x, boxes.bottom.y)].fg, inactive);
        assert_ne!(focused, inactive, "focus must be visible");
    }

    /// The divider is a full-width line in the separator role.
    #[test]
    fn the_preview_draws_a_divider_under_the_text_sampler() {
        let app = picker_app_previewing("fire");
        let buffer = draw(&app, 100, 30);
        let (_, boxes) = preview_of(100, 30);
        let row = content_row(preview_content(boxes.top), 1).expect("the divider row");
        let text = row_text(&buffer, row, row.y);
        assert!(
            text.chars().all(|c| c == '\u{2500}'),
            "the divider is broken: {text:?}"
        );
        // `fire` puts a gradient on `components.separator.primary`, so the line
        // must ramp rather than sit on one colour.
        let colors: Vec<_> = (row.x..row.right())
            .map(|x| buffer[(x, row.y)].fg)
            .collect();
        assert!(
            colors.windows(2).any(|pair| pair[0] != pair[1]),
            "flat line"
        );
    }

    /// The selected host row carries the selection colours across its full
    /// width, next to a plain one.
    #[test]
    fn the_preview_shows_a_selected_and_a_plain_host_row() {
        let app = picker_app_previewing("summer");
        let buffer = draw(&app, 100, 30);
        let (_, boxes) = preview_of(100, 30);
        let content = preview_content(boxes.bottom);
        let selected = content_row(content, 0).expect("the selected host row");
        let plain = content_row(content, 1).expect("the plain host row");
        let style = app.theme().style(StyleRole::DashboardHostListHostSelected);
        assert!(row_text(&buffer, selected, selected.y).starts_with("web-01"));
        for x in selected.x..selected.right() {
            assert_eq!(
                buffer[(x, selected.y)].bg,
                style.bg.unwrap(),
                "the selection must span the row"
            );
        }
        assert!(row_text(&buffer, plain, plain.y).starts_with("db-02"));
        assert_eq!(
            buffer[(plain.x, plain.y)].fg,
            app.theme()
                .style(StyleRole::DashboardHostListHost)
                .fg
                .unwrap()
        );
    }

    /// A bright key next to a dimmer label.
    #[test]
    fn the_preview_shows_the_footer_key_label_contrast() {
        let app = picker_app_previewing("summer");
        let buffer = draw(&app, 100, 30);
        let (_, boxes) = preview_of(100, 30);
        let row = content_row(preview_content(boxes.bottom), 2).expect("the footer row");
        let [(key_x, _, key), (label_x, _, label)] = footer_samples(app.theme());
        assert!(row_text(&buffer, row, row.y).starts_with("^k keys"));
        assert_eq!(buffer[(row.x + key_x, row.y)].fg, key.fg.unwrap());
        assert_eq!(buffer[(row.x + label_x, row.y)].fg, label.fg.unwrap());
        assert_ne!(key.fg, label.fg, "key and label must contrast");
    }

    /// Background, box fill and the padding column are all painted rather than
    /// left on the terminal default.
    #[test]
    fn the_preview_paints_background_fill_and_padding() {
        let app = picker_app_previewing("summer");
        let buffer = draw(&app, 100, 30);
        let (preview, boxes) = preview_of(100, 30);
        let theme = app.theme();

        // The strip between the two boxes is the app background.
        let gap_y = boxes.top.bottom();
        assert!(gap_y < boxes.bottom.y, "the boxes must leave a strip");
        assert_eq!(
            buffer[(preview.x, gap_y)].bg,
            theme.paint_color_at(PaintRole::AppBackground, preview, preview.x, preview.y)
        );

        // Each box interior carries its own fill.
        let top_fill = theme.paint_color_at(
            PaintRole::DashboardHostListBackground,
            boxes.top,
            boxes.top.x,
            boxes.top.y,
        );
        let content = preview_content(boxes.top);
        assert_eq!(buffer[(content.x, content.y)].bg, top_fill);
        // The padding column is blank but coloured — it is the fill made visible.
        let padding = (content.x - 1, content.y);
        assert_eq!(buffer[padding].symbol(), " ");
        assert_eq!(buffer[padding].bg, top_fill);

        let bottom_fill = theme.paint_color_at(
            PaintRole::DashboardDetailsBackground,
            boxes.bottom,
            boxes.bottom.x,
            boxes.bottom.y,
        );
        let bottom_content = preview_content(boxes.bottom);
        assert_eq!(
            buffer[(bottom_content.x - 1, bottom_content.y + 2)].bg,
            bottom_fill
        );
    }

    /// Navigating to another theme repaints the preview with it — the preview
    /// is the live theme, not a snapshot taken when the picker opened.
    #[test]
    fn the_preview_follows_the_selection() {
        let (_, boxes) = preview_of(100, 30);
        let corner = (boxes.top.x, boxes.top.y);
        let default = draw(&picker_app_previewing("default"), 100, 30);
        let fire = draw(&picker_app_previewing("fire"), 100, 30);
        assert_ne!(
            default[corner].fg, fire[corner].fg,
            "the frame must follow the previewed theme"
        );
    }

    // ------------------------------------------------------------- diagnostics

    #[test]
    fn warning_theme_is_selectable_and_displays_ignored_roles() {
        let (app, _dir) = warning_picker_app();
        assert_eq!(
            app.theme().id().as_str(),
            "warned",
            "a warning theme must still preview"
        );
        let area = Rect::new(0, 0, 100, 30);
        let areas = plan_theme_picker_layout(area);
        let buffer = draw(&app, 100, 30);

        let rows = app.theme_picker_rows();
        let selected = app.theme_picker.as_ref().unwrap().selected;
        let offset = scroll_offset(selected, rows.len(), areas.list.height as usize);
        let row_y = areas.list.y + (selected - offset) as u16;
        let row = row_text(&buffer, areas.list, row_y);
        assert!(row.trim_end().ends_with("warning"), "{row:?}");

        let diagnostics = row_text(&buffer, areas.diagnostics, areas.diagnostics.y);
        assert!(
            diagnostics.contains("components.footer.glow"),
            "the ignored role must be named: {diagnostics:?}"
        );
    }

    /// The selection and the diagnostics are the same in all three regimes —
    /// only the geometry changes.
    #[test]
    fn all_three_modes_share_the_selected_row_and_diagnostics() {
        let (app, _dir) = warning_picker_app();
        let selected = app.theme_picker.as_ref().unwrap().selected;
        let rows = app.theme_picker_rows().len();
        let mut modes = Vec::new();
        for (w, h) in [(120, 36), (60, 36), (40, 10)] {
            let areas = plan_theme_picker_layout(Rect::new(0, 0, w, h));
            let buffer = draw(&app, w, h);
            modes.push(areas.mode);

            let offset = scroll_offset(selected, rows, areas.list.height as usize);
            let row_y = areas.list.y + (selected - offset) as u16;
            assert_eq!(
                buffer[(areas.list.x, row_y)].bg,
                theme::SEL_BG,
                "{w}x{h}: the selected row must be visible"
            );
            let diagnostics = row_text(&buffer, areas.diagnostics, areas.diagnostics.y);
            assert!(
                diagnostics.starts_with("unknown component role"),
                "{w}x{h}: {diagnostics:?}"
            );
        }
        assert_eq!(
            modes,
            vec![
                ThemePickerLayoutMode::SideBySide,
                ThemePickerLayoutMode::Stacked,
                ThemePickerLayoutMode::ListOnly,
            ]
        );
    }

    /// A save/reload failure takes the first diagnostics row, in the error
    /// colour, ahead of the row's own diagnostics.
    #[test]
    fn a_picker_error_is_shown_first_and_in_red() {
        let mut app = picker_app();
        // `r` on a manager that belongs to no directory is the one failure the
        // built-ins-only picker can produce without touching a filesystem.
        app.reload_theme_picker();
        let areas = plan_theme_picker_layout(Rect::new(0, 0, 100, 30));
        let buffer = draw(&app, 100, 30);
        let first = row_text(&buffer, areas.diagnostics, areas.diagnostics.y);
        assert!(!first.trim().is_empty(), "the error must be visible");
        assert_eq!(
            buffer[(areas.diagnostics.x, areas.diagnostics.y)].fg,
            theme::red().fg.unwrap()
        );
    }

    // ------------------------------------------------------------------- list

    /// Built-ins first, in their frozen order, at exact coordinates.
    #[test]
    fn the_list_starts_with_the_built_ins_in_order() {
        let app = picker_app();
        // Big enough that all five built-ins are on screen at once: on a
        // stacked layout the list is deliberately short.
        let buffer = draw(&app, 120, 40);
        let areas = plan_theme_picker_layout(Rect::new(0, 0, 120, 40));
        let list = areas.list;
        assert!(list.height >= 5, "the list must show all five built-ins");
        let names = ["Default", "Summer", "Aqua", "Fire", "High Contrast"];
        for (i, _) in names.iter().enumerate() {
            let cell = row_text(&buffer, list, list.y + i as u16);
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
        let list = plan_theme_picker_layout(Rect::new(0, 0, 80, 24)).list;
        assert_eq!(buffer[(list.x, list.y + 2)].bg, theme::SEL_BG);
        assert_ne!(buffer[(list.x, list.y)].bg, theme::SEL_BG);
    }

    /// The legend and the themes-directory line occupy the bottom rows.
    #[test]
    fn the_footer_shows_the_legend_and_the_theme_path() {
        let app = picker_app();
        let areas = plan_theme_picker_layout(Rect::new(0, 0, 80, 24));
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

    // ----------------------------------------------------- reachable geometry

    /// The size most people still run. The two-box preview is the whole point
    /// of this screen, so it may not be the one terminal that never sees it.
    #[test]
    fn the_default_eighty_by_twentyfour_terminal_gets_a_preview() {
        let areas = plan_theme_picker_layout(Rect::new(0, 0, 80, 24));
        assert_ne!(
            areas.mode,
            ThemePickerLayoutMode::ListOnly,
            "80x24 must show the preview"
        );
        assert!(areas.preview.is_some());
        assert!(
            areas.list.height >= PREFERRED_LIST_H,
            "the list stays usable"
        );
    }

    /// The thresholds pinned at the sizes that matter, so they stay a decision
    /// rather than a side effect of the popup percentage.
    #[test]
    fn common_terminal_sizes_get_the_mode_they_should() {
        for (w, h, expected) in [
            (80, 24, ThemePickerLayoutMode::Stacked),
            (40, 20, ThemePickerLayoutMode::Stacked),
            (60, 36, ThemePickerLayoutMode::Stacked),
            (100, 30, ThemePickerLayoutMode::SideBySide),
            (120, 40, ThemePickerLayoutMode::SideBySide),
            (120, 36, ThemePickerLayoutMode::SideBySide),
            // Genuinely too small: 32 interior columns cannot hold the preview,
            // and four body rows cannot hold either box.
            (34, 14, ThemePickerLayoutMode::ListOnly),
            (40, 10, ThemePickerLayoutMode::ListOnly),
            (30, 8, ThemePickerLayoutMode::ListOnly),
        ] {
            let areas = plan_theme_picker_layout(Rect::new(0, 0, w, h));
            assert_eq!(areas.mode, expected, "{w}x{h}");
        }
    }

    /// Stacking is the *fallback* for a cramped terminal, so it must not cost
    /// more vertical space than the wide layout it falls back from.
    ///
    /// One extra row is structural — the stacked list needs a row of its own —
    /// and anything beyond that would make the fallback the harder regime to
    /// reach, which is exactly backwards.
    #[test]
    fn stacking_never_costs_more_height_than_side_by_side() {
        let first_preview_at = |width: u16| {
            (1..80u16)
                .find(|h| {
                    plan_theme_picker_layout(Rect::new(0, 0, width, *h)).mode
                        != ThemePickerLayoutMode::ListOnly
                })
                .expect("some height shows a preview")
        };
        let wide = first_preview_at(120);
        let narrow = first_preview_at(60);
        assert!(
            narrow <= wide + 1,
            "narrow needs {narrow} rows but wide only {wide}"
        );
    }

    /// A gradient background must be sampled per cell by the Task 6 painter.
    /// Reading one colour at the rect's corner would flatten it.
    #[test]
    fn a_gradient_background_is_sampled_per_cell_not_flattened() {
        let (app, _dir) = gradient_background_app();
        let buffer = draw(&app, 100, 30);
        let (preview, boxes) = preview_of(100, 30);

        let content = preview_content(boxes.top);
        let fills: Vec<_> = (content.x..content.right())
            .map(|x| buffer[(x, content.y)].bg)
            .collect();
        assert!(
            fills.windows(2).any(|pair| pair[0] != pair[1]),
            "the box fill is flat: {fills:?}"
        );

        // The strip between the boxes is the app background, and it carries a
        // gradient in this theme too.
        let strip_y = boxes.top.bottom();
        assert!(strip_y < boxes.bottom.y, "the boxes must leave a strip");
        let strip: Vec<_> = (preview.x..preview.right())
            .map(|x| buffer[(x, strip_y)].bg)
            .collect();
        assert!(
            strip.windows(2).any(|pair| pair[0] != pair[1]),
            "the app background is flat: {strip:?}"
        );
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
