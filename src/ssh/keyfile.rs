//! Writing pasted private-key material into `~/.ssh` as proper key files.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use std::sync::atomic::{AtomicU64, Ordering};

static ASKPASS_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A passphrase staged for `SSH_ASKPASS`: a `0600` secret file plus a tiny
/// `0700` helper script that just `cat`s it. Both are removed on drop. This
/// keeps the passphrase out of ssh-keygen's argv (where `ps` would expose it)
/// and, unlike reusing the app binary as the helper, works in unit tests too.
struct KeygenAskpass {
    secret: PathBuf,
    script: PathBuf,
}

impl KeygenAskpass {
    fn new(passphrase: &str) -> std::io::Result<Self> {
        use std::io::Write;
        let dir = std::env::temp_dir();
        let n = ASKPASS_COUNTER.fetch_add(1, Ordering::Relaxed);
        let stem = format!("sshub-kg-{}-{}", std::process::id(), n);
        let secret = dir.join(format!("{stem}.secret"));
        let script = dir.join(format!("{stem}.sh"));

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        // ssh strips the trailing newline from the askpass output.
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

/// Run `ssh-keygen -y -f <path>` and classify the outcome. The passphrase is
/// handed to ssh-keygen through `SSH_ASKPASS` rather than as a `-P` command-line
/// argument, so it never appears in the process argument list where any local
/// user could read it via `ps`.
///
/// Returns `None` when the answer is unknown (ssh-keygen missing, file
/// unreadable, or an error unrelated to encryption) so callers can fail open.
fn probe_key(path: &Path, passphrase: &str) -> Option<KeyProbe> {
    // Staged files are removed when `_askpass` drops at end of scope.
    let _askpass = KeygenAskpass::new(passphrase).ok()?;
    let mut cmd = Command::new("ssh-keygen");
    cmd.arg("-y").arg("-f").arg(path);
    for (k, v) in _askpass.env() {
        cmd.env(k, v);
    }
    let output = cmd.output().ok()?;
    if output.status.success() {
        return Some(KeyProbe::Ok);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    if stderr.contains("passphrase") || stderr.contains("incorrect") {
        Some(KeyProbe::WrongPassphrase)
    } else {
        None // parse error / not a key / etc — don't block on it
    }
}

enum KeyProbe {
    Ok,
    WrongPassphrase,
}

/// Whether the key at `path` is passphrase-protected.
/// `Some(true)`/`Some(false)` when determinable, `None` when unknown.
pub fn key_is_encrypted(path: &Path) -> Option<bool> {
    match probe_key(path, "") {
        Some(KeyProbe::Ok) => Some(false),
        Some(KeyProbe::WrongPassphrase) => Some(true),
        None => None,
    }
}

/// Whether `passphrase` correctly decrypts the key at `path`.
/// `None` when it can't be determined (e.g. ssh-keygen unavailable).
pub fn passphrase_matches(path: &Path, passphrase: &str) -> Option<bool> {
    match probe_key(path, passphrase) {
        Some(KeyProbe::Ok) => Some(true),
        Some(KeyProbe::WrongPassphrase) => Some(false),
        None => None,
    }
}

/// Generate a new SSH key pair using `ssh-keygen`.
/// Uses `KeygenAskpass` to stage the optional passphrase, keeping it off argv.
pub fn generate_key_pair(
    key_type: &str,
    bits: Option<u32>,
    passphrase: &str,
    comment: &str,
    target_path: &Path,
) -> Result<()> {
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent).context("create target directory")?;
    }
    if target_path.exists() {
        anyhow::bail!("Key file already exists: {}", target_path.display());
    }

    let askpass = KeygenAskpass::new(passphrase).context("create askpass helper")?;
    let mut cmd = Command::new("ssh-keygen");
    cmd.arg("-t").arg(key_type);
    if let Some(b) = bits {
        cmd.arg("-b").arg(b.to_string());
    }
    cmd.arg("-C").arg(comment);
    cmd.arg("-f").arg(target_path);

    for (k, v) in askpass.env() {
        cmd.env(k, v);
    }

    let output = cmd.output().context("run ssh-keygen")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ssh-keygen failed: {}", stderr.trim());
    }
    Ok(())
}


/// Does `text` look like pasted private-key material (rather than a path)?
pub fn looks_like_private_key(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("-----BEGIN") && t.contains("PRIVATE KEY-----")
}

