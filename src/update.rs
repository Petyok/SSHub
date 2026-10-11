//! Release check and in-place update (#149).
//!
//! The check asks GitHub which release is the latest; the install goes through
//! the channel the binary came from, so npm and cargo keep their own record of
//! what is installed. Everything shells out (`curl`, `tar`, `npm`, `cargo`)
//! from a worker thread: no HTTP client or archive crate in the binary, and the
//! event loop never waits on the network.

use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::LazyLock;

use anyhow::{anyhow, bail, Context, Result};

const REPO: &str = "Petyok/SSHub";

/// A worker's answer, drained by the event loop.
pub enum UpdateEvent {
    /// `Some(tag)` when the latest release is newer than the running binary.
    Checked(Result<Option<String>>),
    /// The install finished (or did not).
    Installed(Result<()>),
}

/// Ask GitHub for the latest release on a worker thread.
pub fn spawn_check() -> Receiver<UpdateEvent> {
    spawn(|| {
        UpdateEvent::Checked(
            latest_tag().map(|tag| is_newer(&tag, env!("CARGO_PKG_VERSION")).then_some(tag)),
        )
    })
}

/// Install release `tag` on a worker thread.
pub fn spawn_install(tag: String) -> Receiver<UpdateEvent> {
    spawn(move || UpdateEvent::Installed(install(&tag)))
}

fn spawn(job: impl FnOnce() -> UpdateEvent + Send + 'static) -> Receiver<UpdateEvent> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(job());
    });
    rx
}

fn latest_tag() -> Result<String> {
    let body = run(Command::new("curl")
        .args(["-fsSL", "--proto-redir", "=https", "--max-time", "15"])
        .args(["-H", "Accept: application/vnd.github+json"])
        .arg(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        )))?;
    let release: serde_json::Value =
        serde_json::from_str(&body).context("GitHub sent unreadable release info")?;
    release["tag_name"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("GitHub release info has no tag_name"))
}

/// Whether release `tag` (`v1.2.3`) is newer than version `current` (`1.2.2`).
/// Anything that is not plain `MAJOR.MINOR.PATCH` (a pre-release, a typo) is
/// never announced.
fn is_newer(tag: &str, current: &str) -> bool {
    fn parse(s: &str) -> Option<(u64, u64, u64)> {
        let mut parts = s.trim().trim_start_matches('v').split('.');
        let mut next = || parts.next()?.parse::<u64>().ok();
        let version = (next()?, next()?, next()?);
        parts.next().is_none().then_some(version)
    }
    matches!((parse(tag), parse(current)), (Some(latest), Some(current)) if latest > current)
}

fn install(tag: &str) -> Result<()> {
    let version = tag.trim_start_matches('v');
    let exe = std::env::current_exe().unwrap_or_default();
    // npx fetches the newest version on every run; a global install is not
    // what its user asked for.
    if exe.components().any(|c| c.as_os_str() == "_npx") {
        bail!("started with npx: `npx sshub-tui@latest` runs {tag}");
    }
    // Run outside the launch directory, whose toolchain pin or `.npmrc` must
    // not steer a global install.
    let global = |program: &str| {
        let mut cmd = Command::new(program);
        cmd.current_dir(std::env::temp_dir());
        cmd
    };
    match install_channel().as_deref() {
        Some("npm") => run(global("npm").args(["install", "-g", &format!("sshub-tui@{version}")])),
        Some("cargo") => run(global("cargo").args(["install", "sshub", "--version", version])),
        None => replace_binary(tag),
        Some("source") => bail!("built from source: update the checkout and run `just install`"),
        Some(other) => bail!("installed via {other}: update it there"),
    }
    .map(drop)
}

