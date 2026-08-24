use super::*;

impl App {
    /// Handle a keystroke while an embedded session is active.
    ///
    /// Session tab keys are user-configurable (see [`KeyAction::SessionNewTab`]
    /// and friends). `PgUp` / `PgDn` without Ctrl navigate scrollback locally,
    /// except while the remote is on the alternate screen — there the app that
    /// drew it owns paging and the keys are forwarded.
    pub(crate) fn handle_key_session(&mut self, key: KeyEvent) -> Result<()> {
        if self.is_action(KeyAction::LocalShell, &key) {
            self.open_local_shell()?;
            return Ok(());
        }
        if self.is_action(KeyAction::SessionNewTab, &key) {
            self.open_new_session_picker();
            return Ok(());
        }
        if self.is_action(KeyAction::SessionSwitcher, &key) {
            self.open_session_picker(SessionPickerPurpose::SwitchSession);
            return Ok(());
        }
        if self.is_action(KeyAction::SessionCloseTab, &key) {
            self.close_active_session();
            return Ok(());
        }
        if self.is_action(KeyAction::SessionDetach, &key) {
            self.detach_to_dashboard();
            return Ok(());
        }
        if self.is_action(KeyAction::SessionOpenSftp, &key) {
            self.open_sftp_for_active_session();
            return Ok(());
        }
        // While connecting, toggle the debug (`-v`) log view. Only meaningful
        // before the shell reveals, so ignore it once the session is running.
        if self.is_action(KeyAction::SessionToggleLog, &key)
            && matches!(
                self.active_session().map(|s| &s.phase),
                Some(crate::session::SessionPhase::Connecting { .. })
            )
        {
            if let Some(s) = self.active_session_mut() {
                s.toggle_debug_expanded();
            }
            return Ok(());
        }
        if self.is_action(KeyAction::SessionTabPrev, &key) {
            self.switch_session(-1);
            return Ok(());
        }
        if self.is_action(KeyAction::SessionTabNext, &key) {
            self.switch_session(1);
            return Ok(());
        }

        // Accept-changed-host-key flow: when a session has exited because the
        // server's host key changed, `a` purges the stale known_hosts entry and
        // reconnects (which then accept-new's the current key). Any other key
        // falls through to the normal "press any key to close".
        if matches!(key.code, KeyCode::Char('a')) {
            let accept = self.active_session().map(|s| {
                (
                    s.phase.is_terminal() && s.host_key_changed(),
                    s.known_hosts_spec(),
                    s.display_name.clone(),
                )
            });
            if let Some((true, spec, name)) = accept {
                self.close_active_session();
                if let Some(spec) = spec {
                    let _ = std::process::Command::new("ssh-keygen")
                        .args(["-R", &spec])
                        .output();
                }
                if let Some(idx) = self.hosts.iter().position(|h| h.name() == name) {
                    let entry = self.hosts[idx].clone();
                    self.connect_host_entry(entry)?;
                }
                return Ok(());
            }
        }

        // Capture self.terminal_area.height before we take a mutable borrow
        // on `session` — borrowck won't let us re-read self after that.
        let body_rows = self.terminal_area.height.saturating_sub(2).max(1) as usize;

        let cancel_connecting = matches!(
            self.active_session().map(|s| &s.phase),
            Some(crate::session::SessionPhase::Connecting { .. })
        ) && self.is_action(KeyAction::SessionCancel, &key);
        // On the alternate screen the remote app owns paging: tmux, vim and
        // less draw there, and the alternate grid has no scrollback of its own
        // (vt100 gives it zero rows), so stealing PageUp/PageDown for a local
        // scroll made the key do nothing at all. Forward it instead.
        let alternate_screen = self
            .active_session()
            .is_some_and(|s| s.parser.screen().alternate_screen());
        let scroll_up = !alternate_screen && self.is_action(KeyAction::SessionScrollUp, &key);
        let scroll_down = !alternate_screen && self.is_action(KeyAction::SessionScrollDown, &key);

        if cancel_connecting {
            self.close_active_session();
            return Ok(());
        }

        let Some(session) = self.active_session_mut() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        if session.phase.is_terminal() {
            self.close_active_session();
            return Ok(());
        }

        // Local scrollback navigation. Half a screen per press. Keep any active
        // selection anchored to the same text as the view scrolls.
        let half = (body_rows / 2).max(1);
        if scroll_up {
            session.scroll_with_selection(half as i32);
            return Ok(());
        }
        if scroll_down {
            session.scroll_with_selection(-(half as i32));
            return Ok(());
        }

        // Any other key snaps the view back to live, drops the selection, and
        // forwards the keystroke.
        session.selection_clear();
        if session.parser.scrollback() > 0 {
            session.parser.snap_to_bottom();
        }
        let application_cursor = session.parser.screen().application_cursor();
        if let Some(bytes) = crate::session::keys::encode(key, application_cursor) {
            let _ = session.write(&bytes);
        }
        Ok(())
    }

