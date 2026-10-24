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
    /// After a successful install: the release notes of the versions it
    /// brings, `None` when GitHub did not serve them.
    Notes(Option<String>),
}

/// Ask GitHub for the latest release on a worker thread.
pub fn spawn_check() -> Receiver<UpdateEvent> {
    spawn(|| {
        UpdateEvent::Checked(
            latest_tag().map(|tag| is_newer(&tag, env!("CARGO_PKG_VERSION")).then_some(tag)),
        )
    })
}

/// Install release `tag` on a worker thread. The notes follow as a second
/// event, so the install does not look unfinished while they download.
pub fn spawn_install(tag: String) -> Receiver<UpdateEvent> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = install(&tag);
        let installed = result.is_ok();
        if tx.send(UpdateEvent::Installed(result)).is_ok() && installed {
            let _ = tx.send(UpdateEvent::Notes(fetch_notes(&tag)));
        }
    });
    rx
}

/// The changelog this binary was built from.
const CHANGELOG: &str = include_str!("../CHANGELOG.md");

/// The running version's own changelog section.
pub fn running_notes() -> Option<String> {
    let current = parse_version(env!("CARGO_PKG_VERSION"));
    release_notes(CHANGELOG, |v| Some(v) == current)
}

/// Every changelog section between the running version and `tag`, read from
/// the CHANGELOG.md of that tag. The install already happened: a failed fetch
/// only costs the notes.
fn fetch_notes(tag: &str) -> Option<String> {
    let current = parse_version(env!("CARGO_PKG_VERSION"))?;
    let target = parse_version(tag)?;
    let changelog = run(Command::new("curl")
        // The whole changelog (~90 KB), not a small JSON: more time than the
        // check gets. The install is done, so only the notes wait on it.
        .args(["-fsSL", "--proto-redir", "=https", "--max-time", "30", "--max-filesize", "2000000"])
        .arg(format!(
            "https://raw.githubusercontent.com/{REPO}/{tag}/CHANGELOG.md"
        )))
    .ok()?;
    release_notes(&changelog, |v| v > current && v <= target)
}

/// The `## [x.y.z]` sections of a Keep-a-Changelog file whose version `keep`
/// accepts, in file order (newest first). `[Unreleased]` never parses, so it
/// is never kept. The file is hard-wrapped at 80 columns, which a narrower
/// popup would break again mid-line: an indented continuation line joins the
/// bullet above it, and the `**` bold markers go.
fn release_notes(changelog: &str, keep: impl Fn((u64, u64, u64)) -> bool) -> Option<String> {
    let mut out = String::new();
    let mut keeping = false;
    for line in changelog.lines() {
        if let Some(header) = line.strip_prefix("## [") {
            keeping = header
                .split_once(']')
                .and_then(|(version, _)| parse_version(version))
                .is_some_and(&keep);
        }
        if !keeping {
            continue;
        }
        let text = line.trim_start();
        let continues = !text.is_empty()
            && text.len() < line.len()
            && !text.starts_with("- ")
            && out.ends_with('\n')
            && !out.ends_with("\n\n");
        if continues {
            out.pop();
            out.push(' ');
            out.push_str(&text.replace("**", ""));
        } else {
            out.push_str(&line.replace("**", ""));
        }
        out.push('\n');
    }
    let out = out.trim();
    (!out.is_empty()).then(|| out.to_string())
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

/// A plain `MAJOR.MINOR.PATCH`, with or without a leading `v`.
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut parts = s.trim().trim_start_matches('v').split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// Whether release `tag` (`v1.2.3`) is newer than version `current` (`1.2.2`).
/// Anything that is not plain `MAJOR.MINOR.PATCH` (a pre-release, a typo) is
/// never announced.
fn is_newer(tag: &str, current: &str) -> bool {
    matches!((parse_version(tag), parse_version(current)), (Some(latest), Some(current)) if latest > current)
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
        Some("npm") => {
            let mut npm = global("npm");
            npm.args(["install", "-g"]);
            // The npm on PATH, and the prefix it installs into (npm takes it
            // from the `node` that runs it), may belong to another Node than
            // the one holding this copy: a desktop launcher starting the nvm
            // copy with only the system Node on PATH updates the system prefix
            // or fails on its permissions. Install into this copy's prefix,
            // with that prefix's own npm and node when it has them.
            if let Some(prefix) = npm_prefix(&exe) {
                // An unset PATH adds nothing (an empty entry would mean the
                // current directory); a prefix PATH cannot hold leaves it be.
                let path = std::env::var_os("PATH");
                let path = std::iter::once(prefix.join("bin"))
                    .chain(path.iter().flat_map(std::env::split_paths));
                if let Ok(path) = std::env::join_paths(path) {
                    npm.env("PATH", path);
                }
                npm.arg("--prefix").arg(prefix);
            }
            run(npm.arg(format!("sshub-tui@{version}")))
        }
        Some("cargo") => run(global("cargo").args(["install", "sshub", "--version", version])),
        None => replace_binary(tag),
        Some("source") => bail!("built from source: update the checkout and run `just install`"),
        Some(other) => bail!("installed via {other}: update it there"),
    }
    .map(drop)
}

