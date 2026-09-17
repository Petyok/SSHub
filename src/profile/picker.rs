//! Profile picker used at startup and from the running application.
//! Owns no launcher database: CRUD operates on profile directories and
//! `state.toml` only. In-app management protects the active workspace.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::{ProfileRecord, ProfileState, RootDirs};
use crate::theme::catalog::{ColorRole, StyleRole};
use crate::theme::model::ResolvedTheme;

/// Result of one key event in the picker loop.
#[derive(Debug, PartialEq, Eq)]
pub enum PickerOutcome {
    /// Stay in the picker (internal state changed).
    Continue,
    /// Launch this profile.
    Launch(ProfileRecord),
    /// Cancel startup or close in-app management.
    Quit,
}

#[derive(Debug)]
enum View {
    List,
    Create {
        buf: String,
        cursor: usize,
    },
    Rename {
        id: String,
        buf: String,
        cursor: usize,
    },
    ConfirmDelete {
        id: String,
        name: String,
    },
}

pub struct ProfilePicker {
    state: ProfileState,
    roots: RootDirs,
    cursor: usize,
    view: View,
    message: Option<String>,
    error: Option<String>,
    active_id: Option<String>,
    close_on_create_cancel: bool,
}

impl ProfilePicker {
    /// Cursor starts on the last-used profile, falling back to the first.
    pub fn new(roots: RootDirs, state: ProfileState) -> Self {
        let cursor = state
            .last_used_record()
            .and_then(|rec| state.profiles.iter().position(|p| p.id == rec.id))
            .unwrap_or(0);
        Self {
            state,
            roots,
            cursor,
            view: View::List,
            message: None,
            error: None,
            active_id: None,
            close_on_create_cancel: false,
        }
    }

    /// Manage profiles without touching the running workspace. A single-profile
    /// installation opens creation directly; otherwise select the active profile.
    pub fn in_app(roots: RootDirs, state: ProfileState, active_id: String) -> Self {
        let mut picker = Self::new(roots, state);
        picker.cursor = picker
            .state
            .profiles
            .iter()
            .position(|record| record.id == active_id)
            .unwrap_or(0);
        picker.active_id = Some(active_id);
        if picker.profile_count() == 1 {
            picker.view = View::Create {
                buf: String::new(),
                cursor: 0,
            };
            picker.close_on_create_cancel = true;
        }
        picker
    }

    /// Surface application-level switch failures in the still-open manager.
    pub fn set_error(&mut self, message: String) {
        self.error = Some(message);
    }

    pub fn profile_count(&self) -> usize {
        self.state.profiles.len()
    }

