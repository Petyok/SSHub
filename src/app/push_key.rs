use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use super::*;
use crate::store::Identity;

impl App {
    pub(crate) fn trigger_push_key_from_hosts(&mut self) -> Result<()> {
        let Some(_entry) = self.selected_entry().cloned() else {
            return Ok(());
        };

        let key_identities = self.push_key_identities();
        if key_identities.is_empty() {
            self.host_notice = Some("No key identities available — generate or add one first.".into());
            return Ok(());
        }

        self.push_key_identity_picker = Some(PushKeyIdentityPicker { selected: 0 });
        self.mode = AppMode::PushKeyIdentityPicker;
        Ok(())
    }

    pub(crate) fn trigger_push_key_from_keys(&mut self) -> Result<()> {
        let Some(identity) = self.selected_identity().cloned() else {
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

    pub fn push_key_host_matches(&self) -> Vec<(usize, String)> {
        let query = self.push_key_host_picker.as_ref()
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

    pub(crate) fn handle_key_push_key_identity_picker(&mut self, key: KeyEvent) -> Result<()> {
        let len = self.push_key_identities().len();
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
            KeyCode::Enter => {
                let identities = self.push_key_identities();
                let identity = self.push_key_identity_picker.as_ref()
                    .and_then(|p| identities.get(p.selected))
                    .cloned()
                    .cloned();
                self.push_key_identity_picker = None;
                self.mode = AppMode::Normal;

                if let Some(identity) = identity {
                    if let Some(entry) = self.selected_entry().cloned() {
                        self.push_public_key_to_host(&entry, &identity)?;
                    }
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
                let host_idx = self.push_key_host_picker.as_ref()
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

    pub(crate) fn push_public_key_to_host(&mut self, entry: &HostEntry, identity: &Identity) -> Result<()> {
        let Some(ref key_path) = identity.private_key else {
            self.host_notice = Some("Identity does not have a private key path set.".into());
            return Ok(());
        };

        let passphrase = if identity.has_password {
            self.password_store.get(&crate::credentials::identity_key(identity.id)).ok().flatten()
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
                );
                return Ok(());
            }
        };

        let escaped_key = public_key.replace("'", "'\\''");
        let remote_cmd = format!(
            "umask 077 && mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys && \
             key='{}' && \
             (grep -qxF \"$key\" ~/.ssh/authorized_keys || echo \"$key\" >> ~/.ssh/authorized_keys)",
            escaped_key
        );

        let (pending_secret, credential_diag): (Option<crate::session::PendingSecret>, String) =
            resolve_pending_secret(entry, self.password_store.as_ref());

        let mut ssh_argv = self.ssh_argv_for_key_push(entry, &remote_cmd);
        if ssh_argv.first().map(String::as_str) == Some("ssh") {
            ssh_argv.insert(1, "-v".into());
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
        let rows = self.terminal_area.height.max(3);
        let cols = self.terminal_area.width.max(20);
        let meta = session_meta_for_entry(entry);
        
        let config = crate::session::SessionConfig {
            argv: ssh_argv,
            display_name: display_name.clone(),
            meta,
            pending_secret: pending_secret.clone(),
            key_push_identity: Some(identity.name.clone()),
            host_name: entry.name().to_string(),
        };

        match crate::session::Session::spawn(config, rows, cols) {
            Ok(session) => {
                self.sessions.push(session);
                self.active_session = Some(self.sessions.len() - 1);
                self.mode = AppMode::Connecting;
                let _ = self.store.log_auth_event(
                    &host_name,
                    username,
                    via,
                    "launched",
                    &format!("started pushing public key '{}'", identity.name),
                );
            }
            Err(e) => {
                let err_msg = format!("Session spawn failed: {e:#}");
                let _ = self
                    .store
                    .log_auth_event(&host_name, username, via, "fail", &err_msg);
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
                let mut ssh_host = managed_to_ssh_host(m);
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
}
