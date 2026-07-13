use super::*;

impl App {
    /// Launch SSH for the currently selected host and record last-connected time.
    pub fn connect_selected(&mut self) -> Result<()> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(());
        };
        self.connect_host_entry(entry)
    }

    /// Launch SSH for a host by index in [`App::hosts`].
    pub fn connect_host_at(&mut self, host_idx: usize) -> Result<()> {
        let Some(entry) = self.hosts.get(host_idx).cloned() else {
            return Ok(());
        };
        self.connect_host_entry(entry)
    }

    pub(crate) fn connect_host_entry(&mut self, entry: HostEntry) -> Result<()> {
        // Start each connection with a clean per-host log so a fresh command
        // line and its handshake aren't mixed with a previous attempt's.
        self.ssh_log.retain(|e| e.host_name != entry.name());

        // Determine the stored secret to feed ssh at the first prompt. A
        // host-level credential is sent at `password:`-style prompts; an
        // identity-level credential is sent at `Enter passphrase for …`.
        // The Session itself watches the PTY screen and types it once.
        let (pending_secret, credential_diag): (Option<crate::session::PendingSecret>, String) =
            resolve_pending_secret(&entry, self.password_store.as_ref());

        // Build ssh argv. The session hands a stored secret to ssh via
        // SSH_ASKPASS. When a secret is present, auto-accept a genuinely new
        // host key: otherwise ssh (with SSH_ASKPASS_REQUIRE=force) would ask
        // the askpass helper to confirm the fingerprint, get the password back
        // instead of "yes", and deadlock. Changed keys are still refused.
        let mut ssh_argv = ssh_argv_for_entry(&entry);
        if ssh_argv.first().map(String::as_str) == Some("ssh") {
            // `-v` streams ssh's real handshake into the session terminal, so
            // the connect screen shows the genuine process instead of a
            // scripted animation.
            ssh_argv.insert(1, "-v".into());
            if pending_secret.is_some() {
                ssh_argv.insert(1, "-o".into());
                ssh_argv.insert(2, "StrictHostKeyChecking=accept-new".into());
            }
        }

        // Surface the credential decision so it's visible in the SSH log
        // panel after the session ends.
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

        // Pre-validate: check that the first command binary exists on PATH
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

        // Log the actual command being run to ssh_log
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

        // Log auth event based on launch result
        {
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
            // Spawn an embedded PTY session in-process. No external terminal.
            let display_name = entry.name().to_string();
            let rows = self.terminal_area.height.max(3);
            let cols = self.terminal_area.width.max(20);
            let meta = session_meta_for_entry(&entry);
            let config = crate::session::SessionConfig {
                argv: ssh_argv.clone(),
                display_name,
                meta,
                pending_secret: pending_secret.clone(),
                key_push_identity: None,
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
                        "session started",
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
                    return Ok(());
                }
            }
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        if let Some(id) = entry.managed_id() {
            self.store.set_host_last_connected(id, timestamp)?;
            if let Some(idx) = self.hosts.iter().position(|e| e.managed_id() == Some(id)) {
                if let HostEntry::Managed(m) = &mut self.hosts[idx] {
                    m.last_connected = Some(timestamp);
                }
            }
        } else {
            self.metadata.set_last_connected(entry.name(), timestamp)?;
            if let Some(idx) = self.hosts.iter().position(|e| e.name() == entry.name()) {
                if let Some((_, meta)) = self.hosts[idx].legacy_mut() {
                    meta.last_connected = Some(timestamp);
                }
            }
        }

        // Force-refresh auth cache immediately after logging the event
        self.auth_cache_updated = std::time::Instant::now() - std::time::Duration::from_secs(60);
        self.refresh_auth_cache();

        // Kick off a background OS auto-detect probe for managed hosts whose
        // os_icon is still empty. Only Managed hosts carry a stable host_id;
        // Legacy/ssh_config hosts are skipped. Guarded by the inflight set so a
        // rapid re-connect doesn't spawn duplicate probes.
        if let Some(m) = entry.managed() {
            let empty = m.os_icon.as_deref().is_none_or(|s| s.is_empty());
            if empty && !self.os_detect_inflight.contains(&m.id) {
                if let Some(tx) = self.os_detect_tx.as_ref() {
                    let (secret, _diag) =
                        resolve_pending_secret(&entry, self.password_store.as_ref());
                    let argv = ssh_argv_for_entry(&entry);
                    if tx
                        .send(crate::osinfo::OsDetectCmd {
                            host_id: m.id,
                            argv,
                            secret,
                        })
                        .is_ok()
                    {
                        self.os_detect_inflight.insert(m.id);
                    }
                }
            }
        }

        Ok(())
    }

    /// Apply a background OS auto-detect result: persist the detected os_icon,
    /// clear the inflight marker, and reload hosts so the UI reflects it.
    pub fn apply_os_detect(&mut self, ev: crate::osinfo::OsDetectEvent) -> Result<()> {
        let crate::osinfo::OsDetectEvent::Detected { host_id, os } = ev;
        self.store.update_host(
            host_id,
            &crate::store::HostUpdate {
                os_icon: Some(Some(os)),
                ..Default::default()
            },
        )?;
        self.os_detect_inflight.remove(&host_id);
        self.reload_hosts()?;
        Ok(())
    }
}
