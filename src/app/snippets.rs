use super::*;

use crate::store::{NewSnippet, Snippet, SnippetUpdate};
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};

impl App {
    /// Reload the snippet library from the store, keeping the manager selection
    /// on the same snippet by name where possible.
    pub(crate) fn reload_snippets(&mut self) -> Result<()> {
        let selected_name = self
            .snippets
            .get(self.snippet_manage_selected)
            .map(|s| s.name.clone());
        self.snippets = self.store.list_snippets()?;
        match selected_name.and_then(|name| self.snippets.iter().position(|s| s.name == name)) {
            Some(pos) => self.snippet_manage_selected = pos,
            None => self.clamp_snippet_manage_selected(),
        }
        Ok(())
    }

    pub(crate) fn clamp_snippet_manage_selected(&mut self) {
        self.snippet_manage_selected = self
            .snippet_manage_selected
            .min(self.snippets.len().saturating_sub(1));
        self.snap_snippet_manage_selection();
    }

    /// `snippets` indices the manager lists: all of them, or those matching
    /// the `/` filter, best first.
    pub fn snippet_manage_view(&self) -> Vec<usize> {
        rank_snippets(&self.snippets, &self.snippet_manage_query)
    }

    /// Keep the selection on a visible row after the filter or list changed.
    fn snap_snippet_manage_selection(&mut self) {
        let view = self.snippet_manage_view();
        if !view.contains(&self.snippet_manage_selected) {
            self.snippet_manage_selected = view.first().copied().unwrap_or(0);
        }
    }

    /// The highlighted snippet, or `None` when the filter hides every row.
    fn selected_managed_snippet(&self) -> Option<Snippet> {
        self.snippet_manage_view()
            .contains(&self.snippet_manage_selected)
            .then(|| self.snippets.get(self.snippet_manage_selected).cloned())
            .flatten()
    }

    /// Step the selection through the visible rows; saturates at both ends,
    /// so `isize::MIN` / `isize::MAX` jump to the first / last row.
    fn move_snippet_manage_selection(&mut self, delta: isize) {
        let view = self.snippet_manage_view();
        if view.is_empty() {
            return;
        }
        let pos = view
            .iter()
            .position(|&i| i == self.snippet_manage_selected)
            .unwrap_or(0) as isize;
        let next = pos.saturating_add(delta).clamp(0, view.len() as isize - 1);
        self.snippet_manage_selected = view[next as usize];
    }

    // ── Manager overlay ─────────────────────────────────────────

    pub(crate) fn enter_snippet_manage(&mut self) -> Result<()> {
        self.reload_snippets()?;
        self.snippet_notice = None;
        self.clamp_snippet_manage_selected();
        self.mode = AppMode::SnippetManage;
        Ok(())
    }

    fn close_snippet_manage(&mut self) {
        self.snippet_manage_query.clear();
        self.snippet_manage_filtering = false;
        self.mode = AppMode::Normal;
    }

