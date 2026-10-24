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

/// Mode of the helper script. A test can make it non-executable, which is what
/// a `noexec` temp dir does to the exec of it (EACCES).
fn script_mode() -> u32 {
    #[cfg(test)]
    if let Some(mode) = TEST_SCRIPT_MODE.with(|m| m.get()) {
        return mode;
    }
    0o700
}

#[cfg(test)]
thread_local! {
    static TEST_SCRIPT_MODE: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

impl KeygenAskpass {
    /// Stage the helper in the temp dir, else in `$XDG_RUNTIME_DIR`: a `noexec`
    /// mount cannot run it, and ssh-keygen then ignores a missing askpass
    /// instead of failing. A directory is used only if the helper really runs.
    fn new(passphrase: &str) -> std::io::Result<Self> {
        let mut dirs = vec![std::env::temp_dir()];
        dirs.extend(std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from));
        let mut last = None;
        for dir in dirs {
            match Self::stage(&dir, passphrase) {
                Ok(staged) => return Ok(staged),
                Err(e) => last = Some(e),
            }
        }
        Err(last.expect("at least the temp dir was tried"))
    }

    fn stage(dir: &Path, passphrase: &str) -> std::io::Result<Self> {
        use std::io::Write;
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
        let secret_file = opts.open(&secret)?;
        // From here every early return drops `guard`, which removes what exists.
        // `script` stays empty until that file is really ours (`create_new`
        // succeeded), so a failed open never deletes someone else's file.
        let mut guard = Self {
            secret,
            script: PathBuf::new(),
        };
        // ssh strips the trailing newline from the askpass output.
        writeln!(&secret_file, "{passphrase}")?;

        let mut sopts = std::fs::OpenOptions::new();
        sopts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            sopts.mode(script_mode());
        }
        let mut script_file = sopts.open(&script)?;
        guard.script = script;
        // The secret path is passed via env so the script needs no quoting.
        write!(script_file, "#!/bin/sh\nexec cat \"$SSHUB_KG_SECRET\"\n")?;
        // Closed before the exec below: an open write handle is ETXTBSY.
        drop(script_file);

        // Run it once. A `noexec` dir fails here (EACCES); ETXTBSY is another
        // thread's fork still holding the write handle, gone after a moment.
        let mut attempt = 0;
        loop {
            let ran = Command::new(&guard.script)
                .env("SSHUB_KG_SECRET", &guard.secret)
                .output();
            match ran {
                Ok(out) if out.status.success() => return Ok(guard),
                Ok(out) => {
                    return Err(std::io::Error::other(format!(
                        "the askpass helper exited with {}",
                        out.status
                    )))
                }
                Err(e) if e.raw_os_error() == Some(26) && attempt < 20 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(e) => return Err(e),
            }
        }
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
    // The helper could not run: ssh-keygen then reports the key as unreadable
    // without ever having asked, which says nothing about the passphrase.
    if stderr.contains("ssh_askpass") {
        return None;
    }
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
    let pub_path = std::path::PathBuf::from(format!("{}.pub", target_path.display()));
    if pub_path.exists() {
        anyhow::bail!(
            "Public key file already exists: {}. Remove it before generating a new key.",
            pub_path.display()
        );
    }

    let mut cmd = Command::new("ssh-keygen");
    cmd.arg("-t").arg(key_type);
    if let Some(b) = bits {
        cmd.arg("-b").arg(b.to_string());
    }
    cmd.arg("-C").arg(comment);
    cmd.arg("-f").arg(target_path);

    // No passphrase needs no helper (and `-N ""` keeps ssh-keygen from asking).
    let _askpass = if passphrase.is_empty() {
        cmd.arg("-N").arg("");
        None
    } else {
        let askpass = KeygenAskpass::new(passphrase).context("create askpass helper")?;
        for (k, v) in askpass.env() {
            cmd.env(k, v);
        }
        Some(askpass)
    };

    let output = cmd.output().context("run ssh-keygen")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ssh-keygen failed: {}", stderr.trim());
    }
    // ssh-keygen skips an askpass it cannot run and writes the key without a
    // passphrase. Never hand that back as the protected key the user asked for.
    if !passphrase.is_empty() && key_is_encrypted(target_path) != Some(true) {
        let _ = std::fs::remove_file(target_path);
        let _ = std::fs::remove_file(&pub_path);
        anyhow::bail!(
            "the generated key is not passphrase-protected (ssh-keygen did not use the passphrase), so it was deleted"
        );
    }
    Ok(())
}

