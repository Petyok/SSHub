use super::*;
use crate::command_safety::{classify_command, CommandSafety};

impl App {
    pub(crate) fn insert_session_command(
        &mut self,
        command: &str,
        execute: bool,
        require_safe: bool,
    ) {
        if require_safe && classify_command(command) != CommandSafety::Safe {
            self.suggestion_notice("Command is not eligible for insertion.");
            return;
        }
        let Some(session) = self.active_session_mut() else {
            self.suggestion_notice("No active session for insertion.");
            return;
        };
        if !session.is_running() {
            self.suggestion_notice("Session is not running.");
            return;
        }
        session.observe_paste(command);
        if session.write_paste(command.as_bytes()).is_err() {
            session.history.input.invalidate();
            self.suggestion_notice("Could not insert command into session.");
            return;
        }
        if execute {
            let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
            let candidate = session.observe_key(enter);
            let bytes =
                crate::session::keys::encode(enter, session.parser.screen().application_cursor())
                    .unwrap();
            if session.write(&bytes).is_err() {
                session.history.input.invalidate();
                self.suggestion_notice("Command inserted, but Enter could not be sent.");
                return;
            }
            self.record_session_command(candidate);
        }
    }

    pub(crate) fn suggestion_notice(&mut self, message: &str) {
        if let Some(session) = self.active_session_mut() {
            session.set_copy_notice(message.into());
        } else {
            self.show_notice_popup(message.into());
        }
    }

    pub(crate) fn record_session_command(&mut self, candidate: Option<String>) {
        let Some(command) = candidate else {
            return;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let host_id = {
            let Some(session) = self.active_session_mut() else {
                return;
            };
            if !session.is_live_authenticated() {
                return;
            }
            session.history.record(&command, now);
            session.meta.host_id
        };
        // The store changed under the ghost's cached host history; the next
        // frame reloads so a just-ran command can complete immediately.
        self.invalidate_ghost_host_cache();
        if self.config.command_history.enabled
            && host_id.is_some()
            && self
                .store
                .record_command(
                    host_id,
                    &command,
                    self.config.command_history.max_entries_per_host,
                )
                .is_err()
        {
            self.suggestion_notice("Could not save command history.");
        }
    }
}
