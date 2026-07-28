//! The searchable picker shared by every entry point that needs the user to
//! choose from a list: opening a new session tab, pointing the SFTP browser's
//! left pane at a second server, and (from task 3) switching to an already-open
//! session. One state machine, one renderer — the purpose decides the rest.

use super::*;

impl App {
    /// Hosts matching the session tab picker's query, as `(host index, label)`.
    pub fn session_picker_host_matches(&self) -> Vec<(usize, String)> {
        let query = self
            .session_picker
            .as_ref()
            .map(|p| p.query.to_lowercase())
            .unwrap_or_default();
        self.hosts
            .iter()
            .enumerate()
            .filter(|(_, h)| {
                if query.is_empty() {
                    return true;
                }
                let name = h.name().to_lowercase();
                let label = h.display_name().to_lowercase();
                name.contains(&query) || label.contains(&query)
            })
            .map(|(idx, h)| (idx, format!("{}  {}", h.display_name(), h.name())))
            .collect()
    }

    /// The filtered list the picker currently shows.
    pub fn session_picker_rows(&self) -> Vec<PickerRow> {
        let Some(picker) = self.session_picker.as_ref() else {
            return Vec::new();
        };
        if picker.purpose.over_sessions() {
            self.picker_session_rows(&picker.query.to_lowercase())
        } else {
            self.session_picker_host_matches()
                .into_iter()
                .map(|(index, name)| PickerRow {
                    index,
                    badge: None,
                    ordinal: None,
                    name,
                    endpoint: String::new(),
                    current: false,
                })
                .collect()
        }
    }

