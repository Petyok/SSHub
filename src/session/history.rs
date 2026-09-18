//! Best-effort local input indexing; never edits or sends PTY input.
use crate::command_safety::{classify_command, CommandSafety, MAX_COMMAND_BYTES};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub const SESSION_HISTORY_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub command: String,
    pub last_used: i64,
    pub use_count: u64,
}

#[derive(Default)]
pub struct InputTracker {
    line: String,
    uncertain: bool,
}

impl InputTracker {
    pub fn invalidate(&mut self) {
        self.line.clear();
        self.uncertain = true;
    }

    /// Remote output arrived while capture is disallowed (connecting-phase
    /// bytes, scrolled-back output, a password-worded line, …). Only a
    /// pending partial line can be corrupted by missed rewrites, so an
    /// empty tracker stays clean: pre-auth chatter and idle-window output
    /// must not poison the next typed command. Direct input in a risky
    /// context still goes through [`Self::invalidate`] and stays sticky.
    pub fn invalidate_pending_input(&mut self) {
        if !self.line.is_empty() {
            self.invalidate();
        }
    }

    /// Keystrokes forwarded since the last Enter, or empty while uncertain.
    /// Ghost completion reads this (never the grid) so the prompt is never
    /// parsed and remote echo lag cannot corrupt the query.
    pub fn typed(&self) -> &str {
        if self.uncertain {
            ""
        } else {
            &self.line
        }
    }

    /// False after any key the tracker cannot model (arrows, mouse, unknown
    /// sequences): the ghost must hide because the cursor may have moved.
    pub fn is_certain(&self) -> bool {
        !self.uncertain
    }

    /// A fresh shell line begins (authentication just completed): pre-auth
    /// partial input belonged to the login exchange, never to a command.
    /// Called once, on the connected-latch transition.
    pub(crate) fn reset(&mut self) {
        self.line.clear();
        self.uncertain = false;
    }

    pub fn paste(&mut self, text: &str, allowed: bool) {
        // A later line could answer a password prompt not rendered yet. Do not
        // split multiline pastes into apparently safe fragments.
        if !allowed
            || text.chars().any(char::is_control)
            || self.line.len().saturating_add(text.len()) > MAX_COMMAND_BYTES
        {
            self.invalidate();
        } else if !self.uncertain {
            self.line.push_str(text);
        }
    }

    pub fn key(&mut self, key: KeyEvent, allowed: bool) -> Option<String> {
        if !allowed {
            self.invalidate();
            return None;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Char('c' | 'u'), KeyModifiers::CONTROL) => self.reset(),
            (KeyCode::Enter, modifiers) if modifiers.is_empty() => {
                let command = (!self.uncertain).then(|| self.line.trim().to_owned());
                self.reset();
                return command.filter(|s| classify_command(s) == CommandSafety::Safe);
            }
            (KeyCode::Backspace, modifiers) if modifiers.is_empty() => {
                if self.line.pop().is_none() {
                    self.invalidate();
                }
            }
            (KeyCode::Char(ch), modifiers)
                if (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT)
                    && !ch.is_control() =>
            {
                if !self.uncertain {
                    if self.line.len() + ch.len_utf8() > MAX_COMMAND_BYTES {
                        self.invalidate();
                    } else {
                        self.line.push(ch);
                    }
                }
            }
            _ => self.invalidate(),
        }
        None
    }
}

#[derive(Default)]
pub struct SessionHistory {
    pub entries: Vec<HistoryEntry>,
    pub input: InputTracker,
}

impl SessionHistory {
    pub fn record(&mut self, command: &str, timestamp: i64) {
        if classify_command(command) != CommandSafety::Safe {
            return;
        }
        let command = command.trim();
        let count = self
            .entries
            .iter()
            .position(|e| e.command == command)
            .map(|index| self.entries.remove(index).use_count)
            .unwrap_or(0);
        self.entries.insert(
            0,
            HistoryEntry {
                command: command.to_owned(),
                last_used: timestamp,
                use_count: count.saturating_add(1),
            },
        );
        self.entries.truncate(SESSION_HISTORY_LIMIT);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        // A clear action must not index a suffix of the pending remote line.
        self.input.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    #[test]
    fn tracker_edits_and_records_only_complete_safe_lines() {
        let mut input = InputTracker::default();
        input.paste("git statuX", true);
        input.key(key(KeyCode::Backspace), true);
        input.key(key(KeyCode::Char('s')), true);
        assert_eq!(
            input.key(key(KeyCode::Enter), true).as_deref(),
            Some("git status")
        );
        input.paste("mysql -psecret", true);
        assert!(input.key(key(KeyCode::Enter), true).is_none());
        input.paste("discard", true);
        input.key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            true,
        );
        assert!(input.key(key(KeyCode::Enter), true).is_none());
    }
    #[test]
    fn prompt_multiline_control_and_cursor_uncertainty_drop_whole_line() {
        for (text, allowed) in [
            ("credential", false),
            ("ls\ncredential", true),
            ("ls\0", true),
        ] {
            let mut input = InputTracker::default();
            input.paste(text, allowed);
            input.paste(" suffix", true);
            assert!(input.key(key(KeyCode::Enter), true).is_none());
        }
        let mut input = InputTracker::default();
        input.paste("echo foo", true);
        input.key(key(KeyCode::Left), true);
        input.paste("bar", true);
        assert!(input.key(key(KeyCode::Enter), true).is_none());
        input.paste(&"x".repeat(MAX_COMMAND_BYTES + 1), true);
        assert!(input.key(key(KeyCode::Enter), true).is_none());
        input.key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            true,
        );
        input.paste("ls", true);
        assert_eq!(input.key(key(KeyCode::Enter), true).as_deref(), Some("ls"));
    }
    #[test]
    fn history_deduplicates_updates_recency_and_is_bounded() {
        let mut history = SessionHistory::default();
        for i in 0..105 {
            history.record(&format!("echo {i}"), i);
        }
        assert_eq!(history.entries.len(), SESSION_HISTORY_LIMIT);
        history.record("  echo 5  ", 106);
        assert_eq!(history.entries[0].command, "echo 5");
        assert_eq!(history.entries[0].use_count, 2);
        assert_eq!(history.entries[0].last_used, 106);
        history.record("PGPASSWORD=foo psql", 107);
        history.record("ls\n", 108);
        assert_ne!(history.entries[0].command, "ls");
        assert_eq!(history.entries.len(), SESSION_HISTORY_LIMIT);
        history.clear();
        assert!(history.entries.is_empty());
    }
}
