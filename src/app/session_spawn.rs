use super::*;

impl App {
    /// Spawn an embedded PTY session running `argv` and switch into
    /// `Connecting` mode. Shared by ad-hoc connect and local-shell: the
    /// caller passes a complete argv, so this does NOT inject `-v` /
    /// accept-new like the ssh-only path in connect.rs.
    pub(crate) fn spawn_embedded_session(
        &mut self,
        argv: Vec<String>,
        display_name: String,
        meta: crate::session::SessionMeta,
        pending_secret: Option<crate::session::PendingSecret>,
        log_host_name: &str,
    ) -> Result<()> {
        let now_ts = || {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        };

        if let Some(first_cmd) = argv.first() {
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
                    host_name: log_host_name.to_string(),
                    line: msg.clone(),
                    level: crate::ssh::probe::LogLevel::Error,
                    timestamp: now_ts(),
                });
                self.host_notice = Some(msg);
                return Ok(());
            }
        }

        self.push_ssh_log(crate::ssh::probe::SshLogEntry {
            host_name: log_host_name.to_string(),
            line: format!("$ {}", argv.join(" ")),
            level: crate::ssh::probe::LogLevel::Success,
            timestamp: now_ts(),
        });

        let rows = self.terminal_area.height.max(3);
        let cols = self.terminal_area.width.max(20);
        let config = crate::session::SessionConfig {
            argv,
            display_name,
            meta,
            pending_secret,
        };
        match crate::session::Session::spawn(config, rows, cols) {
            Ok(session) => {
                self.sessions.push(session);
                self.active_session = Some(self.sessions.len() - 1);
                self.mode = AppMode::Connecting;
                let _ = self.store.log_auth_event(
                    log_host_name,
                    None,
                    "direct",
                    "launched",
                    "session started",
                );
                Ok(())
            }
            Err(e) => {
                let err_msg = format!("Session spawn failed: {e:#}");
                let _ = self
                    .store
                    .log_auth_event(log_host_name, None, "direct", "fail", &err_msg);
                self.push_ssh_log(crate::ssh::probe::SshLogEntry {
                    host_name: log_host_name.to_string(),
                    line: err_msg.clone(),
                    level: crate::ssh::probe::LogLevel::Error,
                    timestamp: now_ts(),
                });
                self.host_notice = Some(err_msg);
                Ok(())
            }
        }
    }
}
