use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use super::*;
use crate::store::Identity;

impl App {
    pub(crate) fn trigger_push_key_from_hosts(&mut self) -> Result<()> {
        let Some(_entry) = self.selected_entry().cloned() else {
            // A group header (or an empty list) has nothing to push to. Silence
            // here reads as "the key is broken", so say what's wrong.
            self.host_notice =
                Some("Select a host row first — a group header has no key target.".into());
            return Ok(());
        };

        // `self.identities` is lazily filled (Keys tab, host form, group form),
        // so before the first visit to those it is empty and push-key bailed out
        // with "no key identities" on the hosts tab.
        self.reload_identities()?;
        let key_identities = self.push_key_identities();
        if key_identities.is_empty() {
            self.host_notice =
                Some("No key identities available — generate or add one first.".into());
            return Ok(());
        }

        self.push_key_identity_picker = Some(PushKeyIdentityPicker {
            query: String::new(),
            selected: 0,
        });
        self.mode = AppMode::PushKeyIdentityPicker;
        Ok(())
    }

    pub(crate) fn trigger_push_key_from_keys(&mut self) -> Result<()> {
        let Some(identity) = self.selected_identity().cloned() else {
            self.identity_notice = Some("No identity selected.".into());
            return Ok(());
        };

        if identity.private_key.is_none() {
            self.identity_notice = Some("Cannot push a password-only identity.".into());
            return Ok(());
        }

        self.push_key_host_picker = Some(PushKeyHostPicker {
            query: String::new(),
            selected: 0,
        });
        self.mode = AppMode::PushKeyHostPicker;
        Ok(())
    }

    pub fn push_key_identities(&self) -> Vec<&Identity> {
        self.identities
            .iter()
            .filter(|i| i.private_key.is_some())
            .collect()
    }

    /// Key identities matching the picker's query, best match first.
    pub fn push_key_identity_matches(&self) -> Vec<&Identity> {
        let identities = self.push_key_identities();
        let query = self
            .push_key_identity_picker
            .as_ref()
            .map(|p| p.query.as_str())
            .unwrap_or_default();
        let items: Vec<Vec<String>> = identities
            .iter()
            .map(|i| {
                vec![
                    i.name.clone(),
                    i.username.clone().unwrap_or_default(),
                    i.private_key
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ]
            })
            .collect();
        crate::search::fuzzy_indices(query, &items)
            .into_iter()
            .map(|i| identities[i])
            .collect()
    }

    pub fn push_key_host_matches(&self) -> Vec<(usize, String)> {
        let query = self
            .push_key_host_picker
            .as_ref()
            .map(|p| p.query.clone())
            .unwrap_or_default();
        if query.is_empty() {
            return self
                .hosts
                .iter()
                .enumerate()
                .map(|(idx, h)| (idx, format!("{}  {}", h.display_name(), h.name())))
                .collect();
        }
        let mut search = crate::search::HostSearch::new();
        let matched_indices = search.update_query(&self.hosts, &query);
        matched_indices
            .into_iter()
            .map(|idx| {
                (
                    idx,
                    format!(
                        "{}  {}",
                        self.hosts[idx].display_name(),
                        self.hosts[idx].name()
                    ),
                )
            })
            .collect()
    }

