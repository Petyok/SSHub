//! Deliver a stored password/passphrase to ssh via `SSH_ASKPASS` instead of
//! typing it into the PTY.
//!
//! With `SSH_ASKPASS_REQUIRE=force` (OpenSSH ≥ 8.4) ssh calls the askpass
//! helper for both passphrase and password prompts even on a tty, so the
//! "Enter passphrase for key …" / "…'s password:" line never appears on the
//! screen. The helper is this same binary re-executed in askpass mode (see
//! [`maybe_run_askpass`]); the secret is handed over through a private
//! `0600` file that is removed when the session ends.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A secret staged in a short-lived, owner-only file for `SSH_ASKPASS`.
pub struct AskpassSecret {
    path: PathBuf,
}

impl AskpassSecret {
    /// Write `secret` to a fresh `0600` file under `$XDG_RUNTIME_DIR` (or the
    /// system temp dir).
    pub fn new(secret: &str) -> std::io::Result<Self> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("sshub-askpass-{}-{}", std::process::id(), n));

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&path)?;
        // ssh strips the trailing newline from the askpass output.
        writeln!(f, "{secret}")?;
        Ok(Self { path })
    }

    /// Environment for the ssh child so it consults this helper.
    pub fn env(&self, exe: &Path) -> Vec<(String, String)> {
        vec![
            ("SSH_ASKPASS".into(), exe.to_string_lossy().into_owned()),
            ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
            (
                ASKPASS_FILE_ENV.into(),
                self.path.to_string_lossy().into_owned(),
            ),
        ]
    }
}

impl Drop for AskpassSecret {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

const ASKPASS_FILE_ENV: &str = "SSHUB_ASKPASS_FILE";

/// If this process was launched by ssh as its `SSH_ASKPASS` helper, print the
/// staged secret and return `true` (the caller should exit immediately). Set
/// only on the ssh child's environment, so the main TUI process never sees it.
pub fn maybe_run_askpass() -> bool {
    let Some(file) = std::env::var_os(ASKPASS_FILE_ENV) else {
        return false;
    };
    if let Ok(secret) = std::fs::read_to_string(&file) {
        // Content already ends with a newline; emit it verbatim.
        let mut out = std::io::stdout();
        let _ = out.write_all(secret.as_bytes());
        let _ = out.flush();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_secret_in_owner_only_file_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());

        let path;
        {
            let guard = AskpassSecret::new("s3cr3t").unwrap();
            let env = guard.env(std::path::Path::new("/usr/bin/sshub"));
            // File path is exposed via the env we hand to ssh.
            let file = env
                .iter()
                .find(|(k, _)| k == ASKPASS_FILE_ENV)
                .map(|(_, v)| v.clone())
                .unwrap();
            path = PathBuf::from(file);
            assert!(env.iter().any(|(k, v)| k == "SSH_ASKPASS_REQUIRE" && v == "force"));
            assert!(path.exists());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "s3cr3t\n");

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o600);
            }
        }
        // Dropped guard removes the file.
        assert!(!path.exists());

        std::env::remove_var("XDG_RUNTIME_DIR");
    }

    #[test]
    fn askpass_mode_off_without_env() {
        std::env::remove_var(ASKPASS_FILE_ENV);
        assert!(!maybe_run_askpass());
    }
}