/// The global npm prefix a copy at `exe` was installed under: npm puts a global
/// package in `<prefix>/lib/node_modules/<name>`, and its dependencies (the
/// platform package carrying this binary) below that.
fn npm_prefix(exe: &std::path::Path) -> Option<&std::path::Path> {
    exe.ancestors()
        .find(|dir| dir.ends_with("lib/node_modules/sshub-tui"))?
        .ancestors()
        .nth(3)
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
/// line (later ones summarise it); Node's `Error: …` / `Error [ERR_…]: …` and
/// npm's `ERROR: npm … is known not to run on Node.js …` start the same way,
/// case aside. npm: every line says `npm error`, and the cause follows the
/// `code` / `syscall` / `path` / `errno` fields. A Node crash with another
/// error class: its `TypeError: …` (and the like) line under the source
/// excerpt. curl and tar: the first line.
fn failure_line(stderr: &str) -> &str {
    const NPM_FIELDS: [&str; 4] = ["code ", "syscall ", "path ", "errno "];
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines
        .iter()
        .find(|l| l.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("error")))
        .or_else(|| {
            lines.iter().find(|l| {
                l.strip_prefix("npm error ")
                    .is_some_and(|rest| !NPM_FIELDS.iter().any(|f| rest.starts_with(f)))
            })
        })
        .or_else(|| {
            lines.iter().find(|l| {
                l.split_once(": ").is_some_and(|(name, _)| {
                    let name = name.split_once(" [").map_or(name, |(class, _)| class);
                    name.ends_with("Error") && name.chars().all(|c| c.is_ascii_alphanumeric())
                })
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
    fn release_notes_take_the_versions_in_range() {
        let log = "# Changelog\n\n## [Unreleased]\n\n- next\n\n\
            ## [0.3.0] - 2026-10-03\n\n- three\n\n\
            ## [0.2.1] - 2026-10-02\n\n- two-one\n\n\
            ## [0.2.0] - 2026-10-01\n\n- two\n";
        // An update from 0.2.0 to 0.3.0 brings 0.2.1 and 0.3.0, newest first.
        let notes = release_notes(log, |v| v > (0, 2, 0) && v <= (0, 3, 0)).unwrap();
        assert_eq!(
            notes,
            "## [0.3.0] - 2026-10-03\n\n- three\n\n## [0.2.1] - 2026-10-02\n\n- two-one"
        );
        assert_eq!(
            release_notes(log, |v| v == (0, 2, 0)).as_deref(),
            Some("## [0.2.0] - 2026-10-01\n\n- two")
        );
        // Unreleased is never a version; nothing in range is no notes.
        assert_eq!(release_notes(log, |_| false), None);
        assert_eq!(release_notes("## [Unreleased]\n- x\n", |_| true), None);
    }

    #[test]
    fn release_notes_unwrap_bullets_for_the_popup_to_wrap() {
        let log = "## [0.2.0] - 2026-10-01\n\n### Added\n\n\
            - **Thing** - a long entry\n  wrapped over\n  three lines.\n  \n\
            - Second, with a list:\n  - nested item\n    wrapped too\n\n\
            ### Fixed\n\n- one\n";
        assert_eq!(
            release_notes(log, |_| true).unwrap(),
            "## [0.2.0] - 2026-10-01\n\n### Added\n\n\
             - Thing - a long entry wrapped over three lines.\n  \n\
             - Second, with a list:\n  - nested item wrapped too\n\n\
             ### Fixed\n\n- one"
        );
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
        // npm 12.1.0 with a dependency missing from its install: Node crashes
        // before npm prints a single `npm error` line.
        let node_crash = "/usr/lib/node_modules/npm/lib/cli/validate-engines.js:25\n\
            \x20   throw err\n    ^\n\n\
            Error: Cannot find module 'semver/functions/satisfies'\n\
            Require stack:\n- /usr/lib/node_modules/npm/lib/cli/entry.js\n\
            \x20   at Module._resolveFilename (node:internal/modules/cjs/loader:1564:15) {\n\
            \x20 code: 'MODULE_NOT_FOUND',\n}\n\nNode.js v24.20.0\n";
        assert_eq!(
            failure_line(node_crash),
            "Error: Cannot find module 'semver/functions/satisfies'"
        );
        // npm under a Node it cannot parse, and Node errors carrying a code
        // (Node 24; paths shortened).
        let npm_too_old_node = "ERROR: npm v12.1.0 is known not to run on Node.js v14.21.3.  \
            This version of npm supports the following node versions: `^22.22.2`.\n\n\
            ERROR:\n/n/lib/cli.js:3\n\n\nSyntaxError: Unexpected end of input\n";
        assert_eq!(
            failure_line(npm_too_old_node),
            "ERROR: npm v12.1.0 is known not to run on Node.js v14.21.3.  \
            This version of npm supports the following node versions: `^22.22.2`."
        );
        let err_code = "node:internal/modules/cjs/loader:664\n      throw e;\n      ^\n\n\
            Error [ERR_PACKAGE_PATH_NOT_EXPORTED]: Package subpath './dist/x.js' is not defined by \"exports\"\n\
            \x20   at exportsNotFound (node:internal/modules/esm/resolve:313:10)\n";
        assert_eq!(
            failure_line(err_code),
            "Error [ERR_PACKAGE_PATH_NOT_EXPORTED]: Package subpath './dist/x.js' is not defined by \"exports\""
        );
        let type_error =
            "node:fs:440\n    return binding.readFileUtf8(path, stringToFlags(options.flag));\n\
            \x20                  ^\n\n\
            TypeError [ERR_INVALID_ARG_TYPE]: The \"path\" argument must be of type string\n\
            \x20   at Object.readFileSync (node:fs:440:20) {\n  code: 'ERR_INVALID_ARG_TYPE'\n}\n";
        assert_eq!(
            failure_line(type_error),
            "TypeError [ERR_INVALID_ARG_TYPE]: The \"path\" argument must be of type string"
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
    fn npm_prefix_is_where_npm_installs_globally() {
        // npm, not this file, says where its global packages go. The platform
        // package nests in sshub-tui's own node_modules, as on a real install
        // (`<root>/sshub-tui/node_modules/sshub-linux-x64/bin/sshub`).
        let ask = |what: &str| {
            Command::new("npm")
                .args([what, "-g"])
                .current_dir(std::env::temp_dir())
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        };
        let (Some(root), Some(prefix)) = (ask("root"), ask("prefix")) else {
            eprintln!("skipping: no working npm");
            return;
        };
        let exe = Path::new(&root).join("sshub-tui/node_modules/sshub-linux-x64/bin/sshub");
        assert_eq!(npm_prefix(&exe), Some(Path::new(&prefix)));
        assert_eq!(npm_prefix(Path::new("/home/u/.cargo/bin/sshub")), None);
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
