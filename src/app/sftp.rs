use super::*;

use std::path::{Path, PathBuf};

use crate::sftp::model::{Direction, FileEntry, Phase, QueuedTransfer, SftpState, Side};

/// Time constant of the SFTP progress bar's chase (#35).
const SFTP_PROGRESS_TAU: f32 = 0.12;

/// `ui_state` key holding whether SFTP lists dotfiles.
const SFTP_HIDDEN_KEY: &str = "sftp_show_hidden";
use crate::sftp::SftpCommand;

impl App {
    /// Arm an SFTP tab sub-state slide (#35). A no-op under reduced motion,
    /// where each sub-state simply appears at rest.
    fn stamp_sftp_anim(&mut self, kind: SftpAnim) {
        if self.motion_enabled() {
            self.sftp_anim = Some((kind, std::time::Instant::now()));
        }
    }

    /// Advance the SFTP progress bar toward `target` (0.0 to 1.0) and return
    /// what to draw (#35).
    ///
    /// The worker reports progress in chunks, so the raw figure steps; the bar
    /// closes on it continuously and keeps sweeping between updates. Reset to
    /// the target outright when it moves backwards, which means the queue has
    /// moved on to the next (smaller) file rather than made negative progress.
    /// Called once per frame from the render pass.
    pub(crate) fn sftp_progress_advance(&self, target: f32) -> f32 {
        let target = target.clamp(0.0, 1.0);
        if !self.motion_enabled() {
            self.sftp_progress_pos.set(target);
            self.sftp_progress_moving.set(false);
            return target;
        }
        let now = std::time::Instant::now();
        let last = self.sftp_progress_at.replace(Some(now));
        let pos = self.sftp_progress_pos.get();
        let dist = target - pos;
        if last.is_none() || dist < 0.0 || dist < 0.002 {
            self.sftp_progress_pos.set(target);
            self.sftp_progress_moving.set(false);
            return target;
        }
        let dt = now.saturating_duration_since(last.unwrap()).as_secs_f32();
        let next = pos + dist * (1.0 - (-dt / SFTP_PROGRESS_TAU).exp());
        self.sftp_progress_pos.set(next);
        self.sftp_progress_moving.set(true);
        next
    }

    /// Notice either SFTP pane changing directory and stamp which way it went
    /// (#35), so its listing can slide in from the matching side. Detected
    /// centrally because a remote change lands asynchronously, on the worker's
    /// `DirListing`, rather than when the key was pressed.
    pub(crate) fn detect_sftp_navigation(&mut self) {
        let cwds = match self.sftp.as_ref() {
            Some(s) => [s.local.cwd.clone(), s.remote.cwd.clone()],
            // No session: forget the old paths so reconnecting doesn't read as
            // a navigation.
            None => {
                self.anim_prev_cwd = [PathBuf::new(), PathBuf::new()];
                self.sftp_nav = [None, None];
                return;
            }
        };
        for (i, cwd) in cwds.into_iter().enumerate() {
            if cwd == self.anim_prev_cwd[i] {
                continue;
            }
            // Descending into a child goes one way, anything else (parent, or a
            // jump elsewhere) the other. The first listing of a fresh session
            // has no previous path and doesn't animate.
            let fresh = self.anim_prev_cwd[i].as_os_str().is_empty();
            let deeper = cwd.starts_with(&self.anim_prev_cwd[i]);
            self.anim_prev_cwd[i] = cwd;
            self.sftp_nav[i] =
                (!fresh && self.motion_enabled()).then(|| (deeper, std::time::Instant::now()));
        }
    }

    /// Switch to the SFTP tab (index 1). Setter mirror of the other
    /// `switch_to_*_tab` helpers; kept dead-simple because the SFTP tab has no
    /// eager data to refresh (the picker just reuses the host list).
    pub fn switch_to_sftp_tab(&mut self) {
        self.active_tab = 1;
    }

    /// SFTP tab key dispatch. `try_tab_switch` runs first so the tab digits
    /// (`1`-`5`) still work while this tab is focused, exactly like the other
    /// dashboard tabs. Then we branch on whether a live browser session exists:
    /// no `app.sftp` → the host **picker**; `Some` → the dual-pane **browser**.
    pub fn handle_key_sftp(&mut self, key: KeyEvent) -> Result<()> {
        // While a search input is active, capture keys BEFORE try_tab_switch so
        // typed letters that happen to be tab-switch binds (h, i, 1-5) filter
        // instead of switching tabs.
        if self.sftp_picker_searching {
            return self.handle_key_sftp_picker_search(key);
        }
        if self.sftp.as_ref().is_some_and(|s| s.searching) {
            return self.handle_key_sftp_browser_search(key);
        }
        if self.try_tab_switch(&key)? {
            return Ok(());
        }
        if self.sftp.is_none() {
            self.handle_key_sftp_picker(key)
        } else {
            self.handle_key_sftp_browser(key)
        }
    }