    pub(crate) fn handle_key_snippet_manage(&mut self, key: KeyEvent) -> Result<()> {
        self.snippet_notice = None;
        if self.snippet_manage_filtering {
            self.handle_key_snippet_manage_filter(key);
            return Ok(());
        }
        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            _ if self.is_action(KeyAction::Search, &key) => self.snippet_manage_filtering = true,
            _ if self.is_action(KeyAction::Cancel, &key)
                && !self.snippet_manage_query.is_empty() =>
            {
                self.snippet_manage_query.clear();
            }
            _ if self.is_action(KeyAction::Cancel, &key)
                || self.is_action(KeyAction::TabHosts, &key) =>
            {
                self.close_snippet_manage();
            }
            _ if self.is_action(KeyAction::MoveDown, &key) => self.move_snippet_manage_selection(1),
            _ if self.is_action(KeyAction::MoveUp, &key) => self.move_snippet_manage_selection(-1),
            KeyCode::PageDown => self.move_snippet_manage_selection(10),
            KeyCode::PageUp => self.move_snippet_manage_selection(-10),
            KeyCode::Home => self.move_snippet_manage_selection(isize::MIN),
            KeyCode::End => self.move_snippet_manage_selection(isize::MAX),
            _ if self.is_action(KeyAction::AddHost, &key) => self.enter_snippet_form(None),
            _ if self.is_action(KeyAction::Edit, &key) || key.code == KeyCode::Enter => {
                match self.selected_managed_snippet() {
                    Some(snippet) => self.enter_snippet_form(Some(&snippet)),
                    None => self.snippet_notice = Some("No snippet selected".into()),
                }
            }
            _ if self.is_action(KeyAction::Delete, &key) => match self.selected_managed_snippet() {
                Some(snippet) => {
                    self.pending_delete = Some(PendingDelete::Snippet {
                        id: snippet.id,
                        name: snippet.name,
                    });
                    self.mode = AppMode::ConfirmDelete;
                }
                None => self.snippet_notice = Some("No snippet selected".into()),
            },
            _ => {}
        }
        self.snap_snippet_manage_selection();
        Ok(())
    }

    /// Keystrokes while the manager's `/` filter has focus: letters are query
    /// text, arrows move, Enter keeps the filter, Esc drops it.
    fn handle_key_snippet_manage_filter(&mut self, key: KeyEvent) {
        match key.code {
            _ if self.is_action(KeyAction::Cancel, &key) => {
                self.snippet_manage_query.clear();
                self.snippet_manage_filtering = false;
            }
            _ if self.is_action(KeyAction::Connect, &key) => self.snippet_manage_filtering = false,
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.snippet_manage_query.push(c);
            }
            KeyCode::Backspace => {
                self.snippet_manage_query.pop();
            }
            KeyCode::Up => self.move_snippet_manage_selection(-1),
            KeyCode::Down => self.move_snippet_manage_selection(1),
            KeyCode::PageUp => self.move_snippet_manage_selection(-10),
            KeyCode::PageDown => self.move_snippet_manage_selection(10),
            _ => {}
        }
        self.snap_snippet_manage_selection();
    }

    // ── Add/edit form ───────────────────────────────────────────

    pub(crate) fn enter_snippet_form(&mut self, existing: Option<&Snippet>) {
        let form = match existing {
            Some(snippet) => SnippetFormEdit {
                id: Some(snippet.id),
                name: snippet.name.clone(),
                command: snippet.command.clone(),
                description: snippet.description.clone().unwrap_or_default(),
                tags: snippet.tags.join(" "),
                field: SnippetFormField::Name,
                cursor: text_input::char_len(&snippet.name),
                dirty: false,
            },
            None => SnippetFormEdit {
                id: None,
                name: String::new(),
                command: String::new(),
                description: String::new(),
                tags: String::new(),
                field: SnippetFormField::Name,
                cursor: 0,
                dirty: false,
            },
        };
        self.snippet_form = Some(form);
        self.mode = AppMode::SnippetForm;
    }

    pub(crate) fn handle_key_snippet_form(&mut self, key: KeyEvent) -> Result<()> {
        let Some(field) = self.snippet_form.as_ref().map(|f| f.field) else {
            self.mode = AppMode::SnippetManage;
            return Ok(());
        };
        match key.code {
            KeyCode::Esc => self.cancel_snippet_form()?,
            _ if self.is_save_key(&key) => self.save_snippet_form()?,
            // Enter advances through the fields and saves on the last one, the
            // same convention as the host / identity / keygen forms.
            KeyCode::Enter => {
                if field == SnippetFormField::Tags {
                    self.save_snippet_form()?;
                } else {
                    self.snippet_form_move_field(1);
                }
            }
            KeyCode::Up | KeyCode::BackTab => self.snippet_form_move_field(-1),
            KeyCode::Down | KeyCode::Tab => self.snippet_form_move_field(1),
            KeyCode::Backspace if key.modifiers.is_empty() => self.snippet_form_backspace(),
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End | KeyCode::Delete => {
                self.snippet_form_cursor_key(key.code)
            }
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(form) = self.snippet_form.as_mut() {
                    let cursor = form.cursor;
                    let value = Self::snippet_form_field_mut(form);
                    form.cursor = text_input::clear_before_cursor(value, cursor);
                    form.dirty = true;
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.snippet_form_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn cancel_snippet_form(&mut self) -> Result<()> {
        if self.snippet_form.as_ref().is_some_and(|f| f.dirty) {
            self.mode = AppMode::ConfirmDiscard;
            Ok(())
        } else {
            self.discard_snippet_form()
        }
    }

    pub(crate) fn discard_snippet_form(&mut self) -> Result<()> {
        self.snippet_form = None;
        self.enter_snippet_manage()
    }

    pub(crate) fn save_snippet_form(&mut self) -> Result<()> {
        let Some(form) = self.snippet_form.take() else {
            self.mode = AppMode::SnippetManage;
            return Ok(());
        };

        let command = form.command.trim();
        if command.is_empty() {
            self.snippet_notice = Some("Command is required".into());
            self.snippet_form = Some(form);
            return Ok(());
        }
        // No name typed: the start of the command names it.
        let default_name;
        let name = match form.name.trim() {
            "" => {
                default_name = snippet_default_name(command);
                default_name.as_str()
            }
            typed => typed,
        };

        let description = {
            let d = form.description.trim();
            (!d.is_empty()).then(|| d.to_string())
        };
        let tags: Vec<String> = form
            .tags
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        // De-duplicate while keeping first-seen order.
        let mut seen = std::collections::HashSet::new();
        let tags: Vec<String> = tags
            .into_iter()
            .filter(|t| seen.insert(t.clone()))
            .collect();

        if let Some(id) = form.id {
            self.store.update_snippet(
                id,
                &SnippetUpdate {
                    name: Some(name.to_string()),
                    command: Some(command.to_string()),
                    description: Some(description),
                    tags: Some(tags),
                },
            )?;
            self.snippet_notice = Some(format!("Snippet '{name}' updated"));
        } else {
            self.store.create_snippet(&NewSnippet {
                name: name.to_string(),
                command: command.to_string(),
                description,
                tags,
            })?;
            self.snippet_notice = Some(format!("Snippet '{name}' saved"));
        }

        self.reload_snippets()?;
        // Keep the freshly saved snippet highlighted.
        if let Some(pos) = self.snippets.iter().position(|s| s.name == name) {
            self.snippet_manage_selected = pos;
        }
        if !self
            .snippet_manage_view()
            .contains(&self.snippet_manage_selected)
        {
            if let Some(n) = self.snippet_notice.as_mut() {
                n.push_str(" (hidden by the filter)");
            }
            self.snap_snippet_manage_selection();
        }
        self.mode = AppMode::SnippetManage;
        Ok(())
    }

    fn snippet_form_move_field(&mut self, delta: i32) {
        if let Some(form) = self.snippet_form.as_mut() {
            form.field = if delta >= 0 {
                form.field.next()
            } else {
                form.field.prev()
            };
            form.cursor = text_input::char_len(Self::snippet_form_field_ref(form));
        }
    }

    fn snippet_form_insert(&mut self, ch: char) {
        if let Some(form) = self.snippet_form.as_mut() {
            let cursor = form.cursor;
            form.cursor = text_input::insert_at(Self::snippet_form_field_mut(form), cursor, ch);
            form.dirty = true;
        }
    }

    fn snippet_form_backspace(&mut self) {
        if let Some(form) = self.snippet_form.as_mut() {
            let cursor = form.cursor;
            form.cursor = text_input::backspace_at(Self::snippet_form_field_mut(form), cursor);
            form.dirty = true;
        }
    }

    fn snippet_form_cursor_key(&mut self, code: KeyCode) {
        if let Some(form) = self.snippet_form.as_mut() {
            let mut cursor = form.cursor;
            let value = Self::snippet_form_field_mut(form);
            // Delete can remove a character; mark the form dirty so an Esc after
            // it still routes through the discard prompt (matches the host form).
            let changed = text_input::handle_cursor_key(code, value, &mut cursor);
            form.cursor = cursor;
            if changed == Some(true) {
                form.dirty = true;
            }
        }
    }

    fn snippet_form_field_mut(form: &mut SnippetFormEdit) -> &mut String {
        match form.field {
            SnippetFormField::Name => &mut form.name,
            SnippetFormField::Command => &mut form.command,
            SnippetFormField::Description => &mut form.description,
            SnippetFormField::Tags => &mut form.tags,
        }
    }

    pub(super) fn snippet_form_field_ref(form: &SnippetFormEdit) -> &str {
        match form.field {
            SnippetFormField::Name => &form.name,
            SnippetFormField::Command => &form.command,
            SnippetFormField::Description => &form.description,
            SnippetFormField::Tags => &form.tags,
        }
    }

    // ── In-session fuzzy picker ─────────────────────────────────

    /// Open the snippet picker floated over the current session. `return_mode`
    /// is the mode to restore when it closes (the live session).
    pub(crate) fn open_snippet_picker(&mut self, return_mode: AppMode) -> Result<()> {
        self.reload_snippets()?;
        let results = (0..self.snippets.len()).collect();
        self.snippet_picker = Some(SnippetPickerState {
            query: String::new(),
            results,
            selected: 0,
            return_mode,
        });
        self.mode = AppMode::SnippetPicker;
        Ok(())
    }

    pub(crate) fn handle_key_snippet_picker(&mut self, key: KeyEvent) -> Result<()> {
        let Some(state) = self.snippet_picker.as_ref() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };
        let return_mode = state.return_mode;
        match key.code {
            KeyCode::Esc => self.close_snippet_picker(return_mode),
            KeyCode::Enter => self.inject_selected_snippet(true, return_mode),
            KeyCode::Tab => self.inject_selected_snippet(false, return_mode),
            // Plain letters are query text, even ones bound to nav (j/k); list
            // movement lives on the rebindable MoveUp/MoveDown actions below,
            // matched after this arm so a typed name is never eaten as a key
            // (the same ordering the quick-connect palette uses).
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                if let Some(state) = self.snippet_picker.as_mut() {
                    state.query.push(c);
                }
                self.rebuild_snippet_picker_results();
            }
            KeyCode::Backspace => {
                if let Some(state) = self.snippet_picker.as_mut() {
                    state.query.pop();
                }
                self.rebuild_snippet_picker_results();
            }
            _ if self.is_action(KeyAction::MoveUp, &key) => self.move_snippet_picker_selection(-1),
            _ if self.is_action(KeyAction::MoveDown, &key) => self.move_snippet_picker_selection(1),
            _ => {}
        }
        Ok(())
    }

    fn move_snippet_picker_selection(&mut self, delta: isize) {
        if let Some(state) = self.snippet_picker.as_mut() {
            if state.results.is_empty() {
                state.selected = 0;
                return;
            }
            let max = state.results.len() as isize - 1;
            state.selected = (state.selected as isize + delta).clamp(0, max) as usize;
        }
    }

    fn rebuild_snippet_picker_results(&mut self) {
        let Some(state) = self.snippet_picker.as_ref() else {
            return;
        };
        let results = rank_snippets(&self.snippets, &state.query);
        if let Some(state) = self.snippet_picker.as_mut() {
            let len = results.len();
            state.results = results;
            state.selected = if len == 0 {
                0
            } else {
                state.selected.min(len - 1)
            };
        }
    }

    fn close_snippet_picker(&mut self, return_mode: AppMode) {
        self.snippet_picker = None;
        self.mode = return_mode;
    }

    /// Type the selected snippet's command into the active session's PTY, with a
    /// trailing carriage return when `send_enter` (so the shell runs it).
    ///
    /// Failures surface through the [`AppMode::Notice`] modal rather than
    /// `host_notice`: the session view early-returns before the dashboard toast
    /// is drawn, so a dropped write would otherwise be invisible.
    fn inject_selected_snippet(&mut self, send_enter: bool, return_mode: AppMode) {
        let command = self.snippet_picker.as_ref().and_then(|state| {
            state
                .results
                .get(state.selected)
                .and_then(|&idx| self.snippets.get(idx))
                .map(|s| s.command.clone())
        });
        self.snippet_picker = None;

        let Some(command) = command else {
            // Empty picker or no match: nothing to run, return to the session.
            self.mode = return_mode;
            return;
        };
        self.mode = return_mode;
        self.insert_session_command(&command, send_enter, false);
    }

    /// Show a modal notice popup (dismissed by any key). Used for snippet
    /// injection failures, which happen while the session view owns the frame.
    pub(super) fn show_notice_popup(&mut self, message: String) {
        self.notice_popup = Some(message);
        self.mode = AppMode::Notice;
    }
}

