use super::*;

impl App {
    pub(crate) fn handle_audit_filter_click(&mut self, click_x: u16, body_x: u16) -> Result<()> {
        let margin = if self.terminal_area.width >= 132 {
            2
        } else if self.terminal_area.width >= 80 {
            1
        } else {
            0
        };
        let base_x = body_x + margin;

        // "filter: " = 8 chars
        let mut cx = base_x + 8;
        for f in [AuditFilter::All, AuditFilter::Ok, AuditFilter::Fail] {
            let label_len = f.label().len() as u16;
            if click_x >= cx && click_x < cx + label_len {
                self.audit_filter = f;
                self.refresh_audit_events();
                return Ok(());
            }
            cx += label_len + 2;
        }

        // "  range: " gap
        cx += 2 + 7;
        for r in [
            AuditRange::All,
            AuditRange::Today,
            AuditRange::Week,
            AuditRange::Month,
        ] {
            let label_len = r.label().len() as u16;
            if click_x >= cx && click_x < cx + label_len {
                self.audit_range = r;
                self.refresh_audit_events();
                return Ok(());
            }
            cx += label_len + 2;
        }

        Ok(())
    }

    pub(crate) fn handle_key_audit(&mut self, key: KeyEvent) -> Result<()> {
        if self.list_filter_editing {
            self.handle_key_audit_filter(key);
            return Ok(());
        }

        if self.try_tab_switch(&key)? {
            return Ok(());
        }

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            _ if self.is_action(KeyAction::Search, &key) => self.list_filter_editing = true,
            _ if self.is_action(KeyAction::Cancel, &key) && !self.audit_query.is_empty() => {
                self.audit_query.clear();
                self.audit_selected = 0;
                self.refresh_audit_events();
            }
            _ if self.is_action(KeyAction::MoveDown, &key) => self.move_audit_selection(1),
            _ if self.is_action(KeyAction::MoveUp, &key) => self.move_audit_selection(-1),
            KeyCode::PageDown | KeyCode::PageUp | KeyCode::Home | KeyCode::End => {
                self.audit_page_key(key.code)
            }
            _ if self.is_action(KeyAction::AuditFilter, &key) => {
                self.audit_filter = self.audit_filter.next();
                self.audit_selected = 0;
                self.refresh_audit_events();
            }
            _ if self.is_action(KeyAction::AuditRange, &key) => {
                self.audit_range = self.audit_range.next();
                self.audit_selected = 0;
                self.refresh_audit_events();
            }
            _ if self.is_action(KeyAction::Help, &key) => {
                self.open_help();
            }
            _ => {}
        }
        Ok(())
    }

    /// Keystrokes while the Audit filter bar has focus. Enter keeps the
    /// filter, Esc drops it; letters are query text, arrows still move.
    fn handle_key_audit_filter(&mut self, key: KeyEvent) {
        match key.code {
            _ if self.is_action(KeyAction::Cancel, &key) => {
                self.audit_query.clear();
                self.list_filter_editing = false;
            }
            _ if self.is_action(KeyAction::Connect, &key) => {
                self.list_filter_editing = false;
                return;
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.audit_query.push(c);
            }
            KeyCode::Backspace => {
                self.audit_query.pop();
            }
            KeyCode::Up => return self.move_audit_selection(-1),
            KeyCode::Down => return self.move_audit_selection(1),
            KeyCode::PageDown | KeyCode::PageUp | KeyCode::Home | KeyCode::End => {
                return self.audit_page_key(key.code);
            }
            _ => return,
        }
        self.audit_selected = 0;
        self.refresh_audit_events();
    }

    fn move_audit_selection(&mut self, delta: isize) {
        let max = self.auth_events_cache.len().saturating_sub(1) as isize;
        self.audit_selected = (self.audit_selected as isize + delta).clamp(0, max) as usize;
    }

    /// PageUp / PageDown step ten rows; Home / End jump to the ends.
    fn audit_page_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::PageDown => self.move_audit_selection(10),
            KeyCode::PageUp => self.move_audit_selection(-10),
            KeyCode::Home => self.audit_selected = 0,
            KeyCode::End => self.audit_selected = self.auth_events_cache.len().saturating_sub(1),
            _ => {}
        }
    }

    pub(crate) fn refresh_audit_events(&mut self) {
        if let Ok(events) = self.load_audit_events() {
            self.auth_events_cache = events;
        }
        self.audit_selected = self
            .audit_selected
            .min(self.auth_events_cache.len().saturating_sub(1));
    }

    /// Audit events for the current status filter and range, narrowed to
    /// those whose host, user, status or note fuzzy-match `audit_query`.
    /// Matches keep their time order: this is a log, not a ranking.
    pub(crate) fn load_audit_events(&self) -> Result<Vec<crate::store::AuthEvent>> {
        let status = self.audit_filter.sql_status();
        let since = self.audit_range.since_timestamp();
        let events = self.store.list_auth_events_filtered(status, since, 500)?;
        if self.audit_query.is_empty() {
            return Ok(events);
        }
        let items: Vec<Vec<String>> = events
            .iter()
            .map(|e| {
                vec![
                    e.host_name.clone(),
                    e.username.clone().unwrap_or_default(),
                    e.status.clone(),
                    e.note.clone().unwrap_or_default(),
                ]
            })
            .collect();
        let mut keep = crate::search::fuzzy_indices(&self.audit_query, &items);
        keep.sort_unstable();
        Ok(events
            .into_iter()
            .enumerate()
            .filter(|(i, _)| keep.binary_search(i).is_ok())
            .map(|(_, e)| e)
            .collect())
    }
}

