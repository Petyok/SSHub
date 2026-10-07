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

/// Path to hand ssh as its `SSH_ASKPASS` helper: this binary, re-executed.
///
/// Not simply `current_exe()`, which reads `/proc/self/exe` and reports
/// `<path> (deleted)` once the file has been replaced. An upgrade never writes
/// into the running binary: `npm install -g`, `cargo install`, a package manager
/// and `just install` all unlink the old inode and create a new one. ssh then
/// fails to exec that literal path, no stored secret is delivered, and what the
/// user sees is `Permission denied (publickey)`, which points at the server
/// rather than at the upgrade.
///
/// So when the path is gone, strip the marker and take the same path again: after
/// an in-place upgrade a new binary is sitting right there.
pub fn helper_exe() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if exe.exists() {
        return Ok(exe);
    }
    if let Some(replaced) = strip_deleted_marker(&exe) {
        return Ok(replaced);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!(
            "the sshub binary is no longer at {}, which happens when it is \
             upgraded while running; restart SSHub to restore password and \
             passphrase auth",
            exe.display()
        ),
    ))
}

/// `/proc/self/exe` for a replaced binary reads as `<path> (deleted)`. Returns
/// that path when it exists again, which is the normal outcome of an upgrade.
fn strip_deleted_marker(exe: &Path) -> Option<PathBuf> {
    let text = exe.to_str()?;
    let base = PathBuf::from(text.strip_suffix(" (deleted)")?);
    base.exists().then_some(base)
}

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
            (OWNER_ENV.into(), std::process::id().to_string()),
        ]
    }
}

