//! `ssh-copy-id` inside an embedded PTY session tab.
//!
//! Installs a chosen identity's public key on a remote host by running
//! `ssh-copy-id -i <key> [user@]host` in a normal embedded session, so the user
//! types the login password interactively in the terminal (no secret ever
//! touches argv or a stored credential). Two entry points:
//!
//!  - hosts tab: [`App::copy_id_selected_host`] uses the selected host's bound
//!    identity key directly.
//!  - identities tab: [`App::open_copy_id_host_picker`] opens a searchable host
//!    picker so the highlighted identity's key can be pushed to any saved host.
//!
//! All argv construction goes through the pure, injection-safe
//! [`build_copy_id_argv`].

use super::*;
use std::path::Path;

/// Popup state for choosing which saved host receives an identity's key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyIdHostPicker {
    /// Live filter query typed into the picker.
    pub query: String,
    /// Index into the filtered host list.
    pub selected: usize,
    /// Private key of the identity that was highlighted when the picker opened.
    pub key_path: std::path::PathBuf,
}

/// Pure. Build an injection-safe `ssh-copy-id` argv.
///
/// Shape: `["ssh-copy-id", "-i", "<key>", ("-p", "<port>")?, "<user@host>" |
/// "<host>"]` — the `-p <port>` pair is present only when `port != 22`.
///
/// Returns `None` when the destination is unsafe: an empty/whitespace host, a
/// host or user beginning with `-` (which ssh-copy-id would read as an option),
/// or any whitespace/control character in the host or user.
pub fn build_copy_id_argv(key: &Path, user: Option<&str>, host: &str, port: u16) -> Option<Vec<String>> {
    if host.trim().is_empty() || host.starts_with('-') {
        return None;
    }
    if host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    if let Some(u) = user {
        if u.starts_with('-') || u.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return None;
        }
    }

    let mut argv = vec![
        "ssh-copy-id".to_string(),
        "-i".to_string(),
        key.display().to_string(),
    ];
    if port != 22 {
        argv.push("-p".to_string());
        argv.push(port.to_string());
    }
    let dest = match user {
        Some(u) if !u.is_empty() => format!("{u}@{host}"),
        _ => host.to_string(),
    };
    argv.push(dest);
    Some(argv)
}

impl App {
    /// HOSTS TAB: push the selected host's identity key to that host.
    ///
    /// The key comes from the host's bound identity (`private_key`). Without a
    /// managed host or a bound key nothing is spawned; a notice explains why.
    pub fn copy_id_selected_host(&mut self) -> Result<()> {
        // Pull owned copies out of the immutable selected-entry borrow so we can
        // mutate `self` (notice / session spawn) afterwards.
        let extracted = self.selected_entry().and_then(|e| e.managed()).map(|m| {
            let key = m.identity.as_ref().and_then(|i| i.private_key.clone());
            let user = m
                .username
                .clone()
                .or_else(|| m.identity.as_ref().and_then(|i| i.username.clone()));
            (key, user, m.address.clone(), m.port)
        });

        let Some((key, user, host, port)) = extracted else {
            self.host_notice = Some("copy-id needs a managed host".into());
            return Ok(());
        };
        let Some(key) = key else {
            self.host_notice = Some("host has no identity key to copy".into());
            return Ok(());
        };

        let Some(argv) = build_copy_id_argv(&key, user.as_deref(), &host, port) else {
            self.host_notice = Some("unsafe host/user for copy-id".into());
            return Ok(());
        };

        let meta = crate::session::SessionMeta {
            user,
            address: Some(host.clone()),
            port: Some(port),
            ..Default::default()
        };
        self.spawn_embedded_session(argv, format!("copy-id {host}"), meta, None, &host)
    }

    /// IDENTITIES TAB: open a searchable host picker for the highlighted
    /// identity's key. Without a private key nothing opens; a notice explains.
    pub fn open_copy_id_host_picker(&mut self) -> Result<()> {
        let key = self.selected_identity().and_then(|i| i.private_key.clone());
        let Some(key) = key else {
            self.identity_notice = Some("identity has no private key to copy".into());
            return Ok(());
        };
        self.copyid_picker = Some(CopyIdHostPicker {
            query: String::new(),
            selected: 0,
            key_path: key,
        });
        self.mode = AppMode::CopyIdHostPicker;
        Ok(())
    }

    /// The host picker's filtered list, `(host index, label)`. Built on top of
    /// [`App::session_host_matches`] (the shared host list) and further filtered
    /// by the copy-id picker's own query.
    pub(crate) fn copyid_host_matches(&self) -> Vec<(usize, String)> {
        let query = self
            .copyid_picker
            .as_ref()
            .map(|p| p.query.to_lowercase())
            .unwrap_or_default();
        self.session_host_matches()
            .into_iter()
            .filter(|(_, label)| query.is_empty() || label.to_lowercase().contains(&query))
            .collect()
    }