/// How the running binary was installed: shown next to the version, so a
/// support conversation can tell how the binary got onto the machine, and the
/// route an update takes.
///
/// Detection, most specific first:
///
/// * `SSHUB_INSTALL_CHANNEL` set → that value (the npm shim sets `npm`; the
///   prebuilt npm binary is byte-identical to the release tarball one, so no
///   path heuristic can tell them apart).
/// * the binary lives in cargo's install bin dir (`$CARGO_HOME/bin`) → `cargo`.
/// * the binary sits inside a build-target dir (`target/`, `…-target/`) →
///   `source` — a `cargo run`/`cargo build` straight out of the checkout.
/// * the binary lives in `~/.local/bin` → `source` (`just install`).
/// * otherwise → `None`: a release tarball, a distro package or a manual copy
///   shows no suffix.
pub fn install_channel() -> Option<String> {
    static CHANNEL: LazyLock<Option<String>> = LazyLock::new(|| {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let cargo_bin = match std::env::var_os("CARGO_HOME") {
            Some(dir) => Some(std::path::PathBuf::from(dir).join("bin")),
            None => home.as_ref().map(|h| h.join(".cargo").join("bin")),
        };
        let local_bin = home.as_ref().map(|h| h.join(".local").join("bin"));
        resolve_install_channel(
            std::env::var("SSHUB_INSTALL_CHANNEL").ok(),
            std::env::current_exe().ok().as_deref(),
            cargo_bin.as_deref(),
            local_bin.as_deref(),
        )
    });
    CHANNEL.clone()
}

/// Pure resolution of the install channel from its inputs (extracted so it can
/// be unit tested without touching process-global env or the exe location).
fn resolve_install_channel(
    env_val: Option<String>,
    exe: Option<&std::path::Path>,
    cargo_bin: Option<&std::path::Path>,
    local_bin: Option<&std::path::Path>,
) -> Option<String> {
    if let Some(channel) = env_val.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return Some(channel.to_string());
    }
    let exe = exe?;
    if cargo_bin.is_some_and(|dir| exe.starts_with(dir)) {
        return Some("cargo".into());
    }
    // A build-target dir (`target/`, or a custom `…-target/` via CARGO_TARGET_DIR).
    if exe.components().any(|c| {
        let name = c.as_os_str();
        name == "target" || name.to_string_lossy().ends_with("-target")
    }) {
        return Some("source".into());
    }
    if local_bin.is_some_and(|dir| exe.starts_with(dir)) {
        return Some("source".into());
    }
    None
}

/// The release-tarball install: download, check, and swap the running binary.
fn replace_binary(tag: &str) -> Result<String> {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        _ => bail!("no prebuilt binary for this platform: use `cargo install sshub`"),
    };
    // Resolved, so a symlink on PATH keeps pointing at the replaced file.
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .context("cannot locate the running binary")?;
    let dir = exe
        .parent()
        .context("the running binary has no directory")?;
    sweep_stale_update_dirs(dir);
    // Next to the binary, so the final rename stays on one filesystem and the
    // swap is atomic. Removed on every return from here; a quit or a kill
    // mid-download leaves it for the sweep above on the next update.
    let tmp = tempfile::Builder::new()
        .prefix(UPDATE_DIR_PREFIX)
        .tempdir_in(dir)
        .with_context(|| format!("cannot write to {}", dir.display()))?;
    let archive = tmp.path().join("sshub.tar.gz");
    run(Command::new("curl")
        .args([
            "-fsSL",
            "--proto-redir",
            "=https",
            "--max-time",
            "300",
            "-o",
        ])
        .arg(&archive)
        .arg(format!(
            "https://github.com/{REPO}/releases/download/{tag}/sshub-{tag}-{target}.tar.gz"
        )))?;
    // Run as root, tar would otherwise give the file the release runner's uid.
    run(Command::new("tar")
        .args(["--no-same-owner", "-xzf"])
        .arg(&archive)
        .arg("-C")
        .arg(tmp.path())
        .arg("sshub"))?;
    let new = tmp.path().join("sshub");
    // Refuse a binary that does not start here or is not the announced version.
    let reported = run(Command::new(&new).arg("--version"))?;
    let expected = format!("sshub {}", tag.trim_start_matches('v'));
    if reported.trim() != expected {
        bail!(
            "the downloaded binary reports {:?}, expected {expected:?}",
            reported.trim()
        );
    }
    // Keep the old file's mode (umask would otherwise decide it).
    std::fs::set_permissions(&new, std::fs::metadata(&exe)?.permissions())?;
    std::fs::rename(&new, &exe).with_context(|| format!("cannot replace {}", exe.display()))?;
    Ok(reported)
}

