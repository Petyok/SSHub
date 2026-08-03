//! Startup profile picker — a standalone pre-`App` TUI shown between the
//! intro splash and the dashboard when more than one profile exists (or with
//! `--manage-profiles`). Owns no launcher database: CRUD operates on profile
//! directories and `state.toml` only.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::{ProfileRecord, ProfileState, RootDirs};
use crate::tui::theme;

/// Result of one key event in the picker loop.
#[derive(Debug, PartialEq, Eq)]
pub enum PickerOutcome {
    /// Stay in the picker (internal state changed).
    Continue,
    /// Launch this profile.
    Launch(ProfileRecord),
    /// Cancel startup cleanly.
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
        }
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

    /// Shared line-editor handling for Create / Rename views. Returns `Quit`
    /// never; `Launch` never; submit/cancel flip back to the list view.
    fn handle_input_key(&mut self, key: KeyEvent) -> anyhow::Result<PickerOutcome> {
        // Extract current buffer state.
        let (is_create, id, mut buf, mut cursor) = match &self.view {
            View::Create { buf, cursor } => (true, String::new(), buf.clone(), *cursor),
            View::Rename { id, buf, cursor } => (false, id.clone(), buf.clone(), *cursor),
            _ => return Ok(PickerOutcome::Continue),
        };

        match key.code {
            KeyCode::Esc => {
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

    /// Record `record` as last-used in `state.toml` (called on launch).
    pub fn mark_last_used(&self, record: &ProfileRecord) {
        let mut state = self.state.clone();
        state.last_used = Some(record.id.clone());
        let _ = state.save(&self.roots.data_root);
    }

    pub fn render(&self, frame: &mut Frame) {
        let area = popup_area(frame.area(), 46, 9 + self.state.profiles.len() as u16);
        frame.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::popup_border())
            .title(" Select profile ")
            .title_alignment(Alignment::Center)
            .style(theme::text());
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();

        match &self.view {
            View::List => {
                for (i, record) in self.state.profiles.iter().enumerate() {
                    let marker = if i == self.cursor { "▸ " } else { "  " };
                    let style = if i == self.cursor {
                        theme::selected()
                    } else {
                        theme::text()
                    };
                    lines.push(Line::from(vec![
                        Span::styled(marker.to_string(), style),
                        Span::styled(format!("{}. {}", i + 1, record.name), style),
                    ]));
                }
            }
            View::Create { buf, cursor } => {
                lines.push(Line::from(Span::styled(
                    "New profile name:",
                    theme::bright(),
                )));
                lines.push(input_line(buf, *cursor));
            }
            View::Rename { buf, cursor, .. } => {
                lines.push(Line::from(Span::styled("New name:", theme::bright())));
                lines.push(input_line(buf, *cursor));
            }
            View::ConfirmDelete { name, .. } => {
                lines.push(Line::from(Span::styled(
                    format!("Delete profile '{name}' permanently?"),
                    theme::amber(),
                )));
                lines.push(Line::from(Span::styled(
                    "All its hosts, settings, and logs are removed.",
                    theme::mute(),
                )));
                lines.push(Line::from(vec![
                    Span::styled("y", theme::red()),
                    Span::styled(" delete   ", theme::mute()),
                    Span::styled("any other key", theme::bright()),
                    Span::styled(" keep", theme::mute()),
                ]));
            }
        }

        if let Some(msg) = &self.error {
            lines.push(Line::from(Span::styled(msg.clone(), theme::red())));
        } else if let Some(msg) = &self.message {
            lines.push(Line::from(Span::styled(msg.clone(), theme::green())));
        }

        if matches!(self.view, View::List) {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("↑↓/1-9", theme::footer_key()),
                Span::styled(" select  ", theme::footer_label()),
                Span::styled("Enter", theme::footer_key()),
                Span::styled(" launch  ", theme::footer_label()),
                Span::styled("n", theme::footer_key()),
                Span::styled(" new  ", theme::footer_label()),
                Span::styled("r", theme::footer_key()),
                Span::styled(" rename  ", theme::footer_label()),
                Span::styled("d", theme::footer_key()),
                Span::styled(" delete  ", theme::footer_label()),
                Span::styled("Esc", theme::footer_key()),
                Span::styled(" quit", theme::footer_label()),
            ]));
        } else if !matches!(self.view, View::ConfirmDelete { .. }) {
            lines.push(Line::from(vec![
                Span::styled("Enter", theme::footer_key()),
                Span::styled(" confirm  ", theme::footer_label()),
                Span::styled("Esc", theme::footer_key()),
                Span::styled(" cancel", theme::footer_label()),
            ]));
        }

        let body = Paragraph::new(lines).style(theme::text());
        frame.render_widget(body, inner);
    }
}

fn input_line(buf: &str, cursor: usize) -> Line<'static> {
    let mut shown = String::from("> ");
    shown.push_str(buf);
    shown.push(' ');
    let cursor_pos = (crate::text_input::byte_index(&shown, cursor + 2)).min(shown.len());
    let mut spans = vec![Span::styled(shown.clone(), theme::bright())];
    // Block cursor: invert the character under the cursor.
    if let Some(ch) = shown[cursor_pos..].chars().next() {
        spans.push(Span::styled(ch.to_string(), theme::inv()));
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
        terminal.draw(|frame| picker.render(frame)).unwrap();
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
}