#[cfg(test)]
mod tests {

    /// Repro for "the Audit tab shows NOTHING at all": a launched session must
    /// be visible after the exact call the tab switch makes. Drives the real
    /// store write and the real tab refresh — not the unfiltered
    /// `list_auth_events` the launch path was already pinned against.
    #[test]
    fn audit_tab_lists_launched_event_after_tab_switch() {
        let mut app = crate::app::tests::test_app(vec![]);
        app.store
            .log_auth_event(
                "web",
                Some("root"),
                "direct",
                "launched",
                "session started",
                None,
            )
            .unwrap();
        assert!(app.auth_events_cache.is_empty());
        // What `try_tab_switch` does on the audit key (`5` by default).
        app.active_tab = 4;
        app.refresh_audit_events();
        assert_eq!(app.active_tab, 4);
        assert!(
            !app.auth_events_cache.is_empty(),
            "launched connect must be visible on the Audit tab"
        );
        assert_eq!(app.auth_events_cache[0].status, "launched");
    }

    fn app_with_events(hosts: &[&str]) -> crate::app::App {
        let mut app = crate::app::tests::test_app(vec![]);
        for h in hosts {
            app.store
                .log_auth_event(h, Some("root"), "direct", "ok", "connected", None)
                .unwrap();
        }
        app.active_tab = 4;
        app.refresh_audit_events();
        app
    }

    #[test]
    fn slash_filters_audit_events_and_esc_clears() {
        use crate::app::tests::{key, key_char};
        use crossterm::event::KeyCode;
        let mut app = app_with_events(&["web", "db", "web"]);
        assert_eq!(app.auth_events_cache.len(), 3);

        app.handle_key(key_char('/')).unwrap();
        assert!(app.list_filter_editing);
        // `d` and `f` are bound keys elsewhere; here they are query text.
        for c in "db".chars() {
            app.handle_key(key_char(c)).unwrap();
        }
        assert_eq!(app.audit_query, "db");
        assert_eq!(app.auth_events_cache.len(), 1);
        assert_eq!(app.auth_events_cache[0].host_name, "db");

        app.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(!app.list_filter_editing);
        assert_eq!(app.auth_events_cache.len(), 1, "Enter keeps the filter");

        app.handle_key(key(KeyCode::Esc)).unwrap();
        assert!(app.audit_query.is_empty());
        assert_eq!(app.auth_events_cache.len(), 3);
    }

    #[test]
    fn audit_pages_with_pgdn_end_home() {
        use crate::app::tests::key;
        use crossterm::event::KeyCode;
        let mut app = app_with_events(&["h"; 25]);
        app.handle_key(key(KeyCode::PageDown)).unwrap();
        assert_eq!(app.audit_selected, 10);
        app.handle_key(key(KeyCode::End)).unwrap();
        assert_eq!(app.audit_selected, 24);
        app.handle_key(key(KeyCode::PageUp)).unwrap();
        assert_eq!(app.audit_selected, 14);
        app.handle_key(key(KeyCode::Home)).unwrap();
        assert_eq!(app.audit_selected, 0);
    }
}