    fn handle_key_sftp_picker(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            _ if self.is_action(KeyAction::Cancel, &key) => {
                self.active_tab = 0;
            }
            _ if self.is_action(KeyAction::MoveGroupUp, &key) => self.move_selection_by_group(-1),
            _ if self.is_action(KeyAction::MoveGroupDown, &key) => self.move_selection_by_group(1),
            _ if self.is_action(KeyAction::MoveDown, &key) => self.move_selection(1),
            _ if self.is_action(KeyAction::MoveUp, &key) => self.move_selection(-1),
            _ if self.is_action(KeyAction::ToggleGroup, &key) => self.toggle_selected_group(),
            _ if self.is_action(KeyAction::FoldGroupIn, &key) => {
                if self
                    .selected_nav_header()
                    .is_some_and(|si| !self.group_sections[si].collapsed)
                {
                    self.toggle_selected_group();
                }
            }
            _ if self.is_action(KeyAction::FoldGroupOut, &key) => {
                if self
                    .selected_nav_header()
                    .is_some_and(|si| self.group_sections[si].collapsed)
                {
                    self.toggle_selected_group();
                }
            }
            _ if self.is_action(KeyAction::CollapseAll, &key) => {
                let all_collapsed = !self.group_sections.is_empty()
                    && self.group_sections.iter().all(|s| s.collapsed);
                self.set_all_groups_collapsed(!all_collapsed);
            }
            // On a group header, Enter folds the group (matches the hosts tab);
            // on a host row it connects via SFTP.
            _ if self.selected_nav_header().is_some()
                && self.is_action(KeyAction::Connect, &key) =>
            {
                self.toggle_selected_group();
            }
            _ if self.is_action(KeyAction::Connect, &key) => self.sftp_connect_selected()?,
            _ if self.is_action(KeyAction::Search, &key) => {
                self.sftp_picker_searching = true;
                self.search_query.clear();
                self.rebuild_filter();
            }
            _ if self.is_action(KeyAction::Help, &key) => {
                self.open_help();
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_key_sftp_picker_search(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            _ if self.is_action(KeyAction::Cancel, &key) => {
                self.sftp_picker_searching = false;
                self.search_query.clear();
                self.rebuild_filter();
            }
            _ if self.is_action(KeyAction::Connect, &key) => {
                self.sftp_picker_searching = false;
                if self.selected_nav_header().is_some() {
                    self.toggle_selected_group();
                } else {
                    self.sftp_connect_selected()?;
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.search_query.push(c);
                self.rebuild_filter();
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.rebuild_filter();
            }
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            _ => {}
        }
        Ok(())
    }

    fn handle_key_sftp_browser(&mut self, key: KeyEvent) -> Result<()> {
        let running = self
            .sftp
            .as_ref()
            .is_some_and(|s| s.phase == crate::sftp::model::Phase::Running);
        // The local pane stays browsable during a run (its listings are read
        // straight off the filesystem). The remote one doesn't: the worker
        // handles commands in order, so a listing queued behind a transfer
        // would leave the pane looking hung until the transfer finished.
        let can_navigate = !running
            || self
                .sftp
                .as_ref()
                .is_some_and(|s| s.focused_side() == Side::Local);
        // Esc / Cancel disconnects the live session back to the picker.
        if self.is_action(KeyAction::Cancel, &key) {
            self.sftp_disconnect();
            return Ok(());
        }
        // Enter descends into the selected directory of the focused pane.
        if can_navigate && self.is_action(KeyAction::Connect, &key) {
            if let Some((side, path)) = self.sftp.as_ref().and_then(|s| s.enter_dir()) {
                self.sftp_navigate(side, path);
            }
            return Ok(());
        }
        if self.is_action(KeyAction::MoveDown, &key) {
            if let Some(s) = self.sftp.as_mut() {
                s.move_selection(1);
            }
            return Ok(());
        }
        if self.is_action(KeyAction::MoveUp, &key) {
            if let Some(s) = self.sftp.as_mut() {
                s.move_selection(-1);
            }
            return Ok(());
        }
        if self.is_action(KeyAction::Help, &key) {
            self.open_help();
            return Ok(());
        }

        match key.code {
            KeyCode::Tab => {
                if let Some(s) = self.sftp.as_mut() {
                    s.toggle_focus();
                }
            }
            KeyCode::Backspace => {
                if can_navigate {
                    if let Some((side, path)) = self.sftp.as_ref().and_then(|s| s.parent_dir()) {
                        self.sftp_navigate(side, path);
                    }
                }
            }
            // Panes are left=local, right=remote, so the arrow points at the
            // destination pane and the source is the focused one: ← downloads
            // (remote → local), → uploads (local → remote).
            // Staging stays open while a run is in flight: whatever is added
            // rolls into the next pass when this one finishes.
            KeyCode::Left => {
                if let Some(s) = self.sftp.as_mut() {
                    let _ = s.stage_toward(Side::Local);
                }
            }
            KeyCode::Right => {
                if let Some(s) = self.sftp.as_mut() {
                    let _ = s.stage_toward(Side::Remote);
                }
            }
            // Remove the most recently staged transfer from the queue.
            KeyCode::Char('u') => {
                if let Some(s) = self.sftp.as_mut() {
                    let n = s.queue.len();
                    if n > 0 {
                        s.unstage(n - 1);
                    }
                }
            }
            KeyCode::Char('/') => {
                if let Some(s) = self.sftp.as_mut() {
                    s.start_search();
                }
            }
            // Re-list both panes (pick up files changed on either side).
            KeyCode::Char('r') => self.sftp_refresh_panes(),
            // Open an SSH session to this same host (SFTP stays in the background).
            KeyCode::Char('s') => self.open_ssh_for_sftp_host()?,
            // Show or hide dotfiles in both panes.
            KeyCode::Char('.') => self.sftp_toggle_hidden(),
            // Point the left pane at a second server, or send it back to the
            // local filesystem.
            KeyCode::Char('o') => self.open_host_picker(PickerTarget::SftpLeftPane),
            KeyCode::Char('O') => self.sftp_left_pane_to_local(),
            // Confirm: run the whole queue sequentially.
            KeyCode::Char('c') => self.sftp_run_queue(),
            // File ops (frozen while a queue runs).
            KeyCode::Char('d') if !running => self.sftp_arm_delete(),
            KeyCode::Char('n') if !running => self.sftp_open_prompt(SftpPromptKind::Mkdir),
            KeyCode::Char('R') if !running => self.sftp_open_prompt(SftpPromptKind::Rename),
            KeyCode::Char('M') if !running => self.sftp_open_prompt(SftpPromptKind::Chmod),
            _ => {}
        }
        Ok(())
    }

    /// Arm the delete confirmation for the focused pane's selection. Reuses the
    /// shared `ConfirmDelete` dialog via a `PendingDelete::SftpEntry`.
    fn sftp_arm_delete(&mut self) {
        let Some(s) = self.sftp.as_ref() else { return };
        let side = s.focused_side();
        let pane = s.focused_pane();
        let Some(entry) = pane.selected_entry().filter(|e| !e.is_parent()) else {
            return;
        };
        let path = pane.cwd.join(&entry.name);
        self.pending_delete = Some(PendingDelete::SftpEntry {
            side,
            path,
            name: entry.name.clone(),
            is_dir: entry.is_dir,
        });
        self.mode = AppMode::ConfirmDelete;
    }

    /// Open the mkdir / rename text prompt for the focused pane.
    fn sftp_open_prompt(&mut self, kind: SftpPromptKind) {
        let Some(s) = self.sftp.as_ref() else { return };
        let side = s.focused_side();
        let pane = s.focused_pane();
        let base = pane.cwd.clone();
        let (value, old_path) = match kind {
            SftpPromptKind::Mkdir => (String::new(), None),
            SftpPromptKind::Rename => {
                let Some(entry) = pane.selected_entry().filter(|e| !e.is_parent()) else {
                    return;
                };
                (entry.name.clone(), Some(base.join(&entry.name)))
            }
            SftpPromptKind::Chmod => {
                let Some(entry) = pane.selected_entry().filter(|e| !e.is_parent()) else {
                    return;
                };
                // Seed with the current octal permissions so the user edits from
                // the existing value; default to 644 when unknown.
                let octal = format!("{:o}", entry.perm.unwrap_or(0o644) & 0o7777);
                (octal, Some(base.join(&entry.name)))
            }
        };
        let cursor = value.chars().count();
        self.sftp_prompt = Some(SftpPromptEdit {
            kind,
            side,
            base,
            old_path,
            value,
            cursor,
            error: None,
        });
        self.mode = AppMode::SftpPrompt;
    }

    fn sftp_prompt_insert(&mut self, ch: char) {
        if let Some(p) = self.sftp_prompt.as_mut() {
            p.cursor = text_input::insert_at(&mut p.value, p.cursor, ch);
            p.error = None;
        }
    }

    fn sftp_prompt_backspace(&mut self) {
        if let Some(p) = self.sftp_prompt.as_mut() {
            p.cursor = text_input::backspace_at(&mut p.value, p.cursor);
            p.error = None;
        }
    }

    pub(crate) fn handle_key_sftp_prompt(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.sftp_prompt = None;
                self.mode = AppMode::Normal;
            }
            KeyCode::Enter => self.sftp_prompt_commit(),
            KeyCode::Backspace if key.modifiers.is_empty() => self.sftp_prompt_backspace(),
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End | KeyCode::Delete => {
                if let Some(p) = self.sftp_prompt.as_mut() {
                    let mut cursor = p.cursor;
                    text_input::handle_cursor_key(key.code, &mut p.value, &mut cursor);
                    p.cursor = cursor;
                    if key.code == KeyCode::Delete {
                        p.error = None;
                    }
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.sftp_prompt_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    /// Apply the open mkdir / rename prompt. Remote ops are dispatched to the
    /// worker and the prompt closes immediately (the result surfaces as a pane
    /// refresh on OpDone or a browser notice on Error — we deliberately do NOT
    /// keep the prompt waiting on an async event, because OpDone/Error carry no
    /// op identity and would cross-attribute a concurrent delete's result).
    /// Local ops use `std::fs`. Rejects empty names / path separators; never
    /// clobbers an existing target.
    fn sftp_prompt_commit(&mut self) {
        let Some(p) = self.sftp_prompt.as_ref() else {
            return;
        };
        // chmod takes an octal mode, not a name — handle it separately.
        if p.kind == SftpPromptKind::Chmod {
            self.sftp_chmod_commit();
            return;
        }
        let name = p.value.trim().to_string();
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            if let Some(p) = self.sftp_prompt.as_mut() {
                p.error = Some("enter a name without '/'".into());
            }
            return;
        }
        let kind = p.kind;
        let side = p.side;
        let target = p.base.join(&name);
        let old_path = p.old_path.clone();

        // Rename to the (unchanged) current name is a no-op — close the prompt
        // instead of dispatching a from==to rename that the clobber guard would
        // reject or the server would error on.
        if kind == SftpPromptKind::Rename && old_path.as_ref() == Some(&target) {
            self.sftp_prompt = None;
            self.mode = AppMode::Normal;
            return;
        }

        // A pane pointed at a server routes through its worker; only a truly
        // local pane touches the filesystem here.
        if self.sftp_channel(side).is_some() {
            let cmd = match kind {
                SftpPromptKind::Mkdir => crate::sftp::SftpCommand::Mkdir(target),
                SftpPromptKind::Rename => {
                    crate::sftp::SftpCommand::Rename(old_path.unwrap_or_default(), target)
                }
                SftpPromptKind::Chmod => unreachable!("chmod handled earlier"),
            };
            // A missing channel OR a failed send (worker thread dead) means
            // the op won't run — keep the prompt open with an error rather
            // than closing it as if it succeeded.
            let sent = self
                .sftp_channel(side)
                .map(|tx| tx.send(cmd).is_ok())
                .unwrap_or(false);
            if sent {
                self.sftp_prompt = None;
                self.mode = AppMode::Normal;
                self.note_if_hidden(&name);
            } else if let Some(p) = self.sftp_prompt.as_mut() {
                p.error = Some("not connected".into());
            }
            return;
        }

        match side {
            Side::Remote => unreachable!("the remote pane always has a channel"),
            Side::Local => {
                let result: std::io::Result<()> = match kind {
                    SftpPromptKind::Mkdir => std::fs::create_dir(&target),
                    SftpPromptKind::Rename => {
                        // Refuse to clobber an existing target — matches the
                        // remote path (rename with None flags). symlink_metadata
                        // (not exists()) so a dangling symlink at the target still
                        // counts as present and isn't silently overwritten.
                        if target.symlink_metadata().is_ok() {
                            Err(std::io::Error::new(
                                std::io::ErrorKind::AlreadyExists,
                                format!("{} already exists", name),
                            ))
                        } else {
                            std::fs::rename(old_path.unwrap_or_default(), &target)
                        }
                    }
                    SftpPromptKind::Chmod => unreachable!("chmod handled earlier"),
                };
                match result {
                    Ok(()) => {
                        self.sftp_prompt = None;
                        self.mode = AppMode::Normal;
                        self.sftp_refresh_panes();
                        self.note_if_hidden(&name);
                    }
                    Err(e) => {
                        if let Some(p) = self.sftp_prompt.as_mut() {
                            p.error = Some(format!("{e}"));
                        }
                    }
                }
            }
        }
    }

    /// Apply the chmod prompt: parse the octal mode and set it on `old_path`
    /// (remote via the worker, local via `std::fs::set_permissions`).
    fn sftp_chmod_commit(&mut self) {
        let Some(p) = self.sftp_prompt.as_ref() else {
            return;
        };
        let mode = match u32::from_str_radix(p.value.trim(), 8) {
            Ok(m) if m <= 0o7777 => m,
            _ => {
                if let Some(p) = self.sftp_prompt.as_mut() {
                    p.error = Some("enter octal permissions, e.g. 755".into());
                }
                return;
            }
        };
        let side = p.side;
        let Some(path) = p.old_path.clone() else {
            return;
        };

        if self.sftp_channel(side).is_some() {
            let sent = self
                .sftp_channel(side)
                .map(|tx| tx.send(crate::sftp::SftpCommand::Chmod(path, mode)).is_ok())
                .unwrap_or(false);
            if sent {
                self.sftp_prompt = None;
                self.mode = AppMode::Normal;
            } else if let Some(p) = self.sftp_prompt.as_mut() {
                p.error = Some("not connected".into());
            }
            return;
        }

        match side {
            Side::Remote => unreachable!("the remote pane always has a channel"),
            Side::Local => {
                use std::os::unix::fs::PermissionsExt;
                match std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)) {
                    Ok(()) => {
                        self.sftp_prompt = None;
                        self.mode = AppMode::Normal;
                        self.sftp_refresh_panes();
                    }
                    Err(e) => {
                        if let Some(p) = self.sftp_prompt.as_mut() {
                            p.error = Some(format!("{e}"));
                        }
                    }
                }
            }
        }
    }

    /// Execute a confirmed SFTP delete (called from the ConfirmDelete handler).
    /// Remote deletes go through the worker; local deletes use `std::fs`.
    pub(crate) fn sftp_delete_confirmed(&mut self, side: Side, path: PathBuf, is_dir: bool) {
        match side {
            Side::Remote => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // Like mkdir/rename: a missing channel or a failed send (dead
                // worker) means the delete never dispatched — say so instead of
                // silently returning as if the file were gone.
                let parent = path.parent().map(Path::to_path_buf);
                let sent = self
                    .sftp_tx
                    .as_ref()
                    .map(|tx| {
                        tx.send(crate::sftp::SftpCommand::Remove(path, is_dir))
                            .is_ok()
                    })
                    .unwrap_or(false);
                if sent {
                    // Optimistically drop the row so it disappears immediately
                    // (and can't be re-deleted) before the async OpDone refresh —
                    // but only if the pane still shows the directory the delete
                    // targeted: an in-flight listing may have replaced it, and
                    // remove_named matches by name only.
                    if let Some(s) = self.sftp.as_mut() {
                        if parent.as_deref() == Some(s.remote.cwd.as_path()) {
                            s.remote.remove_named(&name);
                        }
                    }
                } else if let Some(s) = self.sftp.as_mut() {
                    s.notice = Some("not connected — delete not sent".into());
                } else {
                    // Session torn down while the confirm dialog was open: the
                    // SFTP pane is gone, so surface the failure where the user
                    // will actually see it.
                    self.host_notice = Some("SFTP disconnected — delete not sent".into());
                }
            }
            // Left pane on a second server: same worker round trip as the right.
            Side::Local if self.sftp.as_ref().is_some_and(|s| s.left_is_remote()) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let sent = self
                    .sftp_tx2
                    .as_ref()
                    .map(|tx| tx.send(SftpCommand::Remove(path, is_dir)).is_ok())
                    .unwrap_or(false);
                if let Some(s) = self.sftp.as_mut() {
                    if sent {
                        s.local.remove_named(&name);
                    } else {
                        s.notice = Some("not connected — delete not sent".into());
                    }
                }
            }
            Side::Local => {
                let res = if is_dir {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                if let Some(s) = self.sftp.as_mut() {
                    if let Err(e) = res {
                        s.notice = Some(format!("{e}"));
                    }
                }
                self.sftp_refresh_panes();
            }
        }
    }