    /// Session-strip keys while on the dashboard with background sessions.
    /// Called from every dashboard tab so the footer hints stay truthful.
    pub(crate) fn handle_key_background_sessions(&mut self, key: &KeyEvent) -> bool {
        if self.sessions.is_empty() {
            return false;
        }
        if self.is_action(KeyAction::LocalShell, key) {
            self.open_local_shell().ok();
            return true;
        }
        // Alt+S from any dashboard tab: the strip is on the header everywhere.
        if self.is_action(KeyAction::SessionSwitcher, key) {
            self.open_session_picker(SessionPickerPurpose::SwitchSession);
            return true;
        }
        if self.is_action(KeyAction::SessionFocus, key) {
            self.focus_active_session();
            return true;
        }
        // Cycling moves the selection along the session strip and stays on the
        // dashboard. Entering the selected session is `SessionFocus`, which sits
        // right next to these in the footer.
        if self.is_action(KeyAction::SessionTabPrev, key) {
            self.switch_session(-1);
            return true;
        }
        if self.is_action(KeyAction::SessionTabNext, key) {
            self.switch_session(1);
            return true;
        }
        if self.is_action(KeyAction::SessionNewTab, key) {
            self.open_new_session_picker();
            return true;
        }
        if self.is_action(KeyAction::SessionCloseTab, key) {
            self.close_active_session();
            return true;
        }
        // Footer "sftp" — real work from any dashboard tab. Detach is not
        // handled here: already on the dashboard, and the footer no longer
        // advertises it (see session_footer_hints).
        if self.is_action(KeyAction::SessionOpenSftp, key) {
            self.open_sftp_for_active_session();
            return true;
        }
        false
    }

    /// Shared accessor for the visible session, if any.
    pub fn active_session(&self) -> Option<&crate::session::Session> {
        self.active_session.and_then(|i| self.sessions.get(i))
    }

    pub fn active_session_mut(&mut self) -> Option<&mut crate::session::Session> {
        let idx = self.active_session?;
        self.sessions.get_mut(idx)
    }

    /// Does this frame hand the whole screen to the session renderer?
    ///
    /// The single source of truth for "a session is on screen": `render_inner`
    /// draws from it, and the OSC 52 relay gate reads it to decide whether a
    /// PTY may reach the host clipboard at all. The session picker draws on top
    /// of a session that keeps rendering behind it, so that case counts.
    pub(crate) fn session_is_rendered(&self) -> bool {
        matches!(self.mode, AppMode::Connecting | AppMode::Session)
            || self.session_picker_over_session()
    }

    /// The session picker opened from a session (rather than the dashboard), so
    /// the session is still painted underneath it.
    pub(crate) fn session_picker_over_session(&self) -> bool {
        self.mode == AppMode::SessionPicker
            && self
                .session_picker
                .as_ref()
                .is_some_and(|p| matches!(p.return_mode, AppMode::Connecting | AppMode::Session))
    }