impl Drop for AskpassSecret {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

const ASKPASS_FILE_ENV: &str = "SSHUB_ASKPASS_FILE";
/// The sshub process that staged the secret: where the ancestry walk stops.
const OWNER_ENV: &str = "SSHUB_ASKPASS_OWNER";

/// Whether the ssh that ran this helper is a ProxyJump/ProxyCommand hop: a
/// process named `ssh` sits between it and the sshub that staged the secret.
/// The hop inherits the destination's environment, so without this check it
/// would be handed the destination's password (#141). Only wrappers such as
/// `script` or `sh -c` sit above the destination ssh itself.
fn asked_by_jump_hop(owner: Option<u32>) -> bool {
    let Some((mut pid, _)) = parent_and_name(std::os::unix::process::parent_id()) else {
        return false;
    };
    for _ in 0..16 {
        if pid <= 1 || Some(pid) == owner {
            return false;
        }
        let Some((parent, name)) = parent_and_name(pid) else {
            return false;
        };
        if name == "ssh" {
            return true;
        }
        pid = parent;
    }
    false
}

/// Parent pid and command name of `pid`.
#[cfg(target_os = "linux")]
fn parent_and_name(pid: u32) -> Option<(u32, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `pid (comm) state ppid …`; comm may itself contain `) `.
    let (head, rest) = stat.rsplit_once(") ")?;
    let name = head.split_once(" (")?.1;
    let parent = rest.split_whitespace().nth(1)?.parse().ok()?;
    Some((parent, name.to_owned()))
}

/// Parent pid and command name of `pid`.
#[cfg(target_os = "macos")]
fn parent_and_name(pid: u32) -> Option<(u32, String)> {
    // SAFETY: an all-zero proc_bsdinfo is a valid value of this plain C struct.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a writable buffer of exactly `size` bytes.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    let name: Vec<u8> = info
        .pbi_comm
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    Some((info.pbi_ppid, String::from_utf8_lossy(&name).into_owned()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn parent_and_name(_pid: u32) -> Option<(u32, String)> {
    None
}

/// If this process was launched by ssh as its `SSH_ASKPASS` helper, print the
/// staged secret and return `true` (the caller should exit immediately). Set
/// only on the ssh child's environment, so the main TUI process never sees it.
pub fn maybe_run_askpass() -> bool {
    let Some(file) = std::env::var_os(ASKPASS_FILE_ENV) else {
        if std::env::var_os(super::askpass_channel::MODE_ENV).is_none()
            && std::env::var_os(super::askpass_channel::SOCKET_ENV).is_none()
            && std::env::var_os(super::askpass_channel::TOKEN_ENV).is_none()
        {
            return false;
        }
        let answer = (|| {
            let path = std::env::var_os(super::askpass_channel::SOCKET_ENV)?;
            let token = std::env::var(super::askpass_channel::TOKEN_ENV).ok()?;
            let prompt = std::env::args().nth(1)?;
            super::askpass_channel::request_answer(
                Path::new(&path),
                &token,
                &prompt,
                super::askpass_channel::asking_ssh_pid(),
                super::askpass_channel::TIMEOUT,
            )
            .ok()
        })();
        let Some(answer) = answer else {
            std::process::exit(1)
        };
        let mut out = std::io::stdout().lock();
        if writeln!(out, "{answer}")
            .and_then(|()| out.flush())
            .is_err()
        {
            std::process::exit(1);
        }
        return true;
    };
    let owner = std::env::var(OWNER_ENV)
        .ok()
        .and_then(|pid| pid.parse().ok());
    if asked_by_jump_hop(owner) {
        std::process::exit(1);
    }
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
    fn file_helper_entry() {
        if std::env::var_os("SSHUB_TEST_FILE_HELPER").is_some() {
            assert!(maybe_run_askpass());
            std::process::exit(0);
        }
    }

    #[test]
    fn staged_secret_is_never_handed_to_a_jump_hop() {
        // ProxyJump (#141): the jump ssh runs under the destination ssh and
        // inherits its askpass environment. Stand-ins named `ssh` (a copy of
        // bash) rebuild both process trees around a real helper process.
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join("ssh");
        std::fs::copy("/bin/bash", &ssh).unwrap();
        // Staged in this test's own dir: another test repoints XDG_RUNTIME_DIR.
        let guard = AskpassSecret {
            path: dir.path().join("secret"),
        };
        std::fs::write(&guard.path, "s3cr3t\n").unwrap();
        let helper = format!(
            "'{}' --exact session::askpass::tests::file_helper_entry --nocapture; echo helper-exit=$?",
            std::env::current_exe().unwrap().display()
        );
        let run = |script: &str| {
            let out = std::process::Command::new(&ssh)
                .args(["-c", script])
                .env("SSHUB_TEST_FILE_HELPER", "1")
                .envs(guard.env(Path::new("unused")))
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };

        // The destination: the helper's `ssh` was started by this process.
        let destination = run(&helper);
        assert!(destination.contains("s3cr3t"), "{destination}");
        assert!(destination.contains("helper-exit=0"), "{destination}");

        // A hop: the helper's `ssh` runs under another `ssh`.
        let hop = run(&format!(
            "'{}' -c \"{}\"; :",
            ssh.display(),
            helper.replace('$', "\\$")
        ));
        assert!(!hop.contains("s3cr3t"), "{hop}");
        assert!(hop.contains("helper-exit=1"), "{hop}");
    }

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
            assert!(env
                .iter()
                .any(|(k, v)| k == "SSH_ASKPASS_REQUIRE" && v == "force"));
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

#[cfg(test)]
mod exe_tests {
    use super::*;

    #[test]
    fn helper_exe_is_the_running_binary() {
        // The test binary exists, so this is the plain path with no marker.
        let exe = helper_exe().unwrap();
        assert!(exe.exists(), "{exe:?}");
        assert_eq!(exe, std::env::current_exe().unwrap());
    }

    #[test]
    fn a_replaced_binary_resolves_to_whatever_took_its_place() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("sshub");
        std::fs::write(&real, b"new binary").unwrap();

        // What /proc/self/exe reads once an upgrade replaced the file.
        let deleted = PathBuf::from(format!("{} (deleted)", real.display()));
        assert_eq!(
            strip_deleted_marker(&deleted).as_deref(),
            Some(real.as_path())
        );

        // Nothing took its place: no path to offer, and the caller must say so
        // rather than handing ssh something that cannot be executed.
        std::fs::remove_file(&real).unwrap();
        assert_eq!(strip_deleted_marker(&deleted), None);
    }

    #[test]
    fn a_path_without_the_marker_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("sshub");
        std::fs::write(&real, b"binary").unwrap();
        assert_eq!(strip_deleted_marker(&real), None);
    }
}