    fn picker_session_rows(&self, query: &str) -> Vec<PickerRow> {
        self.sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                if query.is_empty() {
                    return true;
                }
                let user = s.meta.user.as_deref().unwrap_or_default().to_lowercase();
                let address = s.meta.address.as_deref().unwrap_or_default().to_lowercase();
                s.display_name.to_lowercase().contains(query)
                    || user.contains(query)
                    || address.contains(query)
            })
            .map(|(index, s)| PickerRow {
                index,
                badge: Some(match s.phase {
                    crate::session::SessionPhase::Connecting { .. } => PickerBadge::Connecting,
                    crate::session::SessionPhase::Running { .. } => PickerBadge::Up,
                    crate::session::SessionPhase::Exited { .. } => PickerBadge::Exited,
                }),
                ordinal: Some(index + 1),
                name: s.display_name.clone(),
                endpoint: endpoint_label(&s.meta),
                current: self.active_session == Some(index),
            })
            .collect()
    }

    pub(crate) fn open_new_session_picker(&mut self) {
        self.open_session_picker(SessionPickerPurpose::NewSession);
    }

    /// Open the shared picker for `purpose`. Refuses rather than showing an empty
    /// or nonsensical overlay: a switcher needs at least one session, and the
    /// picker is only meaningful over the dashboard or a session.
    pub(crate) fn open_session_picker(&mut self, purpose: SessionPickerPurpose) {
        if purpose.over_sessions() && self.sessions.is_empty() {
            return;
        }
        if !matches!(
            self.mode,
            AppMode::Normal | AppMode::Session | AppMode::Connecting
        ) {
            return;
        }
        // With an empty query the session list is `App::sessions` verbatim, so the
        // active session's index doubles as its row position. Filter a stale index
        // rather than trusting it.
        let selected = if purpose.over_sessions() {
            self.active_session
                .filter(|&i| i < self.sessions.len())
                .unwrap_or(0)
        } else {
            0
        };
        self.session_picker = Some(SessionPicker {
            purpose,
            query: String::new(),
            selected,
            return_mode: self.mode,
        });
        self.mode = AppMode::SessionPicker;
    }

    /// Dismiss the picker and restore a sensible mode. `return_mode` can be stale:
    /// a picker opened while a session was still connecting may be dismissed after
    /// that session started running or died, so the session modes are re-derived
    /// from the current phase rather than restored verbatim.
    fn close_session_picker(&mut self) {
        let return_mode = self
            .session_picker
            .as_ref()
            .map(|p| p.return_mode)
            .unwrap_or(AppMode::Normal);
        self.session_picker = None;
        match return_mode {
            AppMode::Session | AppMode::Connecting => self.focus_active_session(),
            other => self.mode = other,
        }
        // `focus_active_session` leaves `mode` untouched when there is no active
        // session to focus but sessions remain, which would strand the app in
        // `SessionPicker` with no picker on screen. The dashboard is always a
        // valid place to land.
        if self.mode == AppMode::SessionPicker {
            self.mode = AppMode::Normal;
        }
    }

    pub(crate) fn handle_key_session_picker(&mut self, key: KeyEvent) -> Result<()> {
        let rows = self.session_picker_rows();
        let len = rows.len();
        match key.code {
            KeyCode::Esc => self.close_session_picker(),
            KeyCode::Down => {
                if len > 0 {
                    if let Some(p) = self.session_picker.as_mut() {
                        p.selected = (p.selected + 1) % len;
                    }
                }
            }
            KeyCode::Up => {
                if len > 0 {
                    if let Some(p) = self.session_picker.as_mut() {
                        p.selected = (p.selected + len - 1) % len;
                    }
                }
            }
            KeyCode::Enter => {
                let picked = self
                    .session_picker
                    .as_ref()
                    .and_then(|p| rows.get(p.selected).map(|r| (r.index, p.purpose)));
                // Nothing selectable: leave the overlay up so the query can be
                // corrected, instead of dropping the user out with no feedback.
                let Some((index, purpose)) = picked else {
                    return Ok(());
                };
                match purpose {
                    SessionPickerPurpose::SwitchSession => {
                        // A jump between two session tabs slides the strip just
                        // like the cycling keys do (#35). Only a session origin
                        // has an outgoing tab on screen to carry off: opened from
                        // the dashboard there is none, and that mirror-image gap
                        // is #49, not this branch's business.
                        let from = self
                            .session_picker
                            .as_ref()
                            .filter(|p| {
                                matches!(p.return_mode, AppMode::Session | AppMode::Connecting)
                            })
                            .and(self.active_session)
                            .filter(|&from| from != index);
                        if let Some(from) = from {
                            self.arm_session_tab_switch(if index > from { 1 } else { -1 }, from);
                        }
                        // Retarget first, drop the picker, then derive the mode from
                        // the session we are switching *to* — never from wherever
                        // the picker happened to be opened.
                        self.active_session = Some(index);
                        self.session_picker = None;
                        self.focus_active_session();
                    }
                    SessionPickerPurpose::NewSession => {
                        self.close_session_picker();
                        self.connect_host_at(index)?;
                    }
                    SessionPickerPurpose::SftpLeftPane => {
                        self.close_session_picker();
                        self.sftp_connect_left_pane(index)?;
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(p) = self.session_picker.as_mut() {
                    p.query.pop();
                    p.selected = 0;
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                if let Some(p) = self.session_picker.as_mut() {
                    p.query.push(c);
                    p.selected = 0;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Render a session's endpoint, omitting every part that is not actually known.
/// Nothing is substituted — no `display_name` standing in for a missing address,
/// no assumed port 22 — because a guessed endpoint is worse than none when the
/// whole point of the line is telling two similar sessions apart. IPv6 literals
/// get bracketed once a port is appended.
fn endpoint_label(meta: &crate::session::SessionMeta) -> String {
    let Some(address) = meta.address.as_deref() else {
        return String::new();
    };
    let host = match meta.port {
        Some(port) if address.contains(':') => format!("[{address}]:{port}"),
        Some(port) => format!("{address}:{port}"),
        None => address.to_string(),
    };
    match meta.user.as_deref() {
        Some(user) => format!("{user}@{host}"),
        None => host,
    }
}

#[cfg(test)]
mod tests {
    use crate::session::SessionMeta;

    #[test]
    fn endpoint_label_omits_every_unknown_part() {
        let full = SessionMeta {
            user: Some("micha".into()),
            address: Some("10.0.0.12".into()),
            port: Some(22),
            ..Default::default()
        };
        let no_user = SessionMeta {
            address: Some("10.0.0.12".into()),
            port: Some(2222),
            ..Default::default()
        };
        let no_port = SessionMeta {
            user: Some("root".into()),
            address: Some("example.com".into()),
            ..Default::default()
        };
        let v6 = SessionMeta {
            address: Some("fe80::1".into()),
            port: Some(22),
            ..Default::default()
        };

        assert_eq!(super::endpoint_label(&full), "micha@10.0.0.12:22");
        assert_eq!(super::endpoint_label(&no_user), "10.0.0.12:2222");
        assert_eq!(super::endpoint_label(&no_port), "root@example.com");
        assert_eq!(super::endpoint_label(&v6), "[fe80::1]:22");
        assert_eq!(super::endpoint_label(&SessionMeta::default()), "");
    }
}