    pub(crate) fn handle_key_push_key_identity_picker(&mut self, key: KeyEvent) -> Result<()> {
        let len = self.push_key_identity_matches().len();
        match key.code {
            KeyCode::Esc => {
                self.push_key_identity_picker = None;
                self.mode = AppMode::Normal;
            }
            KeyCode::Down => {
                if len > 0 {
                    if let Some(p) = self.push_key_identity_picker.as_mut() {
                        p.selected = (p.selected + 1) % len;
                    }
                }
            }
            KeyCode::Up => {
                if len > 0 {
                    if let Some(p) = self.push_key_identity_picker.as_mut() {
                        p.selected = (p.selected + len - 1) % len;
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(p) = self.push_key_identity_picker.as_mut() {
                    p.query.pop();
                    p.selected = 0;
                }
            }
            KeyCode::Char(c) if !c.is_control() => {
                if let Some(p) = self.push_key_identity_picker.as_mut() {
                    p.query.push(c);
                    p.selected = 0;
                }
            }
            KeyCode::Enter => {
                let identities = self.push_key_identity_matches();
                let identity = self
                    .push_key_identity_picker
                    .as_ref()
                    .and_then(|p| identities.get(p.selected))
                    .cloned()
                    .cloned();
                // No match: stay open; the list already says "(no matching identities)".
                let Some(identity) = identity else {
                    return Ok(());
                };
                self.push_key_identity_picker = None;
                self.mode = AppMode::Normal;

                if let Some(entry) = self.selected_entry().cloned() {
                    self.push_public_key_to_host(&entry, &identity)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_key_push_key_host_picker(&mut self, key: KeyEvent) -> Result<()> {
        let len = self.push_key_host_matches().len();
        match key.code {
            KeyCode::Esc => {
                self.push_key_host_picker = None;
                self.mode = AppMode::Normal;
            }
            KeyCode::Down => {
                if len > 0 {
                    if let Some(p) = self.push_key_host_picker.as_mut() {
                        p.selected = (p.selected + 1) % len;
                    }
                }
            }
            KeyCode::Up => {
                if len > 0 {
                    if let Some(p) = self.push_key_host_picker.as_mut() {
                        p.selected = (p.selected + len - 1) % len;
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(p) = self.push_key_host_picker.as_mut() {
                    p.query.pop();
                    p.selected = 0;
                }
            }
            KeyCode::Char(c) if !c.is_control() => {
                if let Some(p) = self.push_key_host_picker.as_mut() {
                    p.query.push(c);
                    p.selected = 0;
                }
            }
            KeyCode::Enter => {
                let matches = self.push_key_host_matches();
                let host_idx = self
                    .push_key_host_picker
                    .as_ref()
                    .and_then(|p| matches.get(p.selected))
                    .map(|(idx, _)| *idx);
                let identity = self.selected_identity().cloned();

                self.push_key_host_picker = None;
                self.mode = AppMode::Normal;

                if let (Some(idx), Some(identity)) = (host_idx, identity) {
                    if let Some(entry) = self.hosts.get(idx).cloned() {
                        self.push_public_key_to_host(&entry, &identity)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn push_public_key_to_host(
        &mut self,
        entry: &HostEntry,
        identity: &Identity,
    ) -> Result<()> {
        let Some(ref key_path) = identity.private_key else {
            self.host_notice = Some("Identity does not have a private key path set.".into());
            return Ok(());
        };

        let passphrase = if identity.has_password {
            self.password_store
                .get(&crate::credentials::identity_key(identity.id))
                .ok()
                .flatten()
        } else {
            None
        };

        let public_key = match crate::ssh::read_public_key(key_path, passphrase.as_deref()) {
            Ok(k) => k,
            Err(e) => {
                let err_msg = format!("Failed to read public key: {e:#}");
                self.host_notice = Some(err_msg.clone());
                let username = entry.managed().and_then(|m| m.username.as_deref());
                let _ = self.store.log_auth_event(
                    entry.name(),
                    username,
                    "direct",
                    "fail",
                    &err_msg,
                    None,
                );
                return Ok(());
            }
        };

        let escaped_key = public_key.replace("'", "'\\''");
        // "Already present" goes to stderr, which reaches `debug_log` and never
        // the PTY: PTY bytes before auth read as a prompt that needs the user.
        let remote_cmd = format!(
            "umask 077 && mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys && \
             key='{}' && \
             (grep -qxF \"$key\" ~/.ssh/authorized_keys && echo '{KEY_PRESENT_MARKER}' >&2 || \
              echo \"$key\" >> ~/.ssh/authorized_keys)",
            escaped_key
        );

        let resolved = match entry.managed() {
            Some(m) => match self.store.resolve_connection(m) {
                Ok(r) => Some(r),
                Err(e) => {
                    self.host_notice = Some(format!("{}: {e:#}", entry.name()));
                    return Ok(());
                }
            },
            None => None,
        };
        let effective_identity = resolved.as_ref().and_then(|r| r.identity.as_ref());
        let (pending_secret, credential_diag): (Option<crate::session::PendingSecret>, String) =
            resolve_pending_secret(entry, effective_identity, self.password_store.as_ref());

        let mut ssh_argv = self.ssh_argv_for_key_push(entry, &remote_cmd);
        if ssh_argv.first().map(String::as_str) == Some("ssh") {
            ssh_argv.splice(1..1, crate::session::SSH_DEBUG_ARGS.map(String::from));
            if pending_secret.is_some() {
                ssh_argv.insert(1, "-o".into());
                ssh_argv.insert(2, "StrictHostKeyChecking=accept-new".into());
            }
        }

        // Push diagnostics
        self.ssh_log.retain(|e| e.host_name != entry.name());
        {
            let level = if pending_secret.is_some() {
                crate::ssh::probe::LogLevel::Success
            } else {
                crate::ssh::probe::LogLevel::Info
            };
            self.push_ssh_log(crate::ssh::probe::SshLogEntry {
                host_name: entry.name().to_string(),
                line: credential_diag,
                level,
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
            });
        }

        // Pre-validate PATH
        if let Some(first_cmd) = ssh_argv.first() {
            if std::process::Command::new("which")
                .arg(first_cmd)
                .output()
                .map(|o| !o.status.success())
                .unwrap_or(true)
            {
                let msg = format!(
                    "Command not found: '{}'. Check your PATH or install it.",
                    first_cmd
                );
                self.push_ssh_log(crate::ssh::probe::SshLogEntry {
                    host_name: entry.name().to_string(),
                    line: msg.clone(),
                    level: crate::ssh::probe::LogLevel::Error,
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64,
                });
                self.host_notice = Some(msg);
                return Ok(());
            }
        }

        let now_ts_val = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.push_ssh_log(crate::ssh::probe::SshLogEntry {
            host_name: entry.name().to_string(),
            line: format!("$ {}", ssh_argv.join(" ")),
            level: crate::ssh::probe::LogLevel::Success,
            timestamp: now_ts_val,
        });

        // Log auth event start
        let host_name = entry.name().to_string();
        let username = entry.managed().and_then(|m| {
            m.username
                .as_deref()
                .or_else(|| m.identity.as_ref().and_then(|i| i.username.as_deref()))
        });
        let via = entry
            .managed()
            .and_then(|m| m.proxy_jump.as_deref())
            .unwrap_or("direct");

        let display_name = format!("Push Key to {}", entry.name());
        let meta = match (entry.managed(), resolved.as_ref()) {
            (Some(m), Some(r)) => session_meta_for_resolved(m, r),
            _ => session_meta_for_entry(entry),
        };

        // The jump hosts' own stored passwords: a hop that asks is answered with
        // its own, never with this host's.
        let hops = hop_secrets(
            &self.store,
            &self.hosts,
            self.password_store.as_ref(),
            resolved.as_ref().and_then(|r| r.proxy_jump.as_deref()),
        );
        let config = crate::session::SessionConfig {
            hops,
            argv: ssh_argv,
            display_name: display_name.clone(),
            meta,
            pending_secret: pending_secret.clone(),
            key_push_identity: Some(identity.name.clone()),
            host_name: entry.name().to_string(),
        };

        let grid = self.new_session_grid();
        match crate::session::Session::spawn(config, grid, None) {
            Ok(session) => {
                let _ = self.store.log_auth_event(
                    &host_name,
                    username,
                    via,
                    "launched",
                    &format!("started pushing public key '{}'", identity.name),
                    None,
                );
                self.key_push = Some(KeyPush {
                    identity: identity.name.clone(),
                    host: host_name.clone(),
                    started: std::time::Instant::now(),
                    session: Some(session),
                    state: KeyPushState::Running,
                    user: username.map(str::to_string),
                    via: via.to_string(),
                });
                self.mode = AppMode::KeyPush;
            }
            Err(e) => {
                let err_msg = format!("Session spawn failed: {e:#}");
                let _ = self
                    .store
                    .log_auth_event(&host_name, username, via, "fail", &err_msg, None);
                self.push_ssh_log(crate::ssh::probe::SshLogEntry {
                    host_name: host_name.clone(),
                    line: err_msg.clone(),
                    level: crate::ssh::probe::LogLevel::Error,
                    timestamp: now_ts_val,
                });
                self.host_notice = Some(err_msg);
            }
        }

        // Force-refresh auth cache
        self.auth_cache_updated = std::time::Instant::now() - std::time::Duration::from_secs(60);
        self.refresh_auth_cache();
        Ok(())
    }

    pub(crate) fn ssh_argv_for_key_push(&self, entry: &HostEntry, remote_cmd: &str) -> Vec<String> {
        let mut base_argv = match entry {
            HostEntry::Managed(m) => {
                // Group defaults apply to key pushes too; a resolution
                // failure falls back to the stored row, never aborts.
                let ssh_host = match self.store.resolve_connection(m) {
                    Ok(r) => resolved_to_ssh_host(m, &r),
                    Err(_) => managed_to_ssh_host(m),
                };
                let mut ssh_host = ssh_host;
                ssh_host.remote_command = None;
                if m.source == HostSource::SshConfig {
                    crate::ssh::build_ssh_alias_argv(&ssh_host)
                } else {
                    crate::ssh::build_ssh_argv(&ssh_host)
                }
            }
            HostEntry::Legacy { host, .. } => {
                let mut ssh_host = host.clone();
                ssh_host.remote_command = None;
                crate::ssh::build_ssh_alias_argv(&ssh_host)
            }
        };
        base_argv.push("--".into());
        base_argv.push(remote_cmd.to_string());
        base_argv
    }

    /// Drain the background key-push ssh (once per frame) and settle its
    /// outcome. ssh stopping at a prompt it was not given an answer for flips
    /// the popup to [`KeyPushState::NeedsInput`] instead of hanging silently.
    pub fn poll_key_push(&mut self) {
        let Some(push) = self.key_push.as_mut() else {
            return;
        };
        let Some(s) = push.session.as_mut() else {
            return;
        };
        s.drain();
        let connected = s.is_connected();
        let diags: Vec<String> = s
            .take_diagnostics()
            .into_iter()
            .filter(|l| crate::session::keep_diagnostic(l, connected))
            .collect();
        let outcome = match &s.phase {
            // stderr comes through its own reader thread and can land just
            // after the PTY's EOF; the outcome is read from it, so wait a beat.
            // ponytail: fixed grace, drain until the stderr reader ends if it misses.
            crate::session::SessionPhase::Exited { at, .. }
                if at.elapsed() < std::time::Duration::from_millis(250) =>
            {
                None
            }
            crate::session::SessionPhase::Exited { status, .. } if status == "success" => {
                Some(if s.debug_log().contains(KEY_PRESENT_MARKER) {
                    KeyPushState::AlreadyPresent
                } else {
                    KeyPushState::Installed
                })
            }
            crate::session::SessionPhase::Exited { .. } => {
                Some(KeyPushState::Failed(key_push_failure(s)))
            }
            // The session reveals itself before auth only for PTY output, i.e.
            // a prompt or a host-key question. Right after an auto-typed
            // password it is just ssh finishing auth, so give that a moment.
            crate::session::SessionPhase::Running { started_at }
                if !connected
                    && push.state == KeyPushState::Running
                    && (!s.secret_was_sent()
                        || started_at.elapsed() >= std::time::Duration::from_secs(3)) =>
            {
                push.state = KeyPushState::NeedsInput;
                None
            }
            _ => None,
        };
        let host = push.host.clone();
        for line in diags {
            self.push_ssh_log(crate::ssh::probe::SshLogEntry {
                host_name: host.clone(),
                line,
                level: crate::ssh::probe::LogLevel::Info,
                timestamp: crate::cli::output::now_ts(),
            });
        }
        if let Some(state) = outcome {
            self.finish_key_push(state);
        }
    }

    /// Stop the ssh (if still running), record the outcome in the audit and
    /// ssh logs, and leave the popup showing it.
    fn finish_key_push(&mut self, state: KeyPushState) {
        let Some(push) = self.key_push.as_mut() else {
            return;
        };
        push.session = None;
        let id = &push.identity;
        let (status, note, level) = match &state {
            KeyPushState::Installed => (
                "ok",
                format!("pushed public key '{id}' to host"),
                crate::ssh::probe::LogLevel::Success,
            ),
            KeyPushState::AlreadyPresent => (
                "ok",
                format!("public key '{id}' already present on host"),
                crate::ssh::probe::LogLevel::Success,
            ),
            KeyPushState::Failed(reason) => (
                "fail",
                format!("failed to push public key '{id}' to host: {reason}"),
                crate::ssh::probe::LogLevel::Error,
            ),
            KeyPushState::Cancelled => (
                "fail",
                format!("cancelled pushing public key '{id}'"),
                crate::ssh::probe::LogLevel::Error,
            ),
            KeyPushState::Running | KeyPushState::NeedsInput => return,
        };
        push.state = state;
        let host = push.host.clone();
        let user = push.user.clone();
        let via = push.via.clone();
        let _ = self
            .store
            .log_auth_event(&host, user.as_deref(), &via, status, &note, None);
        self.push_ssh_log(crate::ssh::probe::SshLogEntry {
            host_name: host,
            line: note,
            level,
            timestamp: crate::cli::output::now_ts(),
        });
    }

    /// `AppMode::KeyPush`: Esc cancels a running push; Enter hands a push
    /// stuck on a prompt to a terminal tab; Enter/Esc close the outcome.
    pub(crate) fn handle_key_key_push(&mut self, key: KeyEvent) -> Result<()> {
        let Some(state) = self.key_push.as_ref().map(|p| p.state.clone()) else {
            self.mode = AppMode::Normal;
            return Ok(());
        };
        let live = matches!(state, KeyPushState::Running | KeyPushState::NeedsInput);
        match key.code {
            KeyCode::Esc if live => self.finish_key_push(KeyPushState::Cancelled),
            KeyCode::Enter if state == KeyPushState::NeedsInput => {
                let session = self.key_push.take().and_then(|mut p| p.session.take());
                match session {
                    Some(session) => {
                        // The tab keeps `key_push_identity`, so the event loop
                        // writes the audit outcome when it exits.
                        self.sessions.push(session);
                        self.active_session = Some(self.sessions.len() - 1);
                        self.mode = AppMode::Session;
                    }
                    None => self.mode = AppMode::Normal,
                }
            }
            KeyCode::Enter | KeyCode::Esc if !live => {
                self.key_push = None;
                self.mode = AppMode::Normal;
            }
            _ => {}
        }
        Ok(())
    }
}

/// Written by the remote one-liner (to stderr) when the key was already there.
const KEY_PRESENT_MARKER: &str = "sshub: key already present";

/// Why a key push failed, in words. Once authenticated, the remote command
/// itself failed and its stderr says why; before that, ssh's own error does.
fn key_push_failure(s: &crate::session::Session) -> String {
    if !s.is_connected() {
        return s.failure_reason();
    }
    // ssh's verbose lines carry no `debug` prefix; skip them too.
    const SSH_NOISE: [&str; 5] = [
        "debug",
        "Transferred:",
        "Bytes per second",
        "Authenticated to",
        "Warning: Permanently added",
    ];
    s.debug_log()
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty() && !SSH_NOISE.iter().any(|n| l.starts_with(n)))
        .map(|l| format!("remote command failed: {l}"))
        .unwrap_or_else(|| "the remote command failed (no error output)".to_string())
}

impl KeyPush {
    /// The popup's text for the current state.
    pub fn message(&self) -> String {
        let (id, host) = (&self.identity, &self.host);
        match &self.state {
            KeyPushState::Running => format!(
                "Pushing key {id} to {host}… {}s",
                self.started.elapsed().as_secs()
            ),
            KeyPushState::NeedsInput => format!(
                "ssh to {host} is waiting for input sshub cannot give \
                 (a password or a host-key question).\n\
                 Enter opens it in a terminal tab to answer; Esc cancels."
            ),
            KeyPushState::Installed => format!("Key {id} installed on {host}."),
            KeyPushState::AlreadyPresent => format!("Key {id} is already present on {host}."),
            KeyPushState::Failed(reason) => {
                format!("Could not push key {id} to {host}:\n{reason}")
            }
            KeyPushState::Cancelled => format!("Push of key {id} to {host} cancelled."),
        }
    }
}