/// Write pasted key material to `~/.ssh/sshub_<name>` with `0600` permissions
/// and return the path.
///
/// If the destination already holds the same content it is reused; a name
/// collision with *different* content gets a numeric suffix so an existing
/// key is never overwritten.
pub fn write_key_material(name: &str, contents: &str) -> Result<PathBuf> {
    let home =
        std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME environment variable not set"))?;
    let ssh_dir = PathBuf::from(home).join(".ssh");
    std::fs::create_dir_all(&ssh_dir).context("create ~/.ssh")?;

    let safe_name: String = name
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
    let safe_name = if safe_name.is_empty() {
        "key".to_string()
    } else {
        safe_name
    };

    // Key files must end with a newline or ssh rejects them.
    let mut body = contents.trim_end().to_string();
    body.push('\n');

    for attempt in 0..100 {
        let file_name = if attempt == 0 {
            format!("sshub_{safe_name}")
        } else {
            format!("sshub_{safe_name}-{}", attempt + 1)
        };
        let dest = ssh_dir.join(file_name);

        if dest.exists() {
            match std::fs::read(&dest) {
                Ok(existing) if existing == body.as_bytes() => return Ok(dest),
                _ => continue, // different key under this name — try next suffix
            }
        }

        std::fs::write(&dest, &body)
            .with_context(|| format!("write key file {}", dest.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600))?;
        }
        return Ok(dest);
    }
    anyhow::bail!("too many conflicting sshub_{safe_name}* key files in ~/.ssh")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_key_material_vs_path() {
        assert!(looks_like_private_key(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----"
        ));
        assert!(looks_like_private_key(
            "  -----BEGIN RSA PRIVATE KEY-----\nabc"
        ));
        assert!(!looks_like_private_key("~/.ssh/id_ed25519"));
        assert!(!looks_like_private_key("-----BEGIN CERTIFICATE-----\nabc"));
    }

    fn ssh_keygen_available() -> bool {
        Command::new("ssh-keygen")
            .arg("-?")
            .output()
            .map(|o| o.status.code().is_some())
            .unwrap_or(false)
    }

    #[test]
    fn detects_encrypted_vs_plain_keys() {
        if !ssh_keygen_available() {
            eprintln!("skipping: ssh-keygen not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain");
        let enc = dir.path().join("enc");

        // Unencrypted key.
        Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-f"])
            .arg(&plain)
            .output()
            .unwrap();
        // Passphrase-protected key.
        Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "secret123", "-f"])
            .arg(&enc)
            .output()
            .unwrap();

        assert_eq!(key_is_encrypted(&plain), Some(false));
        assert_eq!(key_is_encrypted(&enc), Some(true));
        assert_eq!(passphrase_matches(&enc, "secret123"), Some(true));
        assert_eq!(passphrase_matches(&enc, "wrong"), Some(false));
        // Missing file → unknown, never blocks.
        assert_eq!(key_is_encrypted(&dir.path().join("nope")), None);
    }

    #[test]
    fn writes_key_with_owner_only_perms_and_dedups() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());

        let blob = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";
        let p1 = write_key_material("work laptop", blob).unwrap();
        assert!(p1.ends_with("sshub_work_laptop"));
        let written = std::fs::read_to_string(&p1).unwrap();
        assert!(written.ends_with("-----END OPENSSH PRIVATE KEY-----\n"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p1).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // Same content → same file; different content → suffixed file.
        let p2 = write_key_material("work laptop", blob).unwrap();
        assert_eq!(p1, p2);
        let p3 = write_key_material(
            "work laptop",
            "-----BEGIN OPENSSH PRIVATE KEY-----\nother\n-----END OPENSSH PRIVATE KEY-----",
        )
        .unwrap();
        assert_ne!(p1, p3);
        assert!(p3.to_string_lossy().contains("sshub_work_laptop-2"));

        std::env::remove_var("HOME");
    }

    #[test]
    fn generates_key_pairs_correctly() {
        if !ssh_keygen_available() {
            eprintln!("skipping: ssh-keygen not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("id_ed25519");

        generate_key_pair("ed25519", None, "mypass123", "test_comment", &key_path).unwrap();

        assert!(key_path.exists());
        assert!(dir.path().join("id_ed25519.pub").exists());
        assert_eq!(key_is_encrypted(&key_path), Some(true));
        assert_eq!(passphrase_matches(&key_path, "mypass123"), Some(true));
        assert_eq!(passphrase_matches(&key_path, "wrong"), Some(false));
    }
}

