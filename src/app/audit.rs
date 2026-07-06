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
        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.auth_events_cache.is_empty() {
                    self.audit_selected =
                        (self.audit_selected + 1).min(self.auth_events_cache.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.audit_selected = self.audit_selected.saturating_sub(1);
            }
            KeyCode::Char('f') if key.modifiers.is_empty() => {
                self.audit_filter = self.audit_filter.next();
                self.audit_selected = 0;
                self.refresh_audit_events();
            }
            KeyCode::Char('r') if key.modifiers.is_empty() => {
                self.audit_range = self.audit_range.next();
                self.audit_selected = 0;
                self.refresh_audit_events();
            }
            KeyCode::Char('1') if key.modifiers.is_empty() => self.active_tab = 0,
            KeyCode::Char('2') if key.modifiers.is_empty() => self.switch_to_tunnels_tab()?,
            KeyCode::Char('3') if key.modifiers.is_empty() => self.switch_to_keys_tab()?,
            KeyCode::Char('4') if key.modifiers.is_empty() => {
                self.active_tab = 3;
                self.refresh_audit_events();
            }
            KeyCode::Char('h') if key.modifiers.is_empty() => self.active_tab = 0,
            _ if self.is_action(KeyAction::Help, &key) => {
                self.pre_help_mode = Some(self.mode);
                self.mode = AppMode::Help;
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