    /// Index of the session actually painted this frame, if any.
    pub(crate) fn visible_session_idx(&self) -> Option<usize> {
        self.session_is_rendered()
            .then_some(self.active_session)
            .flatten()
    }

    /// Per-frame OSC 52 handling for every session's PTY.
    ///
    /// Only the session the user is looking at may put something on the host
    /// clipboard, and only while `clipboard.relay_from_pty` is on. Every other
    /// session discards what it saw immediately: a background tab must not be
    /// able to change the clipboard now, and must not replay an old write when
    /// it later comes to the front.
    pub(crate) fn relay_visible_session_clipboard(&mut self) {
        self.relay_pty_clipboard_with(crate::osc52::write_b64);
    }

    /// [`Self::relay_visible_session_clipboard`] with an injectable sink, so
    /// tests can watch the relay without touching the real clipboard.
    pub(crate) fn relay_pty_clipboard_with(
        &mut self,
        emit: impl FnMut(&str) -> std::io::Result<()>,
    ) {
        let relaying = self
            .config
            .clipboard
            .relay_from_pty
            .then(|| self.visible_session_idx())
            .flatten();
        let mut emit = emit;
        for (i, session) in self.sessions.iter_mut().enumerate() {
            if Some(i) == relaying {
                session.relay_clipboard_writes_with(&mut emit);
            } else {
                session.discard_clipboard_writes();
            }
        }
    }

    /// Return to the dashboard without tearing down background sessions.
    pub fn detach_to_dashboard(&mut self) {
        if self.sessions.is_empty() {
            self.mode = AppMode::Normal;
            return;
        }
        self.mode = AppMode::Normal;
    }

