use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::theme::catalog::{ColorRole, StyleRole};

const MAX_VISIBLE_ROWS: usize = 12;

/// Fuzzy command-snippet picker, floated over the active session. Enter runs the
/// selected snippet in the PTY; Tab inserts it without a trailing newline.
pub fn render(frame: &mut Frame, app: &App) {
    let Some(state) = app.snippet_picker.as_ref() else {
        return;
    };

    let area = frame.area();
    let width = crate::tui::fit_popup(area.width * 70 / 100, 46, area.width.saturating_sub(2));
    let visible = state.results.len().clamp(1, MAX_VISIBLE_ROWS) as u16;
    // border-top + prompt + separator + rows + hint + border-bottom.
    let height = crate::tui::fit_popup(visible + 5, 6, area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);
    let popup = crate::tui::popup_open_rect(popup, app);

    let theme = app.theme();
    let legend = theme.style(StyleRole::PopupLegend);
    let prompt = Style::default().fg(theme.color(ColorRole::StatusSuccess));

    crate::tui::open_popup(frame, popup, theme);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(crate::tui::popup_border_style(theme, popup))
        .title(Span::styled(
            " run snippet ",
            theme.style(StyleRole::PopupTitle),
        ));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    crate::tui::paint_popup_border(frame, popup, theme);

    if inner.width == 0 || inner.height < 3 {
        return;
    }

    // ── prompt line ──────────────────────────────────────────
    let prompt_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let counter = format!("{}/{}", state.results.len(), app.snippets.len());
    let prompt_line = Line::from(vec![
        Span::styled(" \u{276f} ", prompt),
        Span::styled(
            state.query.clone(),
            theme.style(StyleRole::CommandPaletteQuery),
        ),
        Span::styled("\u{2588}", prompt),
    ]);
    frame.render_widget(Paragraph::new(prompt_line), prompt_area);
    let counter_w = counter.len() as u16 + 1;
    let counter_x = inner.x + inner.width.saturating_sub(counter_w);
    frame.render_widget(
        Paragraph::new(Span::styled(counter, legend)),
        Rect::new(counter_x, inner.y, counter_w, 1),
    );

    // ── separator ────────────────────────────────────────────
    let sep_area = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    frame.render_widget(
        Paragraph::new(Span::styled(
            "\u{2500}".repeat(inner.width as usize),
            legend,
        )),
        sep_area,
    );

    // ── result list ──────────────────────────────────────────
    let list_area = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(3),
    );

    if state.results.is_empty() {
        let msg = if app.snippets.is_empty() {
            "No snippets yet — press Shift+S on the dashboard to add one."
        } else {
            "No snippets match your search."
        };
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {msg}"), legend)),
            list_area,
        );
    } else {
        let items: Vec<ListItem> = state
            .results
            .iter()
            .filter_map(|&idx| app.snippets.get(idx))
            .map(|snippet| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!(" {}", snippet.name),
                        theme.style(StyleRole::PickerRow),
                    ),
                    Span::styled(format!("  {}", snippet.command), legend),
                ]))
            })
            .collect();
        let mut list_state = ListState::default();
        list_state.select(Some(state.selected.min(items.len().saturating_sub(1))));
        frame.render_stateful_widget(
            List::new(items).highlight_style(theme.style(StyleRole::PickerRowSelected)),
            list_area,
            &mut list_state,
        );
    }

    // ── hint ─────────────────────────────────────────────────
    let hint_area = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
    frame.render_widget(
        Paragraph::new(Span::styled(
            " \u{21b5} run · Tab insert (no newline) · Esc cancel",
            theme.style(StyleRole::PopupHint),
        )),
        hint_area,
    );
}
