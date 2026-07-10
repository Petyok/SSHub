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
            _ if self.is_action(KeyAction::Help, &key) => {
                self.pre_help_mode = Some(self.mode);
                self.mode = AppMode::Help;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_key_sftp_browser(&mut self, key: KeyEvent) -> Result<()> {
        // Esc / Cancel disconnects the live session back to the picker.
        if self.is_action(KeyAction::Cancel, &key) {
            self.sftp_disconnect();
            return Ok(());
        }
        // Enter descends into the selected directory of the focused pane.
        if self.is_action(KeyAction::Connect, &key) {
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

        match key.code {
            KeyCode::Tab => {
                if let Some(s) = self.sftp.as_mut() {
                    s.toggle_focus();
                }
            }
            KeyCode::Backspace => {
                if let Some((side, path)) = self.sftp.as_ref().and_then(|s| s.parent_dir()) {
                    self.sftp_navigate(side, path);
                }
            }
            // Panes are left=local, right=remote, so the arrow points at the
            // destination: ← downloads (remote → local), → uploads (local → remote).
            KeyCode::Left => {
                if let Some(s) = self.sftp.as_mut() {
                    let _ = s.stage_download();
                }
            }
            KeyCode::Right => {
                if let Some(s) = self.sftp.as_mut() {
                    let _ = s.stage_upload();
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
            // Confirm: run the whole queue sequentially.
            KeyCode::Char('c') => self.sftp_run_queue(),
            _ => {}
        }
        Ok(())
    }

    /// Spawn the worker for the selected host and enter the browser. Refuses
    /// ProxyJump hosts (unsupported by the libssh2 transport in v1) with a
    /// notice instead of a doomed connection attempt.
    fn sftp_connect_selected(&mut self) -> Result<()> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(());
        };

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
        Ok(())
    }

    /// Navigate the focused pane into `path`. Remote listings go through the
    /// worker (async, applied on the `DirListing` event); local listings are
    /// read synchronously from the filesystem here.
    fn sftp_navigate(&mut self, side: Side, path: PathBuf) {
        match side {
            Side::Remote => {
                if let Some(s) = self.sftp.as_mut() {
                    s.remote.cwd = path.clone();
                }
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

    /// Tear down the live session. Dropping the command `Sender` makes the
    /// worker thread self-terminate.
    fn sftp_disconnect(&mut self) {
        self.sftp = None;
        self.sftp_tx = None;
        self.sftp_rx = None;
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
            SftpEvent::Error(msg) => {
                if let Some(s) = self.sftp.as_mut() {
                    s.notice = Some(msg);
                }
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
            let (is_dir, size) = match entry.metadata() {
                Ok(m) => (m.is_dir(), m.len()),
                Err(_) => (false, 0),
            };
            out.push(FileEntry { name, is_dir, size });
        }
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}
