use super::*;

impl App {
    /// Handle a keystroke while an embedded session is active.
    ///
    /// - `Ctrl+D` closes the active tab (returns to dashboard when the last
    ///   tab closes).
    /// - `Ctrl+T` opens a new tab to the same host (duplicates the current
    ///   session config).
    /// - `Ctrl+W` closes the active tab (alias for Ctrl+D).
    /// - `Ctrl+PgUp` / `Ctrl+PgDn` switch tabs.
    /// - `Esc` during Connecting cancels and returns; after running, forwards.
    /// - `PgUp` / `PgDn` navigate scrollback locally (don't reach the shell).
    /// - Everything else snaps scrollback to live and forwards encoded bytes.
    pub(crate) fn handle_key_session(&mut self, key: KeyEvent) -> Result<()> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Tab management intercepts. These never reach the remote.
        if ctrl {
            match key.code {
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    self.duplicate_active_session();
                    return Ok(());
                }
                KeyCode::Char('w') | KeyCode::Char('W') => {
                    self.close_active_session();
                    return Ok(());
                }
                KeyCode::PageUp => {
                    self.switch_session(-1);
                    return Ok(());
                }
                KeyCode::PageDown => {
                    self.switch_session(1);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Capture self.terminal_area.height before we take a mutable borrow
        // on `session` — borrowck won't let us re-read self after that.
        let body_rows = self.terminal_area.height.saturating_sub(2).max(1) as usize;

        let Some(session) = self.active_session_mut() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        if session.phase.is_terminal() {
            self.close_active_session();
            return Ok(());
        }

        if ctrl && matches!(key.code, KeyCode::Char('d') | KeyCode::Char('D')) {
            self.close_active_session();
            return Ok(());
        }

        if key.code == KeyCode::Esc
            && matches!(
                session.phase,
                crate::session::SessionPhase::Connecting { .. }
            )
        {
            self.close_active_session();
            return Ok(());
        }

        // Local scrollback navigation. Half a screen per press.
        let half = (body_rows / 2).max(1);
        match key.code {
            KeyCode::PageUp => {
                session.parser.scroll_up(half);
                return Ok(());
            }
            KeyCode::PageDown => {
                session.parser.scroll_down(half);
                return Ok(());
            }
            _ => {}
        }

        // Any other key snaps the view back to live and forwards.
        if session.parser.scrollback() > 0 {
            session.parser.snap_to_bottom();
        }
        if let Some(bytes) = crate::session::keys::encode(key) {
            let _ = session.write(&bytes);
        }
        Ok(())
    }

    /// Shared accessor for the visible session, if any.
    pub fn active_session(&self) -> Option<&crate::session::Session> {
        self.active_session.and_then(|i| self.sessions.get(i))
    }

    pub fn active_session_mut(&mut self) -> Option<&mut crate::session::Session> {
        let idx = self.active_session?;
        self.sessions.get_mut(idx)
    }

    /// Tear down the active embedded session and return to the dashboard when
    /// it was the last one — otherwise switch to the next remaining tab.
    pub fn close_active_session(&mut self) {
        let Some(idx) = self.active_session else {
            self.mode = AppMode::Normal;
            return;
        };
        if idx < self.sessions.len() {
            // If we were armed with a secret but never fired, surface what
            // we actually saw on the screen so the user can tell us whether
            // the prompt text didn't match or no prompt arrived at all.
            let session = &mut self.sessions[idx];
            if session.was_armed() && !session.secret_was_sent() {
                let snippet = session.screen_tail_snippet();
                let preview: String = snippet
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("(blank)")
                    .chars()
                    .take(120)
                    .collect();
                let host_name = session.display_name.clone();
                self.push_ssh_log(crate::ssh::probe::SshLogEntry {
                    host_name,
                    line: format!(
                        "auth: armed but no prompt matched. last visible line: {preview:?}"
                    ),
                    level: crate::ssh::probe::LogLevel::Info,
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64,
                });
            }

            // Session::drop kills the child + joins the reader thread.
            self.sessions.remove(idx);
        }
        if self.sessions.is_empty() {
            self.active_session = None;
            self.mode = AppMode::Normal;
        } else {
            // Stay at the same index if possible, else drop back to the new last.
            self.active_session = Some(idx.min(self.sessions.len() - 1));
            self.mode = AppMode::Session;
        }
    }

    /// Spawn a fresh session reusing the active tab's config (same host).
    pub fn duplicate_active_session(&mut self) {
        let Some(idx) = self.active_session else {
            return;
        };
        let Some(active) = self.sessions.get(idx) else {
            return;
        };
        let cfg = active.config.clone();
        let rows = self.terminal_area.height.max(3);
        let cols = self.terminal_area.width.max(20);
        match crate::session::Session::spawn(cfg, rows, cols) {
            Ok(session) => {
                self.sessions.push(session);
                self.active_session = Some(self.sessions.len() - 1);
                self.mode = AppMode::Connecting;
            }
            Err(e) => {
                self.host_notice = Some(format!("New tab failed: {e:#}"));
            }
        }
    }

    /// Cycle tabs by `delta` (`+1` = next, `-1` = prev). Wraps at both ends.
    pub fn switch_session(&mut self, delta: isize) {
        if self.sessions.is_empty() {
            self.active_session = None;
            self.mode = AppMode::Normal;
            return;
        }
        let len = self.sessions.len() as isize;
        let cur = self.active_session.unwrap_or(0) as isize;
        let next = ((cur + delta) % len + len) % len;
        self.active_session = Some(next as usize);

        // Reflect the new active session's phase in app.mode, so render
        // dispatch picks the right path.
        let phase = &self.sessions[next as usize].phase;
        self.mode = match phase {
            crate::session::SessionPhase::Connecting { .. } => AppMode::Connecting,
            _ => AppMode::Session,
        };
    }

    /// Legacy alias retained for tests / callers that explicitly want to end
    /// the whole session stack.
    pub fn end_session(&mut self) {
        self.sessions.clear();
        self.active_session = None;
        self.mode = AppMode::Normal;
    }

    /// Copy the SSH log entries for the selected host to the system clipboard
    /// via OSC 52. Works in kitty / iTerm / wezterm / Alacritty out of the box
    /// without needing an external `xclip`/`pbcopy` dependency.
    pub fn yank_ssh_log(&mut self) -> Result<()> {
        let Some(entry) = self.selected_entry() else {
            return Ok(());
        };
        let host_name = entry.name().to_string();
        let lines: Vec<String> = self
            .ssh_log
            .iter()
            .filter(|e| e.host_name == host_name)
            .map(|e| format!("{} {}", crate::tui::format_local_time(e.timestamp), e.line))
            .collect();

        if lines.is_empty() {
            self.host_notice = Some(format!("no log entries to copy for {host_name}"));
            return Ok(());
        }

        let text = lines.join("\n");
        let n = lines.len();
        match write_osc52(&text) {
            Ok(()) => {
                self.host_notice = Some(format!(
                    "copied {n} log line{} for {host_name} to clipboard",
                    if n == 1 { "" } else { "s" }
                ));
            }
            Err(e) => {
                self.host_notice = Some(format!("clipboard copy failed: {e:#}"));
            }
        }
        Ok(())
    }

    /// Mouse events while in a session. When the remote app has enabled mouse
    /// reporting we forward; otherwise the scroll wheel drives local
    /// scrollback navigation and clicks are dropped.
    pub(crate) fn handle_mouse_session(&mut self, mouse: MouseEvent) {
        let Some(session) = self.active_session_mut() else {
            return;
        };

        let mode = session.parser.screen().mouse_protocol_mode();
        let encoding = session.parser.screen().mouse_protocol_encoding();

        if mode != vt100::MouseProtocolMode::None {
            // Remote app is consuming mouse — translate to the wire protocol.
            // Body starts on row 1 (header takes row 0). Translate the global
            // column / row to body-local coordinates.
            let local_y = mouse.row.saturating_sub(1);
            if let Some(bytes) =
                crate::session::keys::encode_mouse(mouse, mouse.column, local_y, mode, encoding)
            {
                let _ = session.write(&bytes);
            }
            return;
        }

        // No remote mouse handling — local scroll wheel drives scrollback.
        match mouse.kind {
            MouseEventKind::ScrollUp => session.parser.scroll_up(3),
            MouseEventKind::ScrollDown => session.parser.scroll_down(3),
            _ => {}
        }
    }
}