const UPDATE_DIR_PREFIX: &str = ".sshub-update-";

/// Remove update directories an earlier run left in `dir`. A live one is never
/// older than the 300 s download limit (its mtime moves when the first byte of
/// the archive arrives and again when it is unpacked), so ten minutes means dead.
fn sweep_stale_update_dirs(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .file_name()
            .to_string_lossy()
            .starts_with(UPDATE_DIR_PREFIX)
            // Not followed: a symlink by that name is not ours to empty.
            && entry.metadata().is_ok_and(|m| {
                m.is_dir()
                    && m.modified()
                        .is_ok_and(|t| t.elapsed().is_ok_and(|age| age.as_secs() > 600))
            });
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Run `cmd` without a terminal and return its stdout. A failure names the
/// command and the stderr line that says why (see [`failure_line`]).
fn run(cmd: &mut Command) -> Result<String> {
    let shown = std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|s| s.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("cannot run `{shown}`"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("`{shown}` failed: {}", failure_line(&stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The stderr line that says why a tool failed. cargo: the first `error…`
/// line (later ones summarise it). npm: every line says `npm error`, and the
/// cause follows the `code` / `syscall` / `path` / `errno` fields. curl and
/// tar: the first line.
fn failure_line(stderr: &str) -> &str {
    const NPM_FIELDS: [&str; 4] = ["code ", "syscall ", "path ", "errno "];
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines
        .iter()
        .find(|l| l.starts_with("error"))
        .or_else(|| {
            lines.iter().find(|l| {
                l.strip_prefix("npm error ")
                    .is_some_and(|rest| !NPM_FIELDS.iter().any(|f| rest.starts_with(f)))
            })
        })
        .or(lines.first())
        .copied()
        .unwrap_or("no output")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn only_a_higher_plain_version_is_newer() {
        assert!(is_newer("v0.18.0", "0.17.6"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        // Numeric, not lexicographic: 10 > 9.
        assert!(is_newer("v0.9.10", "0.9.9"));
        assert!(!is_newer("v0.17.6", "0.17.6"));
        // A dev build ahead of the last release is not told to downgrade.
        assert!(!is_newer("v0.17.6", "0.18.0"));
        // Pre-releases and junk are never announced.
        assert!(!is_newer("v0.18.0-rc.1", "0.17.6"));
        assert!(!is_newer("v0.18", "0.17.6"));
        assert!(!is_newer("v0.18.0.1", "0.17.6"));
        assert!(!is_newer("latest", "0.17.6"));
    }

    #[test]
    fn failure_line_picks_the_cause() {
        // Real stderr, captured from npm 12.0.2, cargo 1.9x, GNU tar and curl
        // (paths shortened). A wrong pick shows the user a field name, a
        // progress line or a summary instead of the reason.
        let npm_eacces = "npm error code EACCES\nnpm error syscall mkdir\n\
            npm error path /usr/lib/node_modules\nnpm error errno -13\n\
            npm error Error: EACCES: permission denied, mkdir '/usr/lib/node_modules'\n\
            npm error     at async mkdir (node:internal/fs/promises:1564:10)\n\
            npm error A complete log of this run can be found in: /h/.npm/_logs/x.log\n";
        assert_eq!(
            failure_line(npm_eacces),
            "npm error Error: EACCES: permission denied, mkdir '/usr/lib/node_modules'"
        );
        let npm_etarget = "npm error code ETARGET\n\
            npm error notarget No matching version found for sshub-tui@99.0.0.\n\
            npm error notarget In most cases you or one of your dependencies are requesting a package version that doesn't exist.\n\
            npm error A complete log of this run can be found in: /h/.npm/_logs/x.log\n";
        assert_eq!(
            failure_line(npm_etarget),
            "npm error notarget No matching version found for sshub-tui@99.0.0."
        );
        let cargo_build = "    Updating crates.io index\n  Installing sshub v0.17.6\n\
            \x20  Compiling thiserror v1.0.69\n   Compiling quote v1.0.47\n\
            error: linker `/nonexistent/cc` not found\n  |\n\
            \x20 = note: No such file or directory (os error 2)\n\n\
            error: could not compile `quote` (build script) due to 1 previous error\n\
            error: failed to compile `sshub v0.17.6`, intermediate artifacts can be found at `/t`.\n";
        assert_eq!(
            failure_line(cargo_build),
            "error: linker `/nonexistent/cc` not found"
        );
        let tar = "tar: sshub: Not found in archive\n\
            tar: Exiting with failure status due to previous errors\n";
        assert_eq!(failure_line(tar), "tar: sshub: Not found in archive");
        let curl = "curl: (6) Could not resolve host: api.github.com\n";
        assert_eq!(
            failure_line(curl),
            "curl: (6) Could not resolve host: api.github.com"
        );
        assert_eq!(failure_line(""), "no output");
    }

    #[test]
    fn sweep_removes_only_dead_update_dirs() {
        // The sweep deletes inside the user's bin directory: anything but an
        // old update directory of ours must survive it.
        let bin = tempfile::tempdir().unwrap();
        let age = |path: &Path| {
            let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
            std::fs::File::open(path)
                .unwrap()
                .set_modified(hour_ago)
                .unwrap();
        };
        let dead = bin.path().join(".sshub-update-dead");
        std::fs::create_dir(&dead).unwrap();
        std::fs::write(dead.join("sshub.tar.gz"), "x").unwrap();
        age(&dead);
        let live = bin.path().join(".sshub-update-live");
        std::fs::create_dir(&live).unwrap();
        let other = bin.path().join("tools");
        std::fs::create_dir(&other).unwrap();
        age(&other);
        let file = bin.path().join(".sshub-update-note");
        std::fs::write(&file, "x").unwrap();
        age(&file);
        let link = bin.path().join(".sshub-update-link");
        std::os::unix::fs::symlink(&other, &link).unwrap();

        sweep_stale_update_dirs(bin.path());

        assert!(!dead.exists(), "an hour-old update dir is dead");
        assert!(live.exists(), "a fresh one may be another sshub's download");
        assert!(other.exists() && file.exists());
        assert!(link.symlink_metadata().is_ok(), "a symlink is not followed");
    }

    #[test]
    fn install_channel_resolution() {
        let ch = |env_val: Option<&str>,
                  exe: Option<&Path>,
                  cargo_bin: Option<&Path>,
                  local_bin: Option<&Path>| {
            resolve_install_channel(env_val.map(str::to_string), exe, cargo_bin, local_bin)
        };

        // The env var wins outright — the npm shim sets it because the prebuilt
        // npm binary is byte-identical to the release-tarball one.
        assert_eq!(ch(Some("npm"), None, None, None).as_deref(), Some("npm"));
        // A blank env value falls through to the path heuristics.
        assert_eq!(
            ch(Some("   "), Some(Path::new("/usr/bin/sshub")), None, None),
            None
        );

        // cargo install → $CARGO_HOME/bin (or the default ~/.cargo/bin).
        assert_eq!(
            ch(
                None,
                Some(Path::new("/home/u/.cargo/bin/sshub")),
                Some(Path::new("/home/u/.cargo/bin")),
                None
            )
            .as_deref(),
            Some("cargo")
        );

        // A dev run straight out of a build dir — plain `target/` and a custom
        // `…-target/` (CARGO_TARGET_DIR) both count.
        assert_eq!(
            ch(
                None,
                Some(Path::new("/repo/target/release/sshub")),
                None,
                None
            )
            .as_deref(),
            Some("source")
        );
        assert_eq!(
            ch(
                None,
                Some(Path::new("/repo/.cargo-target/debug/sshub")),
                None,
                None
            )
            .as_deref(),
            Some("source")
        );

        // `just install` destination.
        assert_eq!(
            ch(
                None,
                Some(Path::new("/home/u/.local/bin/sshub")),
                None,
                Some(Path::new("/home/u/.local/bin"))
            )
            .as_deref(),
            Some("source")
        );

        // A distro package or manual copy — no channel, no suffix.
        assert_eq!(
            ch(None, Some(Path::new("/usr/bin/sshub")), None, None),
            None
        );
        // No exe path resolvable at all.
        assert_eq!(ch(None, None, None, None), None);
    }
}
