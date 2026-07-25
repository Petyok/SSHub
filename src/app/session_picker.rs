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

    pub(crate) fn open_new_session_picker(&mut self) {
        self.open_session_picker(SessionPickerPurpose::NewSession);
    }

    /// Open the shared host picker for whatever `purpose` wants a host.
    pub(crate) fn open_session_picker(&mut self, purpose: SessionPickerPurpose) {
        let return_mode = self.mode;
        self.session_picker = Some(SessionPicker {
            query: String::new(),
            selected: 0,
            return_mode,
            purpose,
        });
        self.mode = AppMode::SessionPicker;
    }

    pub(crate) fn handle_key_session_picker(&mut self, key: KeyEvent) -> Result<()> {
        let return_mode = self
            .session_picker
            .as_ref()
            .map(|p| p.return_mode)
            .unwrap_or(AppMode::Normal);
        let len = self.session_picker_host_matches().len();
        match key.code {
            KeyCode::Esc => {
                self.session_picker = None;
                self.mode = return_mode;
            }
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
                let matches = self.session_picker_host_matches();
                let picked = self
                    .session_picker
                    .as_ref()
                    .and_then(|p| matches.get(p.selected).map(|(idx, _)| (*idx, p.purpose)));
                self.session_picker = None;
                self.mode = return_mode;
                match picked {
                    Some((idx, SessionPickerPurpose::NewSession)) => self.connect_host_at(idx)?,
                    Some((idx, SessionPickerPurpose::SftpLeftPane)) => {
                        self.sftp_connect_left_pane(idx)?
                    }
                    None => {}
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