    pub(crate) fn handle_key_copyid_host_picker(&mut self, key: KeyEvent) -> Result<()> {
        let len = self.copyid_host_matches().len();
        match key.code {
            KeyCode::Esc => {
                self.copyid_picker = None;
                self.mode = AppMode::Normal;
            }
            KeyCode::Down => {
                if len > 0 {
                    if let Some(p) = self.copyid_picker.as_mut() {
                        p.selected = (p.selected + 1) % len;
                    }
                }
            }
            KeyCode::Up => {
                if len > 0 {
                    if let Some(p) = self.copyid_picker.as_mut() {
                        p.selected = (p.selected + len - 1) % len;
                    }
                }
            }
            KeyCode::Enter => self.finish_copy_id_pick()?,
            KeyCode::Backspace => {
                if let Some(p) = self.copyid_picker.as_mut() {
                    p.query.pop();
                    p.selected = 0;
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                if let Some(p) = self.copyid_picker.as_mut() {
                    p.query.push(c);
                    p.selected = 0;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Resolve the highlighted host, build the argv from the picker's stored
    /// key, close the picker, and spawn the copy-id session.
    pub(crate) fn finish_copy_id_pick(&mut self) -> Result<()> {
        let matches = self.copyid_host_matches();
        let host_idx = self
            .copyid_picker
            .as_ref()
            .and_then(|p| matches.get(p.selected))
            .map(|(idx, _)| *idx);
        let key_path = self.copyid_picker.as_ref().map(|p| p.key_path.clone());

        self.copyid_picker = None;
        self.mode = AppMode::Normal;

        let (Some(idx), Some(key)) = (host_idx, key_path) else {
            return Ok(());
        };

        // Derive user/host/port from the chosen entry; a non-managed entry
        // (ssh_config alias / legacy) falls back to its name and defaults.
        let derived = self.hosts.get(idx).map(|entry| match entry.managed() {
            Some(m) => (
                m.username
                    .clone()
                    .or_else(|| m.identity.as_ref().and_then(|i| i.username.clone())),
                m.address.clone(),
                m.port,
            ),
            None => (None, entry.name().to_string(), 22u16),
        });
        let Some((user, host, port)) = derived else {
            return Ok(());
        };

        let Some(argv) = build_copy_id_argv(&key, user.as_deref(), &host, port) else {
            self.host_notice = Some("unsafe host/user for copy-id".into());
            return Ok(());
        };
        let meta = crate::session::SessionMeta {
            user,
            address: Some(host.clone()),
            port: Some(port),
            ..Default::default()
        };
        self.spawn_embedded_session(argv, format!("copy-id {host}"), meta, None, &host)
    }
}

#[cfg(test)]
mod tests {
    use super::build_copy_id_argv;
    use std::path::Path;

    #[test]
    fn builds_with_user_default_port() {
        let argv = build_copy_id_argv(Path::new("/keys/id_ed25519"), Some("root"), "example.com", 22)
            .unwrap();
        assert_eq!(
            argv,
            vec![
                "ssh-copy-id".to_string(),
                "-i".to_string(),
                "/keys/id_ed25519".to_string(),
                "root@example.com".to_string(),
            ]
        );
        // Port 22 omits the `-p` pair entirely.
        assert!(!argv.iter().any(|a| a == "-p"));
    }

    #[test]
    fn builds_without_user() {
        let argv =
            build_copy_id_argv(Path::new("/keys/id_ed25519"), None, "example.com", 22).unwrap();
        assert_eq!(argv.last().unwrap(), "example.com");
        assert!(!argv.iter().any(|a| a.contains('@')));
    }

    #[test]
    fn empty_user_is_treated_as_no_user() {
        let argv =
            build_copy_id_argv(Path::new("/keys/id_ed25519"), Some(""), "example.com", 22).unwrap();
        assert_eq!(argv.last().unwrap(), "example.com");
    }

    #[test]
    fn non_default_port_includes_flag() {
        let argv = build_copy_id_argv(Path::new("/keys/id"), Some("admin"), "10.0.0.1", 2222)
            .unwrap();
        let p = argv.iter().position(|a| a == "-p").unwrap();
        assert_eq!(argv[p + 1], "2222");
        assert_eq!(argv.last().unwrap(), "admin@10.0.0.1");
    }

    #[test]
    fn rejects_leading_dash_host() {
        assert!(build_copy_id_argv(Path::new("/k"), None, "-oProxyCommand=x", 22).is_none());
        assert!(build_copy_id_argv(Path::new("/k"), Some("root"), "-lroot", 22).is_none());
    }

    #[test]
    fn rejects_leading_dash_user() {
        assert!(build_copy_id_argv(Path::new("/k"), Some("-bad"), "example.com", 22).is_none());
    }

    #[test]
    fn rejects_empty_or_whitespace_host() {
        assert!(build_copy_id_argv(Path::new("/k"), None, "", 22).is_none());
        assert!(build_copy_id_argv(Path::new("/k"), None, "   ", 22).is_none());
        assert!(build_copy_id_argv(Path::new("/k"), None, "ex ample.com", 22).is_none());
    }
}