    /// Re-enter the active embedded session from the dashboard.
    pub fn focus_active_session(&mut self) {
        let Some(idx) = self.active_session else {
            if self.sessions.is_empty() {
                self.mode = AppMode::Normal;
            }
            return;
        };
        // Entering from outside, rather than being re-derived while already in a
        // session (an overlay closing over it, a phase change), is what earns the
        // slide: leaving already animates, so arriving looked like a cut.
        let entering = !is_session_mode(self.mode);
        let phase = &self.sessions[idx].phase;
        self.mode = match phase {
            crate::session::SessionPhase::Connecting { .. } => AppMode::Connecting,
            _ => AppMode::Session,
        };
        if entering && self.motion_enabled() {
            self.session_enter_at = Some(std::time::Instant::now());
        }
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
            let next = idx.min(self.sessions.len() - 1);
            // The tab that takes over sat to the right of the closed one, unless
            // the closed one was last — then we fall back to its left neighbour
            // and the slide has to travel the other way (#35).
            if self.mode != AppMode::Normal {
                // The closed tab is gone from the strip, so the highlight
                // travels from where its neighbour now sits.
                self.arm_session_tab_switch(if next == idx { 1 } else { -1 }, next);
            }
            self.active_session = Some(next);
            let phase = &self.sessions[self.active_session.unwrap()].phase;
            self.mode = if self.mode == AppMode::Normal {
                AppMode::Normal
            } else {
                match phase {
                    crate::session::SessionPhase::Connecting { .. } => AppMode::Connecting,
                    _ => AppMode::Session,
                }
            };
        }
    }

    /// Arm the tab slide that carries the tab at `from` off in direction `dir`
    /// (`+1` = the new tab arrives from the right) while the strip's highlight
    /// travels with it (#35).
    ///
    /// The single place that gates the transition on reduced motion and stamps
    /// it, so every path that retargets `active_session` — cycling, closing a
    /// tab, the switcher — animates identically. `dir` stays a parameter
    /// because it is not always the sign of the index delta: the strip wraps,
    /// so cycling past either end travels the way the key pointed.
    pub(crate) fn arm_session_tab_switch(&mut self, dir: i8, from: usize) {
        if !self.motion_enabled() {
            return;
        }
        self.session_tab_switch = Some(SessionTabSwitch {
            dir,
            from,
            at: std::time::Instant::now(),
        });
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

        // Carry the tab we're leaving off in the direction of travel (#35). The
        // strip wraps, so the direction comes from `delta`, not the indices.
        if next != cur {
            self.arm_session_tab_switch(if delta > 0 { 1 } else { -1 }, cur as usize);
        }

        // On the dashboard only the strip moves; the mode below would drag the
        // user into the session they were merely scrolling past.
        if self.mode == AppMode::Normal {
            return;
        }

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
        self.shutdown_all();
        self.mode = AppMode::Normal;
    }

    /// Kill every embedded SSH child and clear tab state. Called on quit and
    /// from [`Drop`] so detached sessions never outlive the app.
    pub fn shutdown_all(&mut self) {
        self.sessions.clear();
        self.active_session = None;
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
        let alternate_screen = session.parser.screen().alternate_screen();
        let (rows, cols) = session.parser.screen().size();

        // Body-local grid coordinates (header takes row 0).
        let row = mouse.row.saturating_sub(1).min(rows.saturating_sub(1));
        let col = mouse.column.min(cols.saturating_sub(1));
        let shift = mouse.modifiers.contains(KeyModifiers::SHIFT);

        // Local text selection drives the mouse when the remote app isn't
        // consuming it (plain shell → just drag to select, no Shift needed);
        // Shift inverts that decision in either direction. See
        // [`crate::session::keys::selects_locally`] for why the mouse mode
        // alone does not decide it.
        let selecting = crate::session::keys::selects_locally(mode, alternate_screen, shift);

        if selecting {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => session.selection_start(row, col),
                MouseEventKind::Drag(MouseButton::Left) => {
                    // Arm edge autoscroll when the pointer is dragged past the
                    // top/bottom of the grid, so a selection can extend beyond
                    // what's currently visible (the poll tick keeps it going).
                    let dir = crate::session::edge_autoscroll_dir(mouse.row as i32 - 1, rows);
                    session.set_drag_autoscroll(dir, col);
                    session.selection_extend(row, col);
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    if let Some(text) = session.selection_finish() {
                        let chars = text.chars().count();
                        match write_osc52(&text) {
                            Ok(()) => session.set_copy_notice(format!("copied {chars} chars")),
                            Err(e) => session.set_copy_notice(format!("copy failed: {e}")),
                        }
                    }
                }
                // xterm's alternateScroll: the alternate grid keeps no
                // scrollback, so the notch becomes arrow keys for the app that
                // owns the screen instead of a local scroll that can move
                // nothing. Shift opts out — it is the wheel's way back to our
                // own scrollback, which is what it means in a real terminal.
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                    if alternate_screen && !shift =>
                {
                    let app_cursor = session.parser.screen().application_cursor();
                    if let Some(bytes) =
                        crate::session::keys::alternate_scroll_keys(mouse.kind, app_cursor)
                    {
                        let _ = session.write(&bytes);
                        // Arrow keys are all a terminal can do here, and in tmux
                        // they land in the shell's history rather than scrolling
                        // anything — so say once where the real switch is.
                        session.hint_alternate_scroll();
                    }
                }
                MouseEventKind::ScrollUp => session.scroll_with_selection(3),
                MouseEventKind::ScrollDown => session.scroll_with_selection(-3),
                // Any other press clears a pending selection.
                MouseEventKind::Down(_) => session.selection_clear(),
                _ => {}
            }
            return;
        }

        // Remote app is consuming mouse — translate to the wire protocol.
        // Only events inside the grid go out: row 0 is our own header and
        // anything past the last row is the footer, and reporting either as an
        // edge row of the grid made a stray click on sshub's chrome land on the
        // remote (tmux's status line with `status-position top`, for one).
        if mouse.row == 0 || mouse.row.saturating_sub(1) >= rows || mouse.column >= cols {
            return;
        }
        if let Some(bytes) = crate::session::keys::encode_mouse(mouse, col, row, mode, encoding) {
            let _ = session.write(&bytes);
        }
    }
}