/// Read public key from path.pub, or extract it from private_key_path using ssh-keygen.
pub fn read_public_key(private_key_path: &Path, passphrase: Option<&str>) -> Result<String> {
    let expanded = crate::ssh::expand_tilde(&private_key_path.to_string_lossy());
    let path = Path::new(&expanded);

    // 1. Try reading the .pub file first
    let pub_path = std::path::PathBuf::from(format!("{}.pub", path.display()));
    if pub_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&pub_path) {
            return Ok(content.trim().to_string());
        }
    }

    // 2. Extract on the fly via ssh-keygen -y
    let pass = passphrase.unwrap_or("");
    let askpass = KeygenAskpass::new(pass).context("create askpass helper")?;
    let mut cmd = Command::new("ssh-keygen");
    cmd.arg("-y").arg("-f").arg(path);
    for (k, v) in askpass.env() {
        cmd.env(k, v);
    }
    let output = cmd.output().context("run ssh-keygen -y")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to extract public key: {}", stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
    {
        let mut dir = std::fs::DirBuilder::new();
        dir.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            dir.mode(0o700);
        }
        dir.create(&ssh_dir).context("create ~/.ssh")?;
    }

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

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // 0600 from the first byte: never a window where the key is
            // readable under the process umask.
            opts.mode(0o600);
        }
        let mut file = match opts.open(&dest) {
            Ok(f) => f,
            // Lost a race for this name: take the next suffix.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("write key file {}", dest.display())),
        };
        use std::io::Write as _;
        file.write_all(body.as_bytes())
            .with_context(|| format!("write key file {}", dest.display()))?;
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
        crate::config::with_test_home(dir.path(), || {
            let blob =
                "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";
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
        });
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

    #[test]
    fn read_public_key_supports_dots_in_filename() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("prod.ed25519");
        let pub_path = dir.path().join("prod.ed25519.pub");

        let expected_key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPxyz test_comment";
        std::fs::write(&pub_path, expected_key).unwrap();

        let key = read_public_key(&key_path, None).unwrap();
        assert_eq!(key, expected_key);
    }

    /// Oracle (real ssh-keygen): with a helper that cannot run, as under a
    /// `noexec` temp dir, ssh-keygen only prints `ssh_askpass: exec(...)` and
    /// carries on. A passphrase the user asked for must never turn into a
    /// silently unencrypted key, and a probe must not call a correct passphrase
    /// wrong.
    #[test]
    fn a_helper_that_cannot_run_never_yields_an_unencrypted_key() {
        if !ssh_keygen_available() {
            eprintln!("skipping: ssh-keygen not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("id");
        TEST_SCRIPT_MODE.with(|m| m.set(Some(0o600)));
        let generated = generate_key_pair("ed25519", None, "hunter22", "c", &key);
        TEST_SCRIPT_MODE.with(|m| m.set(None));
        let opens_without_passphrase = Command::new("ssh-keygen")
            .args(["-y", "-P", "", "-f"])
            .arg(&key)
            .output()
            .unwrap()
            .status
            .success();
        assert!(
            generated.is_err() || !opens_without_passphrase,
            "ssh-keygen was told to encrypt the key and silently did not"
        );
        assert!(!key.exists() && !dir.path().join("id.pub").exists());
    }

    #[test]
    fn a_key_without_a_passphrase_needs_no_helper() {
        if !ssh_keygen_available() {
            eprintln!("skipping: ssh-keygen not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("id");
        TEST_SCRIPT_MODE.with(|m| m.set(Some(0o600)));
        let generated = generate_key_pair("ed25519", None, "", "c", &key);
        TEST_SCRIPT_MODE.with(|m| m.set(None));
        generated.unwrap();
        assert!(Command::new("ssh-keygen")
            .args(["-y", "-P", "", "-f"])
            .arg(&key)
            .output()
            .unwrap()
            .status
            .success());
    }

    #[test]
    fn a_helper_that_cannot_run_does_not_make_a_right_passphrase_wrong() {
        if !ssh_keygen_available() {
            eprintln!("skipping: ssh-keygen not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("id");
        generate_key_pair("ed25519", None, "hunter22", "c", &key).unwrap();
        assert_eq!(passphrase_matches(&key, "hunter22"), Some(true));
        TEST_SCRIPT_MODE.with(|m| m.set(Some(0o600)));
        let verdict = passphrase_matches(&key, "hunter22");
        TEST_SCRIPT_MODE.with(|m| m.set(None));
        assert_ne!(verdict, Some(false), "a correct passphrase called wrong");
    }

    #[test]
    fn a_generated_key_is_encrypted_and_opens_with_its_passphrase() {
        if !ssh_keygen_available() {
            eprintln!("skipping: ssh-keygen not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("id");
        generate_key_pair("ed25519", None, "hunter22", "c", &key).unwrap();
        assert_eq!(key_is_encrypted(&key), Some(true));
        assert_eq!(passphrase_matches(&key, "hunter22"), Some(true));
        assert_eq!(passphrase_matches(&key, "other"), Some(false));
    }
}
