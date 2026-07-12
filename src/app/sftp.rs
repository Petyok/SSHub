use super::*;

use std::path::{Path, PathBuf};

use crate::sftp::model::{FileEntry, Phase, SftpState, Side};
use crate::sftp::SftpCommand;

impl App {
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
                self.pre_help_mode = Some(self.mode);
                self.mode = AppMode::Help;
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
        // Esc / Cancel disconnects the live session back to the picker.
        if self.is_action(KeyAction::Cancel, &key) {
            self.sftp_disconnect();
            return Ok(());
        }
        // Enter descends into the selected directory of the focused pane.
        if !running && self.is_action(KeyAction::Connect, &key) {
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
            self.pre_help_mode = Some(self.mode);
            self.mode = AppMode::Help;
            return Ok(());
        }

        match key.code {
            KeyCode::Tab => {
                if let Some(s) = self.sftp.as_mut() {
                    s.toggle_focus();
                }
            }
            KeyCode::Backspace => {
                if !running {
                    if let Some((side, path)) = self.sftp.as_ref().and_then(|s| s.parent_dir()) {
                        self.sftp_navigate(side, path);
                    }
                }
            }
            // Panes are left=local, right=remote, so the arrow points at the
            // destination: ← downloads (remote → local), → uploads (local → remote).
            KeyCode::Left => {
                if !running {
                    if let Some(s) = self.sftp.as_mut() {
                        let _ = s.stage_download();
                    }
                }
            }
            KeyCode::Right => {
                if !running {
                    if let Some(s) = self.sftp.as_mut() {
                        let _ = s.stage_upload();
                    }
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
        let Some(entry) = pane.selected_entry() else {
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
                let Some(entry) = pane.selected_entry() else {
                    return;
                };
                (entry.name.clone(), Some(base.join(&entry.name)))
            }
            SftpPromptKind::Chmod => {
                let Some(entry) = pane.selected_entry() else {
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

        match side {
            Side::Remote => {
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
                    .sftp_tx
                    .as_ref()
                    .map(|tx| tx.send(cmd).is_ok())
                    .unwrap_or(false);
                if sent {
                    self.sftp_prompt = None;
                    self.mode = AppMode::Normal;
                } else if let Some(p) = self.sftp_prompt.as_mut() {
                    p.error = Some("not connected".into());
                }
            }
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

        match side {
            Side::Remote => {
                let sent = self
                    .sftp_tx
                    .as_ref()
                    .map(|tx| tx.send(crate::sftp::SftpCommand::Chmod(path, mode)).is_ok())
                    .unwrap_or(false);
                if sent {
                    self.sftp_prompt = None;
                    self.mode = AppMode::Normal;
                } else if let Some(p) = self.sftp_prompt.as_mut() {
                    p.error = Some("not connected".into());
                }
            }
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
        state.local.set_entries(read_local_dir(&local_cwd));

        // Kick off the first remote listing; the worker queues it until the
        // connection completes.
        let _ = tx.send(SftpCommand::ListDir(Side::Remote, remote_cwd));

        self.sftp = Some(state);
        self.sftp_tx = Some(tx);
        self.sftp_rx = Some(rx);
        self.sftp_host = Some(entry.name().to_string());
        Ok(())
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
            Side::Local => {
                let entries = read_local_dir(&path);
                if let Some(s) = self.sftp.as_mut() {
                    s.local.cwd = path;
                    s.local.set_entries(entries);
                }
            }
        }
    }

    fn sftp_run_queue(&mut self) {
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
        if let Some(tx) = self.sftp_tx.as_ref() {
            if tx.send(SftpCommand::RunQueue(queue)).is_ok() {
                if let Some(s) = self.sftp.as_mut() {
                    s.phase = Phase::Running;
                    s.progress = None;
                }
            }
        }
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
        let entries = read_local_dir(&local_cwd);
        if let Some(s) = self.sftp.as_mut() {
            s.local.set_entries(entries);
        }
    }

    /// Tear down the live session. Dropping the command `Sender` makes the
    /// worker thread self-terminate.
    fn sftp_disconnect(&mut self) {
        self.sftp = None;
        self.sftp_tx = None;
        self.sftp_rx = None;
        self.sftp_host = None;
    }

    /// Apply one [`SftpEvent`] drained from the worker to the live `sftp` state.
    /// A no-op when there's no live session (events for a torn-down session).
    pub fn apply_sftp_event(&mut self, ev: crate::sftp::SftpEvent) {
        use crate::sftp::model::Progress;
        use crate::sftp::SftpEvent;

        match ev {
            SftpEvent::Connected => {
                if let Some(s) = self.sftp.as_mut() {
                    s.notice = None;
                }
            }
            SftpEvent::ConnectFailed(msg) => {
                self.sftp_disconnect();
                self.host_notice = Some(format!("SFTP connection failed: {msg}"));
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
            SftpEvent::QueueDone => {
                if let Some(s) = self.sftp.as_mut() {
                    s.phase = Phase::Browsing;
                    s.progress = None;
                    s.queue.clear();
                }
                // Refresh both panes so completed transfers show up.
                self.sftp_refresh_panes();
            }
            SftpEvent::OpDone => {
                // A remote remove/mkdir/rename landed — re-list so it shows.
                self.sftp_refresh_panes();
            }
            SftpEvent::Error(msg) => {
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
