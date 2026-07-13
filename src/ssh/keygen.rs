//! Generating a fresh ed25519 keypair via `ssh-keygen`, with the passphrase
//! delivered through a staged `SSH_ASKPASS` helper rather than the process
//! argument list (where any local user could read it via `ps`).

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static ASKPASS_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The pair of paths written by [`generate_ed25519`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeygenOutcome {
    pub private_key: PathBuf,
    pub public_key: PathBuf,
}

/// A passphrase staged for `SSH_ASKPASS`: a `0600` secret file plus a tiny
/// `0700` helper script that just `cat`s it. Both are removed on drop. This
/// mirrors `keyfile.rs::KeygenAskpass` and keeps the passphrase out of
/// ssh-keygen's argv. An empty passphrase writes a blank line, which ssh-keygen
/// reads as "no passphrase".
struct KeygenAskpass {
    secret: PathBuf,
    script: PathBuf,
}

impl KeygenAskpass {
    fn new(passphrase: &str) -> std::io::Result<Self> {
        use std::io::Write;
        let dir = std::env::temp_dir();
        let n = ASKPASS_COUNTER.fetch_add(1, Ordering::Relaxed);
        let stem = format!("sshub-kg-new-{}-{}", std::process::id(), n);
        let secret = dir.join(format!("{stem}.secret"));
        let script = dir.join(format!("{stem}.sh"));

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        // ssh strips the trailing newline from the askpass output; an empty
        // passphrase therefore stays empty.
        writeln!(opts.open(&secret)?, "{passphrase}")?;

        let mut sopts = std::fs::OpenOptions::new();
        sopts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            sopts.mode(0o700);
        }
        // The secret path is passed via env so the script needs no quoting.
        write!(
            sopts.open(&script)?,
            "#!/bin/sh\nexec cat \"$SSHUB_KG_SECRET\"\n"
        )?;

        Ok(Self { secret, script })
    }

    fn env(&self) -> Vec<(String, String)> {
        vec![
            (
                "SSH_ASKPASS".into(),
                self.script.to_string_lossy().into_owned(),
            ),
            ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
            (
                "SSHUB_KG_SECRET".into(),
                self.secret.to_string_lossy().into_owned(),
            ),
            // Some ssh-keygen builds still consult DISPLAY before askpass.
            ("DISPLAY".into(), ":0".into()),
        ]
    }
}

impl Drop for KeygenAskpass {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.secret);
        let _ = std::fs::remove_file(&self.script);
    }
}

/// Reduce an arbitrary identity name to a safe filename component. Mirrors the
/// sanitizer in `keyfile.rs::write_key_material`.
fn sanitize_name(name: &str) -> String {
    let safe: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        "key".to_string()
    } else {
        safe
    }
}

/// Generate an ed25519 keypair at `dir/<sanitized name>` (+ `.pub`).
///
/// The passphrase (empty string or `None` => none) reaches ssh-keygen through a
/// staged `SSH_ASKPASS` helper, never via `-N`/argv. ssh-keygen prompts for the
/// passphrase twice (enter + confirm); the helper answers both. Never
/// overwrites: bails if either target path already exists. The private key is
/// chmod'd `0600` on unix.
pub fn generate_ed25519(dir: &Path, name: &str, passphrase: Option<&str>) -> Result<KeygenOutcome> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("create key directory {}", dir.display()))?;

    let safe_name = sanitize_name(name);
    let dest = dir.join(&safe_name);
    let public = dest.with_extension("pub");

    if dest.exists() || public.exists() {
        anyhow::bail!(
            "a key named \"{safe_name}\" already exists in {} — refusing to overwrite",
            dir.display()
        );
    }

    // Empty string and None both mean "no passphrase" (blank askpass answer).
    let secret = passphrase.unwrap_or("");
    let askpass = KeygenAskpass::new(secret).context("stage passphrase helper")?;

    let mut cmd = Command::new("ssh-keygen");
    // No `-N`: the passphrase is answered interactively through SSH_ASKPASS.
    cmd.args(["-t", "ed25519", "-q", "-f"]).arg(&dest);
    for (k, v) in askpass.env() {
        cmd.env(k, v);
    }
    let output = cmd
        .output()
        .context("failed to run ssh-keygen (is it installed?)")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ssh-keygen failed: {}", stderr.trim());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600));
    }

    Ok(KeygenOutcome {
        private_key: dest,
        public_key: public,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_keygen_available() -> bool {
        Command::new("ssh-keygen")
            .arg("-?")
            .output()
            .map(|o| o.status.code().is_some())
            .unwrap_or(false)
    }

    #[test]
    fn generates_plaintext_key_with_owner_only_perms() {
        if !ssh_keygen_available() {
            eprintln!("skipping: ssh-keygen not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = generate_ed25519(dir.path(), "work laptop", None).unwrap();

        // "work laptop" → "work_laptop".
        assert!(out.private_key.ends_with("work_laptop"));
        assert!(out.public_key.ends_with("work_laptop.pub"));
        assert!(out.private_key.exists());
        assert!(out.public_key.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&out.private_key)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        assert_eq!(crate::ssh::key_is_encrypted(&out.private_key), Some(false));
    }

    #[test]
    fn generates_passphrase_protected_key() {
        if !ssh_keygen_available() {
            eprintln!("skipping: ssh-keygen not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = generate_ed25519(dir.path(), "secure", Some("secret")).unwrap();

        assert!(out.private_key.exists());
        assert_eq!(
            crate::ssh::passphrase_matches(&out.private_key, "secret"),
            Some(true)
        );
        assert_eq!(
            crate::ssh::passphrase_matches(&out.private_key, "wrong"),
            Some(false)
        );
    }

    #[test]
    fn refuses_to_overwrite_existing_key() {
        if !ssh_keygen_available() {
            eprintln!("skipping: ssh-keygen not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        generate_ed25519(dir.path(), "dup", None).unwrap();
        // Second attempt with the same name/dir must error rather than clobber.
        assert!(generate_ed25519(dir.path(), "dup", None).is_err());
    }
}