/// Name for a snippet saved without one: the command's first line, trimmed,
/// cut to 30 characters.
fn snippet_default_name(command: &str) -> String {
    let line = command.lines().next().unwrap_or("").trim();
    line.chars()
        .take(30)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// Fuzzy-rank snippets against `query`, returning indices into `snippets`
/// best-match first. An empty query keeps the store's own (name) order.
///
/// Matches over each snippet's name, command, joined tags and description,
/// keeping the best-scoring field — the same per-field-max approach the host
/// search uses.
pub(crate) fn rank_snippets(snippets: &[Snippet], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..snippets.len()).collect();
    }

    // Reuse one matcher across keystrokes (like `HostSearch`) rather than
    // rebuilding it on every query. The picker runs on the single-threaded event
    // loop; a thread-local keeps tests (which call this in parallel) isolated.
    thread_local! {
        static MATCHER: std::cell::RefCell<Matcher> =
            std::cell::RefCell::new(Matcher::new(Config::DEFAULT));
    }

    MATCHER.with(|cell| {
        let mut matcher = cell.borrow_mut();
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, usize)> = Vec::new();

        for (idx, snippet) in snippets.iter().enumerate() {
            let mut best: Option<u32> = None;
            let mut score_field = |field: &str, best: &mut Option<u32>| {
                if field.is_empty() {
                    return;
                }
                buf.clear();
                if let Some(score) = pattern.score(Utf32Str::new(field, &mut buf), &mut matcher) {
                    *best = Some(best.map_or(score, |cur| cur.max(score)));
                }
            };
            score_field(&snippet.name, &mut best);
            score_field(&snippet.command, &mut best);
            let tags = snippet.tags.join(" ");
            score_field(&tags, &mut best);
            if let Some(description) = snippet.description.as_deref() {
                score_field(description, &mut best);
            }
            if let Some(score) = best {
                scored.push((score, idx));
            }
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, idx)| idx).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::rank_snippets;
    use crate::store::Snippet;

    fn snippet(name: &str, command: &str, tags: &[&str], description: Option<&str>) -> Snippet {
        Snippet {
            id: 0,
            name: name.into(),
            command: command.into(),
            description: description.map(str::to_string),
            tags: tags.iter().map(|t| (*t).to_string()).collect(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn fixture() -> Vec<Snippet> {
        vec![
            snippet(
                "restart nginx",
                "sudo systemctl restart nginx",
                &["web", "ops"],
                Some("bounce the web server"),
            ),
            snippet("tail syslog", "tail -f /var/log/syslog", &["logs"], None),
            snippet("disk usage", "df -h", &["ops"], Some("free space")),
        ]
    }

    #[test]
    fn empty_query_keeps_store_order() {
        let snippets = fixture();
        assert_eq!(rank_snippets(&snippets, ""), vec![0, 1, 2]);
        assert_eq!(rank_snippets(&snippets, "   "), vec![0, 1, 2]);
    }

    #[test]
    fn matches_name() {
        let snippets = fixture();
        assert_eq!(rank_snippets(&snippets, "nginx"), vec![0]);
    }

    #[test]
    fn matches_command_body() {
        let snippets = fixture();
        assert_eq!(rank_snippets(&snippets, "df"), vec![2]);
    }

    #[test]
    fn matches_tag_and_description() {
        let snippets = fixture();
        assert_eq!(rank_snippets(&snippets, "logs"), vec![1]);
        assert_eq!(rank_snippets(&snippets, "space"), vec![2]);
    }

    #[test]
    fn no_match_is_empty() {
        let snippets = fixture();
        assert!(rank_snippets(&snippets, "zzzznope").is_empty());
    }
}
