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
        if self.try_tab_switch(&key)? {
            return Ok(());
        }

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            _ if self.is_action(KeyAction::MoveDown, &key) => {
                if !self.auth_events_cache.is_empty() {
                    self.audit_selected =
                        (self.audit_selected + 1).min(self.auth_events_cache.len() - 1);
                }
            }
            _ if self.is_action(KeyAction::MoveUp, &key) => {
                self.audit_selected = self.audit_selected.saturating_sub(1);
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

    pub(crate) fn refresh_audit_events(&mut self) {
        let status = self.audit_filter.sql_status();
        let since = self.audit_range.since_timestamp();
        if let Ok(events) = self.store.list_auth_events_filtered(status, since, 500) {
            self.auth_events_cache = events;
        }
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
}