    fn current(&self) -> Option<&ProfileRecord> {
        self.state
            .profiles
            .get(self.cursor.min(self.state.profiles.len().saturating_sub(1)))
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> anyhow::Result<PickerOutcome> {
        self.error = None;
        match &self.view {
            View::List => self.handle_list_key(key),
            View::Create { .. } => {
                let outcome = self.handle_input_key(key)?;
                Ok(outcome)
            }
            View::Rename { .. } => {
                let outcome = self.handle_input_key(key)?;
                Ok(outcome)
            }
            View::ConfirmDelete { .. } => self.handle_confirm_key(key),
        }
    }

    /// Paste edits names only; list and confirmation views never replay commands.
    pub fn handle_paste(&mut self, text: &str) {
        let (buf, cursor) = match &mut self.view {
            View::Create { buf, cursor } | View::Rename { buf, cursor, .. } => (buf, cursor),
            _ => return,
        };
        for ch in text.chars().filter(|ch| !ch.is_control()) {
            *cursor = crate::text_input::insert_at(buf, *cursor, ch);
            self.error = None;
        }
    }

    fn handle_list_key(&mut self, key: KeyEvent) -> anyhow::Result<PickerOutcome> {
        let count = self.state.profiles.len();
        match key.code {
            KeyCode::Esc => return Ok(PickerOutcome::Quit),
            KeyCode::Enter => {
                if let Some(record) = self.current().cloned() {
                    return Ok(PickerOutcome::Launch(record));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if count > 0 {
                    self.cursor = (self.cursor + count - 1) % count;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if count > 0 {
                    self.cursor = (self.cursor + 1) % count;
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let idx = c.to_digit(10).expect("ASCII digit") as usize - 1;
                if idx < count {
                    self.cursor = idx;
                }
            }
            KeyCode::Char('n') => {
                self.view = View::Create {
                    buf: String::new(),
                    cursor: 0,
                };
            }
            KeyCode::Char('r' | 'd')
                if self.current().is_some_and(|record| {
                    self.active_id.as_deref() == Some(record.id.as_str())
                }) =>
            {
                self.error = Some("cannot rename or delete the active profile".into());
            }
            KeyCode::Char('r') => {
                if let Some(record) = self.current().cloned() {
                    self.view = View::Rename {
                        id: record.id.clone(),
                        buf: record.name.clone(),
                        cursor: crate::text_input::char_len(&record.name),
                    };
                }
            }
            KeyCode::Char('d') => {
                if count <= 1 {
                    self.error = Some("cannot delete the last remaining profile".into());
                } else if let Some(record) = self.current().cloned() {
                    self.view = View::ConfirmDelete {
                        id: record.id.clone(),
                        name: record.name.clone(),
                    };
                }
            }
            _ => {}
        }
        Ok(PickerOutcome::Continue)
    }

    /// Shared Create / Rename editor. Only cancelling initial in-app creation
    /// closes the picker; other submit/cancel actions return to the list.
    fn handle_input_key(&mut self, key: KeyEvent) -> anyhow::Result<PickerOutcome> {
        // Extract current buffer state.
        let (is_create, id, mut buf, mut cursor) = match &self.view {
            View::Create { buf, cursor } => (true, String::new(), buf.clone(), *cursor),
            View::Rename { id, buf, cursor } => (false, id.clone(), buf.clone(), *cursor),
            _ => return Ok(PickerOutcome::Continue),
        };

        match key.code {
            KeyCode::Esc => {
                if is_create && self.close_on_create_cancel {
                    return Ok(PickerOutcome::Quit);
                }
                self.view = View::List;
                return Ok(PickerOutcome::Continue);
            }
            KeyCode::Enter => {
                let name = buf.trim().to_string();
                if is_create {
                    match super::create_profile(&self.roots, &mut self.state, &name) {
                        Ok(record) => {
                            self.message = Some(format!("created profile '{}'", record.name));
                            self.cursor = self
                                .state
                                .profiles
                                .iter()
                                .position(|p| p.id == record.id)
                                .unwrap_or(0);
                            self.view = View::List;
                            self.close_on_create_cancel = false;
                        }
                        Err(e) => {
                            self.error = Some(format!("{e:#}"));
                        }
                    }
                } else {
                    match super::rename_profile(&self.roots, &mut self.state, &id, &name) {
                        Ok(()) => {
                            self.message = Some(format!("renamed profile to '{name}'"));
                            if let Some(pos) = self.state.profiles.iter().position(|p| p.id == id) {
                                self.cursor = pos;
                            }
                            self.view = View::List;
                        }
                        Err(e) => {
                            self.error = Some(format!("{e:#}"));
                        }
                    }
                }
                return Ok(PickerOutcome::Continue);
            }
            KeyCode::Char(ch) => {
                cursor = crate::text_input::insert_at(&mut buf, cursor, ch);
            }
            KeyCode::Backspace => {
                cursor = crate::text_input::backspace_at(&mut buf, cursor);
            }
            other => {
                if crate::text_input::handle_cursor_key(other, &mut buf, &mut cursor).is_none() {
                    return Ok(PickerOutcome::Continue);
                }
            }
        }

        self.view = if is_create {
            View::Create { buf, cursor }
        } else {
            View::Rename { id, buf, cursor }
        };
        Ok(PickerOutcome::Continue)
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> anyhow::Result<PickerOutcome> {
        let (id, name) = match &self.view {
            View::ConfirmDelete { id, name } => (id.clone(), name.clone()),
            _ => return Ok(PickerOutcome::Continue),
        };
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                match super::delete_profile(&self.roots, &mut self.state, &id) {
                    Ok(()) => {
                        self.message = Some(format!("deleted profile '{name}'"));
                        if self.cursor >= self.state.profiles.len() && self.cursor > 0 {
                            self.cursor = self.state.profiles.len() - 1;
                        }
                    }
                    Err(e) => {
                        self.error = Some(format!("{e:#}"));
                    }
                }
                self.view = View::List;
            }
            _ => {
                self.view = View::List;
            }
        }
        Ok(PickerOutcome::Continue)
    }

    pub fn render(&self, frame: &mut Frame, theme: &ResolvedTheme) {
        let height = 9u16.saturating_add(u16::try_from(self.profile_count()).unwrap_or(u16::MAX));
        let area = popup_area(frame.area(), 46, height);
        frame.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(crate::tui::popup_border_style(theme, area))
            .title(Span::styled(
                " Select profile ",
                theme.style(StyleRole::PopupTitle),
            ))
            .title_alignment(Alignment::Center)
            .style(theme.style(StyleRole::TextPrimary));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();

        match &self.view {
            View::List => {
                // Reserve space for status and both legend rows. On very short
                // terminals prioritize one selected row over the legend.
                let status_rows = u16::from(self.error.is_some() || self.message.is_some());
                let visible_rows = usize::from(inner.height.saturating_sub(3 + status_rows).max(1));
                let start = self.cursor.saturating_sub(visible_rows - 1);
                for (i, record) in self
                    .state
                    .profiles
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(visible_rows)
                {
                    let marker = if i == self.cursor { "▸ " } else { "  " };
                    let style = if i == self.cursor {
                        theme.style(StyleRole::PickerRowSelected)
                    } else {
                        theme.style(StyleRole::PickerRow)
                    };
                    lines.push(Line::from(vec![
                        Span::styled(marker.to_string(), style),
                        Span::styled(format!("{}. {}", i + 1, record.name), style),
                        Span::styled(
                            if self.active_id.as_deref() == Some(record.id.as_str()) {
                                " (active)"
                            } else {
                                ""
                            },
                            style,
                        ),
                    ]));
                }
            }
            View::Create { buf, cursor } => {
                lines.push(Line::from(Span::styled(
                    "New profile name:",
                    theme.style(StyleRole::TextBright),
                )));
                lines.push(input_line(buf, *cursor, theme));
            }
            View::Rename { buf, cursor, .. } => {
                lines.push(Line::from(Span::styled(
                    "New name:",
                    theme.style(StyleRole::TextBright),
                )));
                lines.push(input_line(buf, *cursor, theme));
            }
            View::ConfirmDelete { name, .. } => {
                lines.push(Line::from(Span::styled(
                    format!("Delete profile '{name}' permanently?"),
                    theme.style(StyleRole::PopupWarning),
                )));
                lines.push(Line::from(Span::styled(
                    "All its hosts, settings, and logs are removed.",
                    theme.style(StyleRole::TextMuted),
                )));
                lines.push(Line::from(vec![
                    Span::styled("y", theme.style(StyleRole::PopupError)),
                    Span::styled(" delete   ", theme.style(StyleRole::TextMuted)),
                    Span::styled("any other key", theme.style(StyleRole::TextBright)),
                    Span::styled(" keep", theme.style(StyleRole::TextMuted)),
                ]));
            }
        }

        if let Some(msg) = &self.error {
            lines.push(Line::from(Span::styled(
                msg.clone(),
                theme.style(StyleRole::PopupError),
            )));
        } else if let Some(msg) = &self.message {
            lines.push(Line::from(Span::styled(
                msg.clone(),
                ratatui::style::Style::default().fg(theme.color(ColorRole::StatusSuccess)),
            )));
        }

        if matches!(self.view, View::List) {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("↑↓/1-9", theme.style(StyleRole::FooterKey)),
                Span::styled(" select  ", theme.style(StyleRole::FooterLabel)),
                Span::styled("Enter", theme.style(StyleRole::FooterKey)),
                Span::styled(
                    if self.active_id.is_some() {
                        " switch"
                    } else {
                        " launch"
                    },
                    theme.style(StyleRole::FooterLabel),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("n", theme.style(StyleRole::FooterKey)),
                Span::styled(" new  ", theme.style(StyleRole::FooterLabel)),
                Span::styled("r", theme.style(StyleRole::FooterKey)),
                Span::styled(" rename  ", theme.style(StyleRole::FooterLabel)),
                Span::styled("d", theme.style(StyleRole::FooterKey)),
                Span::styled(" delete  ", theme.style(StyleRole::FooterLabel)),
                Span::styled("Esc", theme.style(StyleRole::FooterKey)),
                Span::styled(
                    if self.active_id.is_some() {
                        " close"
                    } else {
                        " quit"
                    },
                    theme.style(StyleRole::FooterLabel),
                ),
            ]));
        } else if !matches!(self.view, View::ConfirmDelete { .. }) {
            lines.push(Line::from(vec![
                Span::styled("Enter", theme.style(StyleRole::FooterKey)),
                Span::styled(" confirm  ", theme.style(StyleRole::FooterLabel)),
                Span::styled("Esc", theme.style(StyleRole::FooterKey)),
                Span::styled(" cancel", theme.style(StyleRole::FooterLabel)),
            ]));
        }

        let body = Paragraph::new(lines).style(theme.style(StyleRole::TextPrimary));
        frame.render_widget(body, inner);
        crate::tui::paint_popup_border(frame, area, theme);
    }
}

fn input_line(buf: &str, cursor: usize, theme: &ResolvedTheme) -> Line<'static> {
    let mut shown = String::from("> ");
    shown.push_str(buf);
    shown.push(' ');
    let cursor_pos = (crate::text_input::byte_index(&shown, cursor + 2)).min(shown.len());
    let mut spans = vec![Span::styled(
        shown.clone(),
        theme.style(StyleRole::FormInput),
    )];
    // Block cursor: invert the character under the cursor.
    if let Some(ch) = shown[cursor_pos..].chars().next() {
        spans.push(Span::styled(
            ch.to_string(),
            theme.style(StyleRole::PickerRowSelected),
        ));
    }
    Line::from(spans)
}