    fn handle_key_sftp_browser_search(&mut self, key: KeyEvent) -> Result<()> {
        let Some(s) = self.sftp.as_mut() else {
            return Ok(());
        };
        match key.code {
            KeyCode::Esc => s.search_cancel(),
            KeyCode::Enter => s.search_confirm(),
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                s.search_push(c)
            }
            KeyCode::Backspace => s.search_backspace(),
            KeyCode::Up => s.move_selection(-1),
            KeyCode::Down => s.move_selection(1),
            _ => {}
        }
        Ok(())
    }

    fn sftp_connect_selected(&mut self) -> Result<()> {
        // Read the selection BEFORE touching the filter: clearing the search
        // query rebuilds the visible list, which would remap the selected index
        // onto a different (unfiltered) host and connect to the wrong one.
        let entry = self.selected_entry().cloned();
        self.sftp_picker_searching = false;
        // Picker search reuses the shared host filter; clear it so a leftover
        // query doesn't silently filter the hosts tab after we connect.
        self.search_query.clear();
        self.rebuild_filter();
        let Some(entry) = entry else {
            return Ok(());
        };
        self.sftp_connect_to(entry)
    }

    /// Detach the active SSH session to the background and open the SFTP tab
    /// connected to that same host (found by name in the host list). If an SFTP
    /// session is already live, just switch to the tab and leave it as-is.
    pub(crate) fn open_sftp_for_active_session(&mut self) {
        let Some(name) = self.active_session().map(|s| s.display_name.clone()) else {
            return;
        };
        let Some(entry) = self.hosts.iter().find(|h| h.name() == name).cloned() else {
            self.host_notice = Some(format!("no saved host '{name}' to open SFTP for"));
            return;
        };
        self.detach_to_dashboard();
        self.active_tab = 1;
        if self.sftp.is_none() {
            let _ = self.sftp_connect_to(entry);
        }
    }

    /// Spawn the worker for a specific host entry and enter the browser. Refuses
    /// ProxyJump hosts (unsupported by the libssh2 transport in v1) with a
    /// notice instead of a doomed connection attempt.
    fn sftp_connect_to(&mut self, entry: HostEntry) -> Result<()> {
        let ssh_host = match &entry {
            HostEntry::Managed(m) => managed_to_ssh_host(m),
            HostEntry::Legacy { host, .. } => host.clone(),
        };

        if ssh_host.proxy_jump.is_some() {
            self.host_notice =
                Some("SFTP via ProxyJump isn't supported yet — pick a direct host.".into());
            return Ok(());
        }

        let (secret, _diag) = resolve_pending_secret(&entry, self.password_store.as_ref());
        let agent = crate::ssh::agent::detect_agent();
        let (tx, rx) = crate::sftp::spawn_sftp_worker(ssh_host, secret, agent);

        // Remote starts relative to the login dir (".", resolved by the server);
        // local mirrors the process cwd.
        let remote_cwd = PathBuf::from(".");
        let local_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        let mut state = SftpState::new(remote_cwd.clone(), local_cwd.clone());
        // Show a "connecting…" state (not an empty browser) until the worker
        // reports Connected. An unreachable host never flashes a blank window —
        // it fails into the Notice popup instead.
        state.connecting = true;
        state.local.set_entries(read_local_dir(&local_cwd));

        // Kick off the first remote listing; the worker queues it until the
        // connection completes.
        let _ = tx.send(SftpCommand::ListDir(Side::Remote, remote_cwd));

        self.sftp = Some(state);
        self.sftp_tx = Some(tx);
        self.sftp_rx = Some(rx);
        self.sftp_host = Some(entry.name().to_string());
        self.apply_saved_hidden();
        // Slide the "connecting…" layer in from the right over the picker (#35).
        self.stamp_sftp_anim(SftpAnim::ConnectIn);
        Ok(())
    }

    /// Read the remembered dotfile setting at startup.
    pub(crate) fn load_sftp_hidden(&mut self) {
        if let Ok(Some(raw)) = self.store.get_ui_state(SFTP_HIDDEN_KEY) {
            self.sftp_show_hidden = raw == "1";
        }
    }

    /// Warn when an entry just created or renamed lands under the dotfile
    /// filter: it would otherwise vanish from the listing on success, which
    /// reads as the operation having failed.
    fn note_if_hidden(&mut self, name: &str) {
        if self.sftp_show_hidden || !name.starts_with('.') {
            return;
        }
        if let Some(s) = self.sftp.as_mut() {
            s.notice = Some(format!("{name} is hidden — press . to show dotfiles"));
        }
    }

    /// Flip dotfile visibility in both panes and remember it, so the choice
    /// survives a restart the way collapsed host groups do.
    fn sftp_toggle_hidden(&mut self) {
        let Some(show) = self.sftp.as_mut().map(|s| s.toggle_hidden()) else {
            return;
        };
        self.sftp_show_hidden = show;
        let _ = self
            .store
            .set_ui_state(SFTP_HIDDEN_KEY, if show { "1" } else { "0" });
    }

    /// Restore the remembered dotfile setting into a freshly opened browser.
    fn apply_saved_hidden(&mut self) {
        let show = self.sftp_show_hidden;
        if let Some(s) = self.sftp.as_mut() {
            s.local.show_hidden = show;
            s.remote.show_hidden = show;
        }
    }

    /// The worker that owns `side`: the browser's own for the right pane, the
    /// second one for a left pane pointed at another server, and `None` for a
    /// left pane showing the local filesystem (which needs no worker).
    fn sftp_channel(&self, side: Side) -> Option<&std::sync::mpsc::Sender<SftpCommand>> {
        match side {
            Side::Remote => self.sftp_tx.as_ref(),
            Side::Local if self.sftp.as_ref().is_some_and(|s| s.left_is_remote()) => {
                self.sftp_tx2.as_ref()
            }
            Side::Local => None,
        }
    }

    /// Point the left pane at a second server, so two remote hosts can be
    /// browsed side by side (the right pane keeps its own session).
    ///
    /// Runs its own worker: libssh2 has no server-to-server copy, so the two
    /// ends stay independent connections and a transfer between them is
    /// relayed locally.
    pub(crate) fn sftp_connect_left_pane(&mut self, host_idx: usize) -> Result<()> {
        let Some(entry) = self.hosts.get(host_idx).cloned() else {
            return Ok(());
        };
        if self.sftp.is_none() {
            self.host_notice = Some("connect the SFTP browser first".into());
            return Ok(());
        }
        let ssh_host = match &entry {
            HostEntry::Managed(m) => managed_to_ssh_host(m),
            HostEntry::Legacy { host, .. } => host.clone(),
        };
        if ssh_host.proxy_jump.is_some() {
            self.host_notice =
                Some("SFTP via ProxyJump isn't supported yet — pick a direct host.".into());
            return Ok(());
        }

        let (secret, _diag) = resolve_pending_secret(&entry, self.password_store.as_ref());
        let agent = crate::ssh::agent::detect_agent();
        let (tx, rx) = crate::sftp::spawn_sftp_worker(ssh_host, secret, agent);
        let cwd = PathBuf::from(".");
        // The worker queues the listing until its handshake completes.
        let _ = tx.send(SftpCommand::ListDir(Side::Remote, cwd.clone()));
        self.sftp_tx2 = Some(tx);
        self.sftp_rx2 = Some(rx);
        if let Some(s) = self.sftp.as_mut() {
            s.left_host = Some(entry.name().to_string());
            s.left_connecting = true;
            s.local.cwd = cwd;
            s.local.set_entries(Vec::new());
        }
        Ok(())
    }

    /// Send the left pane back to the local filesystem, dropping its server
    /// connection (the worker self-terminates when its sender goes).
    pub(crate) fn sftp_left_pane_to_local(&mut self) {
        // Any relay in flight has one end on the server we're dropping.
        if self.sftp_relay.is_some() {
            self.sftp_relay_abort("second host disconnected — transfer stopped");
        }
        self.sftp_tx2 = None;
        self.sftp_rx2 = None;
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let entries = read_local_dir(&cwd);
        if let Some(s) = self.sftp.as_mut() {
            s.left_host = None;
            s.left_connecting = false;
            s.local.cwd = cwd;
            s.local.set_entries(entries);
        }
    }

    /// Open an SSH session to the host the SFTP browser is connected to (the
    /// reverse of `open_sftp_for_active_session` — completes the round trip).
    /// The SFTP session stays live in the background.
    fn open_ssh_for_sftp_host(&mut self) -> Result<()> {
        let Some(name) = self.sftp_host.clone() else {
            return Ok(());
        };
        // Re-attach to an existing background session for this host (e.g. the one
        // we came from via SessionOpenSftp) instead of spawning a duplicate.
        if let Some(idx) = self.sessions.iter().position(|s| s.display_name == name) {
            self.active_session = Some(idx);
            self.focus_active_session();
            return Ok(());
        }
        // No live session for this host → open a fresh SSH session.
        let Some(entry) = self.hosts.iter().find(|h| h.name() == name).cloned() else {
            if let Some(s) = self.sftp.as_mut() {
                s.notice = Some(format!("no saved host '{name}' to open SSH for"));
            }
            return Ok(());
        };
        self.connect_host_entry(entry)
    }

    /// Navigate the focused pane into `path`. Remote listings go through the
    /// worker (async, applied on the `DirListing` event); local listings are
    /// read synchronously from the filesystem here.
    fn sftp_navigate(&mut self, side: Side, path: PathBuf) {
        match side {
            Side::Remote => {
                // Don't touch cwd/entries optimistically: the DirListing event
                // applies both atomically. So a second navigation before it
                // arrives still builds paths from a consistent cwd+entries, and a
                // failed listing leaves the current directory visible (not blank).
                if let Some(tx) = self.sftp_tx.as_ref() {
                    let _ = tx.send(SftpCommand::ListDir(Side::Remote, path));
                }
            }
            Side::Local if self.sftp.as_ref().is_some_and(|s| s.left_is_remote()) => {
                // Left pane is a second server: same async path as the right
                // one, just down the other worker's channel.
                if let Some(tx) = self.sftp_tx2.as_ref() {
                    let _ = tx.send(SftpCommand::ListDir(Side::Remote, path));
                }
            }
            Side::Local => {
                let entries = read_local_dir(&path);
                if let Some(s) = self.sftp.as_mut() {
                    s.local.cwd = path;
                    s.local.set_entries(entries);
                }
            }
        }
    }

    pub(crate) fn sftp_run_queue(&mut self) {
        if self
            .sftp
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Running)
        {
            return;
        }
        let queue = match self.sftp.as_ref() {
            Some(s) if !s.queue.is_empty() => s.queue.clone(),
            _ => return,
        };
        self.sftp_run_failed = false;
        // Two servers: neither worker can talk to the other, so each item is
        // relayed through a local temp file, one leg at a time.
        if self.sftp.as_ref().is_some_and(|s| s.left_is_remote()) {
            // Owner-only from the moment it exists: `DirBuilder::mode` is
            // applied by the mkdir itself, so there is no window where the
            // user's files sit in a world-readable directory.
            let mut builder = tempfile::Builder::new();
            builder.prefix("sshub-relay-");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                builder.permissions(std::fs::Permissions::from_mode(0o700));
            }
            let tmp_dir = match builder.tempdir() {
                Ok(dir) => dir,
                Err(e) => {
                    if let Some(s) = self.sftp.as_mut() {
                        s.notice = Some(format!("no scratch space for the relay: {e}"));
                    }
                    return;
                }
            };
            if let Some(s) = self.sftp.as_mut() {
                s.phase = Phase::Running;
                s.progress = None;
                s.running = queue.clone();
            }
            self.sftp_relay = Some(SftpRelay {
                total: queue.len(),
                items: queue.into_iter().collect(),
                tmp_dir,
                leg: RelayLeg::Fetching,
            });
            self.sftp_relay_step();
            return;
        }

        self.sftp_run_failed = false;
        if let Some(tx) = self.sftp_tx.as_ref() {
            if tx.send(SftpCommand::RunQueue(queue.clone())).is_ok() {
                if let Some(s) = self.sftp.as_mut() {
                    s.phase = Phase::Running;
                    s.progress = None;
                    // Remember exactly what went out: anything staged while
                    // this runs stays queued for the next pass.
                    s.running = queue;
                }
            }
        }
    }

    /// Start (or continue) relaying server-to-server transfers, one leg at a
    /// time: the source worker pulls an item into a temp directory, then the
    /// destination worker pushes it up from there.
    fn sftp_relay_step(&mut self) {
        let Some(relay) = self.sftp_relay.as_ref() else {
            return;
        };
        let Some(item) = relay.items.front().cloned() else {
            // Nothing left: tidy up and let the queue finish normally.
            self.sftp_relay_finish();
            return;
        };
        let leg = relay.leg;
        let tmp = relay.tmp_dir.path().join(&item.name);
        // `Download` means right-to-left, so the source is the right pane.
        let (from, to) = match item.direction {
            Direction::Download => (Side::Remote, Side::Local),
            Direction::Upload => (Side::Local, Side::Remote),
        };
        let (side, cmd) = match leg {
            RelayLeg::Fetching => (
                from,
                QueuedTransfer {
                    direction: Direction::Download,
                    src: item.src.clone(),
                    dst: tmp,
                    name: item.name.clone(),
                    is_dir: item.is_dir,
                },
            ),
            RelayLeg::Pushing => (
                to,
                QueuedTransfer {
                    direction: Direction::Upload,
                    src: tmp,
                    dst: item.dst.clone(),
                    name: item.name.clone(),
                    is_dir: item.is_dir,
                },
            ),
        };
        let sent = self
            .sftp_channel(side)
            .map(|tx| tx.send(SftpCommand::RunQueue(vec![cmd])).is_ok())
            .unwrap_or(false);
        if !sent {
            self.sftp_relay_abort("not connected — transfer stopped");
        }
    }

    /// Fold a worker's `QueueDone` into the relay: finish the leg, move to the
    /// next one, or take the completed item off the queue. Returns true when
    /// the event belonged to a relay (so the normal completion path is skipped).
    fn sftp_relay_queue_done(&mut self) -> bool {
        let Some(relay) = self.sftp_relay.as_mut() else {
            return false;
        };
        match relay.leg {
            RelayLeg::Fetching => {
                relay.leg = RelayLeg::Pushing;
            }
            RelayLeg::Pushing => {
                // The item is across; drop its temp copy and the queue entry.
                if let Some(done) = relay.items.pop_front() {
                    let tmp = relay.tmp_dir.path().join(&done.name);
                    let _ = if done.is_dir {
                        std::fs::remove_dir_all(&tmp)
                    } else {
                        std::fs::remove_file(&tmp)
                    };
                    if let Some(s) = self.sftp.as_mut() {
                        s.queue.retain(|q| *q != done);
                        s.running.retain(|q| *q != done);
                    }
                }
                if let Some(relay) = self.sftp_relay.as_mut() {
                    relay.leg = RelayLeg::Fetching;
                }
            }
        }
        self.sftp_relay_step();
        true
    }

    /// Report a relay leg's progress against the whole item, so the bar counts
    /// one file rather than restarting for each leg: the fetch fills the first
    /// half, the push the second.
    fn sftp_relay_progress(&mut self, transferred: u64, size: u64) {
        let Some(relay) = self.sftp_relay.as_ref() else {
            return;
        };
        let done = relay.total.saturating_sub(relay.items.len());
        let half = if relay.leg == RelayLeg::Fetching {
            0.0
        } else {
            0.5
        };
        let leg = if size > 0 {
            (transferred as f64 / size as f64).clamp(0.0, 1.0) * 0.5
        } else {
            0.0
        };
        let fraction = half + leg;
        if let Some(s) = self.sftp.as_mut() {
            s.progress = Some(crate::sftp::model::Progress {
                index: done,
                total: relay.total,
                // Scaled to a notional 1000 units so the bar can show a
                // fraction of a single relayed item.
                transferred: (fraction * 1000.0) as u64,
                size: 1000,
            });
        }
    }

    /// Consume one owed trailing `QueueDone`, if any. Returns whether this
    /// event was one of them and should be ignored.
    fn sftp_take_swallowed_done(&mut self) -> bool {
        if self.sftp_swallow_done == 0 {
            return false;
        }
        self.sftp_swallow_done -= 1;
        true
    }

    /// Stop a relay part-way with a notice, leaving whatever hasn't moved in
    /// the queue so it can be retried.
    fn sftp_relay_abort(&mut self, msg: &str) {
        // Dropping the relay drops its scratch directory with it.
        self.sftp_relay = None;
        // The leg that failed still owes us a `QueueDone`; swallow it, and tell
        // both workers to stop in case one is still grinding through a tree.
        self.sftp_swallow_done += 1;
        for tx in [self.sftp_tx.as_ref(), self.sftp_tx2.as_ref()]
            .into_iter()
            .flatten()
        {
            let _ = tx.send(SftpCommand::Cancel);
        }
        if let Some(s) = self.sftp.as_mut() {
            s.phase = Phase::Browsing;
            s.progress = None;
            s.running.clear();
            s.notice = Some(msg.to_string());
        }
    }

    /// Every item is across: clear the temp directory and settle the browser.
    fn sftp_relay_finish(&mut self) {
        // Dropping the relay drops its scratch directory with it.
        self.sftp_relay = None;
        if let Some(s) = self.sftp.as_mut() {
            s.phase = Phase::Browsing;
            s.progress = None;
            s.running.clear();
        }
        self.sftp_refresh_panes();
    }

    /// Re-list both panes: remote via the worker (async `DirListing`), local
    /// synchronously. Used by the `r` refresh key and after a queue completes.
    fn sftp_refresh_panes(&mut self) {
        let (remote_cwd, local_cwd) = match self.sftp.as_ref() {
            Some(s) => (s.remote.cwd.clone(), s.local.cwd.clone()),
            None => return,
        };
        if let Some(tx) = self.sftp_tx.as_ref() {
            let _ = tx.send(SftpCommand::ListDir(Side::Remote, remote_cwd));
        }
        if self.sftp.as_ref().is_some_and(|s| s.left_is_remote()) {
            if let Some(tx) = self.sftp_tx2.as_ref() {
                let _ = tx.send(SftpCommand::ListDir(Side::Remote, local_cwd));
            }
            return;
        }
        let entries = read_local_dir(&local_cwd);
        if let Some(s) = self.sftp.as_mut() {
            s.local.set_entries(entries);
        }
    }

    /// Tear down the live session. Dropping the command `Sender` makes the
    /// worker thread self-terminate.
    fn sftp_disconnect(&mut self) {
        // Mirror the way in (#35): a handshake that never completed carries the
        // "connecting…" layer off to the right, a live browser parts its panes
        // toward both edges. Both reveal the picker underneath.
        let kind = self.sftp.as_ref().map(|s| {
            if s.connecting {
                SftpAnim::ConnectOut
            } else {
                SftpAnim::PanesOut
            }
        });
        if let Some(kind) = kind {
            self.stamp_sftp_anim(kind);
        }
        // Dropping the relay drops its scratch directory with it.
        self.sftp_relay = None;
        self.sftp = None;
        self.sftp_tx = None;
        self.sftp_rx = None;
        self.sftp_host = None;
        self.sftp_tx2 = None;
        self.sftp_rx2 = None;
    }

    /// Apply one [`SftpEvent`] from the *left* pane's worker (the second
    /// server). The worker speaks in its own terms -- everything it reports is
    /// `Side::Remote` -- so its listings are folded into the left pane here.
    pub fn apply_sftp_event_left(&mut self, ev: crate::sftp::SftpEvent) {
        use crate::sftp::SftpEvent;

        match ev {
            SftpEvent::Connected => {
                if let Some(s) = self.sftp.as_mut() {
                    s.left_connecting = false;
                    s.notice = None;
                }
            }
            SftpEvent::ConnectFailed(msg) => {
                let host = self
                    .sftp
                    .as_ref()
                    .and_then(|s| s.left_host.clone())
                    .unwrap_or_default();
                // Fall back to the local filesystem rather than leaving the
                // pane stuck on a server that never answered.
                self.sftp_left_pane_to_local();
                self.notice_popup = Some(format!("Could not connect to {host}:\n{msg}"));
                self.mode = AppMode::Notice;
            }
            SftpEvent::DirListing(_, path, entries) => {
                if let Some(s) = self.sftp.as_mut() {
                    s.local.cwd = path;
                    s.local.set_entries(entries);
                }
            }
            SftpEvent::OpDone => self.sftp_refresh_panes(),
            SftpEvent::Error(msg) if self.sftp_relay.is_some() => {
                self.sftp_run_failed = true;
                self.sftp_relay_abort(&msg);
                self.sftp_refresh_panes();
            }
            SftpEvent::Error(msg) => {
                self.sftp_run_failed = true;
                if let Some(s) = self.sftp.as_mut() {
                    s.notice = Some(msg);
                }
                self.sftp_refresh_panes();
            }
            // This worker only ever runs one leg of a relay at a time.
            SftpEvent::QueueDone => {
                if !self.sftp_take_swallowed_done() {
                    self.sftp_relay_queue_done();
                }
            }
            SftpEvent::Progress {
                transferred, size, ..
            } => self.sftp_relay_progress(transferred, size),
            SftpEvent::TransferDone(_) => {}
        }
    }

    /// Apply one [`SftpEvent`] drained from the worker to the live `sftp` state.
    /// A no-op when there's no live session (events for a torn-down session).
    pub fn apply_sftp_event(&mut self, ev: crate::sftp::SftpEvent) {
        use crate::sftp::model::Progress;
        use crate::sftp::SftpEvent;

        match ev {
            SftpEvent::Connected => {
                let was_connecting = self.sftp.as_ref().is_some_and(|s| s.connecting);
                if let Some(s) = self.sftp.as_mut() {
                    s.notice = None;
                    s.connecting = false;
                }
                // Handshake done: bring the two panes in from their edges (#35).
                if was_connecting {
                    self.stamp_sftp_anim(SftpAnim::PanesIn);
                }
            }
            SftpEvent::ConnectFailed(msg) => {
                let host = self.sftp_host.clone().unwrap_or_default();
                self.sftp_disconnect();
                // Surface the failure as a modal popup instead of silently
                // reverting to the picker — an unreachable host is loud, not blank.
                self.notice_popup = Some(if host.is_empty() {
                    format!("SFTP connection failed:\n{msg}")
                } else {
                    format!("Could not connect to {host}:\n{msg}")
                });
                self.mode = AppMode::Notice;
            }
            SftpEvent::DirListing(side, path, entries) => {
                if let Some(s) = self.sftp.as_mut() {
                    match side {
                        Side::Remote => {
                            s.remote.cwd = path;
                            s.remote.set_entries(entries);
                        }
                        Side::Local => {
                            s.local.cwd = path;
                            s.local.set_entries(entries);
                        }
                    }
                }
            }
            SftpEvent::Progress {
                transferred, size, ..
            } if self.sftp_relay.is_some() => self.sftp_relay_progress(transferred, size),
            SftpEvent::Progress {
                index,
                total,
                transferred,
                size,
            } => {
                if let Some(s) = self.sftp.as_mut() {
                    s.progress = Some(Progress {
                        index,
                        total,
                        transferred,
                        size,
                    });
                }
            }
            SftpEvent::TransferDone(_) => {}
            SftpEvent::QueueDone if self.sftp_take_swallowed_done() => {}
            SftpEvent::QueueDone if self.sftp_relay.is_some() => {
                self.sftp_relay_queue_done();
            }
            SftpEvent::QueueDone => {
                let mut more = false;
                if let Some(s) = self.sftp.as_mut() {
                    s.phase = Phase::Browsing;
                    s.progress = None;
                    more = s.finish_run();
                }
                // Refresh both panes so completed transfers show up.
                self.sftp_refresh_panes();
                // Anything staged mid-run rolls straight into the next pass: a
                // queue you can add to while it works has to keep working. A
                // run that reported an error stops there instead -- restarting
                // it would retry the failing transfer forever.
                if more && !self.sftp_run_failed {
                    self.sftp_run_queue();
                }
            }
            SftpEvent::OpDone => {
                // A remote remove/mkdir/rename landed — re-list so it shows.
                self.sftp_refresh_panes();
            }
            SftpEvent::Error(msg) if self.sftp_relay.is_some() => {
                self.sftp_run_failed = true;
                self.sftp_relay_abort(&msg);
                self.sftp_refresh_panes();
            }
            SftpEvent::Error(msg) => {
                self.sftp_run_failed = true;
                if let Some(s) = self.sftp.as_mut() {
                    s.notice = Some(msg);
                }
                // Re-list so optimistic UI changes roll back — e.g. a failed
                // remote delete restores the row that was dropped up front.
                self.sftp_refresh_panes();
            }
        }
    }
}

/// Read a local directory into `FileEntry` rows, directories first then
/// case-insensitive by name. Unreadable dirs / entries degrade gracefully to an
/// empty listing rather than erroring the UI.
fn read_local_dir(path: &Path) -> Vec<FileEntry> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(path) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // file_type() does not follow the link (is_symlink detection);
            // fs::metadata does, so a symlink-to-dir keeps is_dir=true and
            // stays enterable, and a symlink-to-file shows its target size.
            // Transfer planning never descends into symlinks regardless.
            let ftype = entry.file_type().ok();
            let is_symlink = ftype.map(|t| t.is_symlink()).unwrap_or(false);
            let meta = std::fs::metadata(entry.path()).ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let perm = meta.as_ref().map(|m| {
                use std::os::unix::fs::PermissionsExt;
                m.permissions().mode() & 0o7777
            });
            out.push(FileEntry {
                name,
                is_dir,
                size,
                is_symlink,
                perm,
            });
        }
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}