fn popup_area(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{self, ProfileState, RootDirs};

    fn roots(dir: &std::path::Path) -> RootDirs {
        RootDirs {
            data_root: dir.to_path_buf(),
            config_root: dir.to_path_buf(),
            compat: false,
        }
    }

    fn state_with(names: &[&str]) -> ProfileState {
        ProfileState {
            profiles: names
                .iter()
                .map(|n| ProfileRecord {
                    id: format!("id-{n}"),
                    name: (*n).to_string(),
                })
                .collect(),
            last_used: None,
        }
    }

    #[test]
    fn startup_picker_paste_preserves_unicode_and_editor_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let mut picker = ProfilePicker::new(roots(dir.path()), state_with(&["default", "work"]));
        let key = |code| KeyEvent::new(code, crossterm::event::KeyModifiers::empty());
        picker.handle_key(key(KeyCode::Char('n'))).unwrap();
        picker.handle_paste("Café");
        picker.handle_key(key(KeyCode::Left)).unwrap();
        picker.handle_paste("界\r\n\t\u{1b}\u{7f}\u{85}");
        assert!(matches!(
            &picker.view,
            View::Create { buf, cursor } if buf == "Caf界é" && *cursor == 4
        ));
        assert_eq!(picker.profile_count(), 2);
        picker.handle_key(key(KeyCode::Esc)).unwrap();
        picker.handle_paste("jndy\n");
        assert!(matches!(picker.view, View::List));
        assert_eq!(picker.cursor, 0);
    }

    #[test]
    fn cursor_starts_on_last_used_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state_with(&["default", "work"]);
        state.last_used = Some("id-work".into());
        let picker = ProfilePicker::new(roots(dir.path()), state);
        assert_eq!(picker.cursor, 1);
    }

    #[test]
    fn enter_launches_selected_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut picker = ProfilePicker::new(roots(dir.path()), state_with(&["default", "work"]));
        picker
            .handle_key(KeyEvent::new(
                KeyCode::Down,
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        let outcome = picker
            .handle_key(KeyEvent::new(
                KeyCode::Enter,
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        assert_eq!(
            outcome,
            PickerOutcome::Launch(ProfileRecord {
                id: "id-work".into(),
                name: "work".into(),
            })
        );
    }

    #[test]
    fn escape_quits() {
        let dir = tempfile::tempdir().unwrap();
        let mut picker = ProfilePicker::new(roots(dir.path()), state_with(&["default"]));
        let outcome = picker
            .handle_key(KeyEvent::new(
                KeyCode::Esc,
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        assert_eq!(outcome, PickerOutcome::Quit);
    }

    #[test]
    fn number_keys_select_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut picker = ProfilePicker::new(roots(dir.path()), state_with(&["a", "b", "c"]));
        picker
            .handle_key(KeyEvent::new(
                KeyCode::Char('3'),
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        assert_eq!(picker.cursor, 2);
    }

    #[test]
    fn create_rename_delete_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        let state = state_with(&["default"]);
        // Real profile directory for the seeded entry.
        std::fs::create_dir_all(dir.path().join("profiles/default")).unwrap();

        let mut picker = ProfilePicker::new(roots(dir.path()), state.clone());

        // Create via the input view.
        picker
            .handle_key(KeyEvent::new(
                KeyCode::Char('n'),
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        for ch in "work".chars() {
            picker
                .handle_key(KeyEvent::new(
                    KeyCode::Char(ch),
                    crossterm::event::KeyModifiers::empty(),
                ))
                .unwrap();
        }
        picker
            .handle_key(KeyEvent::new(
                KeyCode::Enter,
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        assert!(dir.path().join("profiles/work").exists());

        let state = ProfileState::load(dir.path()).unwrap().unwrap();
        assert_eq!(state.profiles.len(), 2);
        let work_id = state.by_name("work").unwrap().id.clone();

        // Rename: select "work" (cursor moved onto it), press r, change name.
        let mut picker = ProfilePicker::new(roots(dir.path()), state);
        picker
            .handle_key(KeyEvent::new(
                KeyCode::Down,
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        picker
            .handle_key(KeyEvent::new(
                KeyCode::Char('r'),
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        // Clear the buffer.
        for _ in 0..8 {
            picker
                .handle_key(KeyEvent::new(
                    KeyCode::Backspace,
                    crossterm::event::KeyModifiers::empty(),
                ))
                .unwrap();
        }
        for ch in "client".chars() {
            picker
                .handle_key(KeyEvent::new(
                    KeyCode::Char(ch),
                    crossterm::event::KeyModifiers::empty(),
                ))
                .unwrap();
        }
        picker
            .handle_key(KeyEvent::new(
                KeyCode::Enter,
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        assert!(dir.path().join("profiles/client").exists());
        assert!(!dir.path().join("profiles/work").exists());

        // Rename preserves the stable id.
        let state = ProfileState::load(dir.path()).unwrap().unwrap();
        let renamed = state.by_name("client").unwrap();
        assert_eq!(renamed.id, work_id);

        // Delete the renamed profile.
        let mut picker = ProfilePicker::new(roots(dir.path()), state);
        picker
            .handle_key(KeyEvent::new(
                KeyCode::Down,
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        picker
            .handle_key(KeyEvent::new(
                KeyCode::Char('d'),
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        picker
            .handle_key(KeyEvent::new(
                KeyCode::Char('y'),
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        assert!(!dir.path().join("profiles/client").exists());
        let state = ProfileState::load(dir.path()).unwrap().unwrap();
        assert_eq!(state.profiles.len(), 1);

        let _ = profile::validate_profile_name("sanity");
    }

    #[test]
    fn deleting_last_profile_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut picker = ProfilePicker::new(roots(dir.path()), state_with(&["default"]));
        picker
            .handle_key(KeyEvent::new(
                KeyCode::Char('d'),
                crossterm::event::KeyModifiers::empty(),
            ))
            .unwrap();
        assert!(picker.error.is_some());
        assert!(matches!(picker.view, View::List));
    }

    #[test]
    fn picker_renders_smoke() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let dir = tempfile::tempdir().unwrap();
        let mut state = state_with(&["default", "work"]);
        state.last_used = Some("id-work".into());
        let picker = ProfilePicker::new(roots(dir.path()), state);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = crate::test_support::resolved_default();
        terminal.draw(|frame| picker.render(frame, &theme)).unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("Select profile"));
        assert!(text.contains("default"));
        assert!(text.contains("work"));
    }

    #[test]
    fn picker_renders_on_tiny_terminals() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let dir = tempfile::tempdir().unwrap();
        let picker = ProfilePicker::new(roots(dir.path()), state_with(&["default"]));
        let theme = crate::test_support::resolved_default();
        for (width, height) in [(1, 1), (3, 2), (8, 4), (20, 6)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| picker.render(frame, &theme)).unwrap();
        }
    }

    fn press(picker: &mut ProfilePicker, code: KeyCode) -> PickerOutcome {
        picker
            .handle_key(KeyEvent::new(code, crossterm::event::KeyModifiers::empty()))
            .unwrap()
    }

    fn render_text(picker: &ProfilePicker, width: u16, height: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        let theme = crate::test_support::resolved_default();
        terminal.draw(|frame| picker.render(frame, &theme)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn in_app_single_profile_creates_second_then_lists() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let mut state = ProfileState::default();
        let active = profile::create_profile(&roots, &mut state, "default").unwrap();
        let mut picker = ProfilePicker::in_app(roots, state, active.id.clone());
        assert!(matches!(picker.view, View::Create { .. }));
        assert_eq!(picker.profile_count(), 1);
        for ch in "work".chars() {
            assert_eq!(
                press(&mut picker, KeyCode::Char(ch)),
                PickerOutcome::Continue
            );
        }
        assert_eq!(press(&mut picker, KeyCode::Enter), PickerOutcome::Continue);
        assert!(matches!(picker.view, View::List));
        assert_eq!(picker.profile_count(), 2);
        let saved = ProfileState::load(dir.path()).unwrap().unwrap();
        assert_eq!(saved.profiles.len(), 2);
        assert_eq!(saved.by_name("default").unwrap(), &active);
        let work = saved.by_name("work").unwrap();
        assert!(dir.path().join("profiles/work/config.toml").is_file());
        assert_eq!(
            press(&mut picker, KeyCode::Enter),
            PickerOutcome::Launch(work.clone())
        );
        press(&mut picker, KeyCode::Char('n'));
        assert_eq!(press(&mut picker, KeyCode::Esc), PickerOutcome::Continue);
        assert!(matches!(picker.view, View::List));
    }

    #[test]
    fn in_app_direct_create_cancel_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut picker = ProfilePicker::in_app(
            roots(dir.path()),
            state_with(&["default"]),
            "id-default".into(),
        );
        press(&mut picker, KeyCode::Char('w'));
        assert_eq!(press(&mut picker, KeyCode::Esc), PickerOutcome::Quit);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn in_app_active_profile_cannot_be_renamed_or_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let mut state = ProfileState::default();
        let active = profile::create_profile(&roots, &mut state, "default").unwrap();
        let other = profile::create_profile(&roots, &mut state, "work").unwrap();
        state.last_used = Some(other.id);
        let before = std::fs::read(dir.path().join(profile::STATE_FILE)).unwrap();
        let mut picker = ProfilePicker::in_app(roots, state, active.id.clone());
        assert_eq!(picker.current(), Some(&active));
        for code in [KeyCode::Char('r'), KeyCode::Char('d')] {
            assert_eq!(press(&mut picker, code), PickerOutcome::Continue);
            assert!(matches!(picker.view, View::List));
            assert!(render_text(&picker, 80, 24).contains("active profile"));
            assert_eq!(
                std::fs::read(dir.path().join(profile::STATE_FILE)).unwrap(),
                before
            );
            assert!(dir.path().join("profiles/default").is_dir());
        }
        press(&mut picker, KeyCode::Down);
        press(&mut picker, KeyCode::Char('r'));
        assert!(matches!(picker.view, View::Rename { .. }));
        press(&mut picker, KeyCode::Esc);
        press(&mut picker, KeyCode::Char('d'));
        assert!(matches!(picker.view, View::ConfirmDelete { .. }));
    }

    #[test]
    fn in_app_non_active_crud_preserves_active_profile() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let mut state = ProfileState::default();
        let active = profile::create_profile(&roots, &mut state, "default").unwrap();
        let other = profile::create_profile(&roots, &mut state, "work").unwrap();
        let active_config = dir.path().join("profiles/default/config.toml");
        let before = std::fs::read(&active_config).unwrap();
        let mut picker = ProfilePicker::in_app(roots, state, active.id.clone());

        press(&mut picker, KeyCode::Down);
        press(&mut picker, KeyCode::Char('r'));
        for _ in "work".chars() {
            press(&mut picker, KeyCode::Backspace);
        }
        for ch in "client".chars() {
            press(&mut picker, KeyCode::Char(ch));
        }
        assert_eq!(press(&mut picker, KeyCode::Enter), PickerOutcome::Continue);
        assert!(matches!(picker.view, View::List));
        assert_eq!(picker.profile_count(), 2);
        let saved = ProfileState::load(dir.path()).unwrap().unwrap();
        assert_eq!(saved.by_name("client").unwrap().id, other.id);
        assert_eq!(saved.by_name("default"), Some(&active));
        assert!(dir.path().join("profiles/client/config.toml").is_file());
        assert!(!dir.path().join("profiles/work").exists());

        press(&mut picker, KeyCode::Char('d'));
        assert!(matches!(picker.view, View::ConfirmDelete { .. }));
        let text = render_text(&picker, 80, 24);
        assert!(text.contains("Delete profile 'client' permanently?"));
        assert_eq!(
            press(&mut picker, KeyCode::Char('y')),
            PickerOutcome::Continue
        );
        assert!(matches!(picker.view, View::List));
        assert_eq!(picker.profile_count(), 1);
        assert_eq!(picker.current(), Some(&active));
        let saved = ProfileState::load(dir.path()).unwrap().unwrap();
        assert_eq!(saved.profiles, vec![active]);
        assert!(!dir.path().join("profiles/client").exists());
        assert_eq!(std::fs::read(&active_config).unwrap(), before);
    }

    #[test]
    fn in_app_render_shows_active_switch_close_and_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut picker = ProfilePicker::in_app(
            roots(dir.path()),
            state_with(&["default", "work"]),
            "id-work".into(),
        );
        picker.set_error("switch failed".into());
        let text = render_text(&picker, 80, 24);
        assert!(text.contains("work (active)"));
        assert!(text.contains("Enter switch"));
        assert!(text.contains("Esc close"));
        assert!(text.contains("switch failed"));
        for (width, height) in [(1, 1), (3, 2), (8, 4), (20, 6)] {
            render_text(&picker, width, height);
        }
    }

    #[test]
    fn long_list_keeps_selected_profile_visible() {
        let dir = tempfile::tempdir().unwrap();
        let names: Vec<String> = (0..30).map(|i| format!("profile-{i}")).collect();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut picker = ProfilePicker::in_app(
            roots(dir.path()),
            state_with(&name_refs),
            "id-profile-29".into(),
        );
        assert!(render_text(&picker, 80, 12).contains("profile-29 (active)"));
        press(&mut picker, KeyCode::Down);
        assert!(render_text(&picker, 80, 12).contains("1. profile-0"));
    }
}
