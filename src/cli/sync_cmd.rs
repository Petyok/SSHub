//! The headless host-sync commands: `sync-serve`, `sync-enroll`, `sync-join`,
//! `sync-now`, `sync-status`.
//!
//! Dispatched from `main.rs` before `CliContext::bootstrap_with`: they open the
//! launcher store and the device identity themselves. `sync-serve` and
//! `sync-enroll` run on the far side of an ssh connection, so stdout there
//! carries protocol frames and nothing else.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::json;

use crate::profile::ProfilePaths;
use crate::store::LauncherStore;
use crate::sync::client::TransportError;
use crate::sync::crypto::DeviceIdentity;
use crate::sync::enroll::{run_sync_enroll, EnrollProgress, EnrollRole, SasConfirm, SasPrompt};
use crate::sync::identity_store::load_or_init_identity;
use crate::sync::oplog::VerifiedBy;
use crate::sync::protocol::RoundSummary;
use crate::sync::runtime::{device_name, enroll_limits, Peers, SyncRuntime};
use crate::sync::transport::{Conn, Limits};

use super::help;

const COMMANDS: [&str; 6] = [
    "sync-serve",
    "sync-enroll",
    "sync-join",
    "sync-now",
    "sync-status",
    "sync-review",
];

/// Appended to every device sync-now cannot reach for want of a host.
const HOW_TO_RECORD_A_HOST: &str =
    "press t on its host in the hosts tab, or run `sshub sync-join <host>` (a repeat ceremony records it)";

const EMPTY_RING: &str = "No sync ring on this profile yet.";

pub fn is_sync_command(cmd: &str) -> bool {
    COMMANDS.contains(&cmd)
}

/// Run one of the sync commands; the profile is resolved only after the
/// arguments (and `--help`) are accepted. Returns the process exit code.
pub fn run(
    cmd: &str,
    args: &[String],
    resolve: impl FnOnce() -> Result<ProfilePaths>,
) -> Result<i32> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        help::print_command_help(cmd);
        return Ok(0);
    }
    let parsed = match parse_args(cmd, args) {
        Ok(p) => p,
        Err(msg) => return Ok(usage_error(cmd, &msg)),
    };
    let paths = match resolve() {
        Ok(p) => p,
        Err(e) => return Ok(failure(&format!("{e:#}"))),
    };
    let result = match cmd {
        "sync-serve" => serve(&paths),
        "sync-enroll" => enroll(&paths),
        "sync-join" => join(&paths, &parsed.positionals[0]),
        "sync-now" => now(
            &paths,
            parsed.positionals.first().map(String::as_str),
            parsed.json,
        ),
        "sync-status" => status(&paths, parsed.json),
        "sync-review" => review(&paths, parsed.json, parsed.resolve.as_ref()),
        other => anyhow::bail!("unknown sync command: {other}"),
    };
    Ok(match result {
        Ok(code) => code,
        Err(e) => failure(&format!("{cmd}: {e:#}")),
    })
}

fn usage_error(cmd: &str, msg: &str) -> i32 {
    eprintln!("sshub: {msg}");
    eprintln!("       run `sshub {cmd} --help` for usage");
    2
}

fn failure(msg: &str) -> i32 {
    eprintln!("sshub: {msg}");
    1
}

#[derive(Debug, PartialEq, Eq)]
struct Parsed {
    positionals: Vec<String>,
    json: bool,
    /// `sync-review`: (approve, id prefix).
    resolve: Option<(bool, String)>,
}

/// Arguments after the (already extracted) global profile flags.
fn parse_args(cmd: &str, args: &[String]) -> Result<Parsed, String> {
    let takes_format = matches!(cmd, "sync-now" | "sync-status" | "sync-review");
    let mut out = Parsed {
        positionals: Vec::new(),
        json: false,
        resolve: None,
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if takes_format && a == "--format" {
            out.json = match args.get(i + 1).map(String::as_str) {
                Some("plain") => false,
                Some("json") => true,
                Some(other) => return Err(format!("unknown format '{other}'")),
                None => return Err("--format requires a value".into()),
            };
            i += 2;
        } else if cmd == "sync-review" && (a == "--approve" || a == "--reject") {
            let id = args
                .get(i + 1)
                .filter(|v| !v.starts_with('-'))
                .ok_or_else(|| format!("{a} requires an op id"))?;
            if out.resolve.is_some() {
                return Err("give exactly one of --approve and --reject".into());
            }
            out.resolve = Some((a == "--approve", id.clone()));
            i += 2;
        } else if a.starts_with('-') {
            return Err(format!("unknown option '{a}' for {cmd}"));
        } else {
            out.positionals.push(a.to_string());
            i += 1;
        }
    }
    let max = match cmd {
        "sync-join" => 1,
        "sync-now" => 1,
        _ => 0,
    };
    if out.positionals.len() > max {
        return Err(format!("{cmd} takes {max} argument(s)"));
    }
    if cmd == "sync-join" && out.positionals.is_empty() {
        return Err("sync-join needs a host (a launcher host name or [user@]host[:port])".into());
    }
    Ok(out)
}

fn open_store(paths: &ProfilePaths) -> Result<LauncherStore> {
    LauncherStore::open(paths.launcher_db()).context("open the launcher database")
}

fn open_identity(paths: &ProfilePaths) -> Result<DeviceIdentity> {
    let secrets = crate::credentials::open_for_profile(paths);
    load_or_init_identity(paths, secrets.as_ref())
}

/// Keep the real stdout for protocol frames and point fd 1 at stderr, so a
/// stray print anywhere in the process can never corrupt the stream.
fn take_protocol_stdout() -> Result<File> {
    use std::os::fd::FromRawFd;
    // SAFETY: dup/dup2 on the process's own standard descriptors; the new fd
    // is owned exclusively by the returned File.
    unsafe {
        let fd = libc::dup(1);
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context("dup stdout");
        }
        if libc::dup2(2, 1) < 0 {
            let err = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(err).context("redirect stdout to stderr");
        }
        Ok(File::from_raw_fd(fd))
    }
}

fn serve(paths: &ProfilePaths) -> Result<i32> {
    let out = take_protocol_stdout()?;
    let store = open_store(paths)?;
    let identity = open_identity(paths)?;
    let mut conn = Conn::new(
        std::io::stdin(),
        out,
        Limits::default(),
        Arc::new(AtomicBool::new(false)),
    );
    // A broken config must not stop a peer from syncing: serve without mail.
    let config = crate::config::load_config_at(&paths.config_file)
        .map_err(|e| eprintln!("sshub: config not read, no notifications: {e:#}"))
        .ok();
    let notify = config.as_ref().map(|c| &c.sync.notify);
    let round = crate::sync::hub::run_sync_serve_notify(&store, &identity, &mut conn, notify);
    mail_owed_codes(&store, &identity, notify);
    round?;
    Ok(0)
}

/// The emailed third-factor code, when this device is the registered sender and
/// a Red change has everything but that factor. Failures go to stderr only:
/// stdout carries frames.
fn mail_owed_codes(
    store: &LauncherStore,
    identity: &DeviceIdentity,
    notify: Option<&crate::config::NotifyConfig>,
) {
    let Some(cfg) = notify.filter(|c| c.command.is_some()) else {
        return;
    };
    let now = crate::store::now_ts();
    let owed = match crate::sync::ceremony::owed_code_mails(store, identity, now) {
        Ok(o) => o,
        Err(e) => return eprintln!("sshub: could not look for owed code mails: {e:#}"),
    };
    for (hash, subject, body) in owed {
        match crate::sync::notify::send_notification(cfg, &subject, &body) {
            Ok(()) => {
                if let Err(e) = crate::sync::ceremony::mark_code_sent(store, &hash) {
                    eprintln!("sshub: code mailed but not recorded: {e:#}");
                }
            }
            Err(e) => eprintln!("sshub: could not mail the code: {e:#}"),
        }
    }
}

/// A string a ring peer wrote, safe to print: control characters shown escaped.
fn plain(text: &str) -> String {
    super::theme::sanitize_foreign(text)
}

fn is_yes(line: &str) -> bool {
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}

/// Said wherever the words cannot be compared with a second screen.
const NO_SCREEN_NOTE: &str = "The other device has no screen: these words prove nothing. Its identity rests on the SSH host key you verified for";

/// The question for one SAS prompt, shown on whichever terminal asks.
fn sas_question(p: &SasPrompt<'_>, host: Option<&str>) -> String {
    if p.peer_has_screen {
        format!(
            "Enrolling {}. Compare these words with the other device: {}\nDo they match? [y/N] ",
            p.peer_name, p.sas
        )
    } else {
        format!(
            "Enrolling {}. SAS words: {}\n{NO_SCREEN_NOTE} {}.\nAdmit this device? [y/N] ",
            p.peer_name,
            p.sas,
            host.unwrap_or("this connection")
        )
    }
}

/// `sync-enroll` on a terminal (`ssh -t`): ask on /dev/tty.
struct TtyPrompt;

impl SasConfirm for TtyPrompt {
    fn confirm(&mut self, p: &SasPrompt<'_>) -> bool {
        let tty = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .and_then(|w| w.try_clone().map(|r| (w, r)));
        let Ok((mut w, r)) = tty else {
            return false;
        };
        let _ = write!(w, "{}", sas_question(p, None));
        let _ = w.flush();
        let mut line = String::new();
        BufReader::new(r).read_line(&mut line).is_ok() && is_yes(&line)
    }
}

fn stdin_is_tty() -> bool {
    // SAFETY: isatty only inspects descriptor 0.
    unsafe { libc::isatty(0) == 1 }
}

fn enroll(paths: &ProfilePaths) -> Result<i32> {
    // Decided before the protocol takes over stdin.
    let interactive = stdin_is_tty();
    let out = take_protocol_stdout()?;
    let store = open_store(paths)?;
    let identity = open_identity(paths)?;
    let mut conn = Conn::new(
        std::io::stdin(),
        out,
        enroll_limits(),
        Arc::new(AtomicBool::new(false)),
    );
    // Without a tty there is no human here: never confirm as one. The words go
    // to stderr for the log only.
    let mut logged = |p: &SasPrompt<'_>| {
        eprintln!(
            "SAS words: {} (peer has screen: {}) (peer {})",
            p.sas,
            if p.peer_has_screen { "yes" } else { "no" },
            p.peer_name
        );
        TtyPrompt.confirm(p)
    };
    let ui: Option<&mut dyn SasConfirm> = if interactive { Some(&mut logged) } else { None };
    let outcome = run_sync_enroll(&mut conn, &store, &identity, &device_name(), true, ui)?;
    if !interactive {
        // No human here, so nothing says whether the peer had a screen: this
        // line is for the log only.
        eprintln!(
            "SAS words: {} (peer has screen: unknown, no tty here; admitted on the SSH session)",
            outcome.sas
        );
    }
    Ok(0)
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn runtime(paths: &ProfilePaths) -> Result<SyncRuntime> {
    let store = open_store(paths)?;
    let identity = open_identity(paths)?;
    Ok(SyncRuntime::new(paths.clone(), Arc::new(store), identity))
}

/// `sync-join`'s prompt: the words on `out`, the anchor's progress on `err`, the
/// answer from `input` (the terminal's stdin, stdout and stderr in the CLI).
struct JoinPrompt<'a, I, O, E> {
    host: &'a str,
    input: I,
    out: O,
    err: E,
}

impl<I: BufRead, O: Write, E: Write> SasConfirm for JoinPrompt<'_, I, O, E> {
    fn confirm(&mut self, p: &SasPrompt<'_>) -> bool {
        let out = &mut self.out;
        if p.peer_has_screen {
            let _ = writeln!(out, "Compare these words with the other device: {}", p.sas);
            let _ = writeln!(out, "Peer: {}", p.peer_name);
            let _ = write!(out, "Do they match? [y/N] ");
        } else {
            let _ = writeln!(out, "SAS words: {}", p.sas);
            let _ = writeln!(out, "Peer: {}", p.peer_name);
            let _ = writeln!(out, "{NO_SCREEN_NOTE} {}.", self.host);
            let _ = write!(out, "Admit this device? [y/N] ");
        }
        let _ = out.flush();
        let mut line = String::new();
        self.input.read_line(&mut line).is_ok() && is_yes(&line)
    }

    fn progress(&mut self, p: EnrollProgress) {
        let _ = writeln!(self.err, "{}", crate::sync::runtime::progress_line(p));
    }
}

fn join(paths: &ProfilePaths, host: &str) -> Result<i32> {
    let rt = runtime(paths)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let mut ui = JoinPrompt {
        host,
        input: std::io::stdin().lock(),
        out: std::io::stdout(),
        err: std::io::stderr(),
    };
    match rt.join(host, &mut ui, &cancel) {
        Ok(o) => {
            let how = match o.verified_by {
                VerifiedBy::Sas => "verified by SAS",
                VerifiedBy::SshHostKey => "verified by the SSH host key only",
            };
            let id = o.peer.device_id();
            match o.role {
                EnrollRole::Joiner => println!(
                    "Joined the ring of {} (device {}), {how}. This device is a mirror until the ring approves more.",
                    o.peer_name,
                    short(&id)
                ),
                EnrollRole::Inviter => println!(
                    "Admitted {} (device {}) to this ring as a mirror, {how}.",
                    o.peer_name,
                    short(&id)
                ),
            }
            Ok(0)
        }
        Err(e) => Ok(failure(&format!("sync-join: {e}"))),
    }
}

/// Pick one ring device by exact id, exact name, or unique id prefix.
/// `devices` is `(id, name)`.
fn resolve_device<'a>(devices: &'a [(String, String)], query: &str) -> Result<&'a str, String> {
    let pick = |hits: Vec<&'a (String, String)>| match hits.as_slice() {
        [] => None,
        [one] => Some(Ok(one.0.as_str())),
        many => Some(Err(format!(
            "'{query}' is ambiguous: {}",
            many.iter()
                .map(|d| format!("{} ({})", d.1, short(&d.0)))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    };
    let by_id = devices.iter().filter(|d| d.0 == query).collect();
    let by_name = devices.iter().filter(|d| d.1 == query).collect();
    let by_prefix = devices
        .iter()
        .filter(|d| !query.is_empty() && d.0.starts_with(query))
        .collect();
    for hits in [by_id, by_name, by_prefix] {
        if let Some(r) = pick(hits) {
            return r;
        }
    }
    Err(format!("no ring device matches '{query}'"))
}

enum Outcome {
    Ok(Box<RoundSummary>),
    Skipped,
    /// Announced by the ring, waiting for this machine's approval.
    Admitting,
    Failed(String),
}

fn reason(e: &TransportError) -> &str {
    match e {
        TransportError::Offline(r) | TransportError::Failed(r) => r,
    }
}

fn ids(v: &[String]) -> String {
    v.iter().map(|d| short(d)).collect::<Vec<_>>().join(",")
}

fn now(paths: &ProfilePaths, device: Option<&str>, json_out: bool) -> Result<i32> {
    let store = open_store(paths)?;
    let rows = store.sync_devices()?;
    if rows.is_empty() {
        if json_out {
            println!("[]");
        } else {
            println!("{EMPTY_RING} Nothing to sync.");
        }
        return Ok(0);
    }
    // Devices the ring announced that this machine has not admitted yet.
    let admitting: Vec<(String, String)> = store
        .sync_pending()?
        .iter()
        .filter_map(|op| match &op.body {
            crate::sync::oplog::OpBody::DeviceAdd { device, name, .. } => {
                Some((device.device_id(), name.clone()))
            }
            _ => None,
        })
        .filter(|(id, _)| !rows.iter().any(|r| r.id == *id))
        .collect();
    drop(store);
    let rt = runtime(paths)?;
    let me = rt.identity.public().device_id();
    let peers = Peers::load(paths)?;
    let pairs: Vec<(String, String)> = rows
        .iter()
        .filter(|r| r.id != me)
        .map(|r| (r.id.clone(), r.name.clone()))
        .collect();
    let targets: Vec<(String, String)> = match device {
        Some(q) => match resolve_device(&pairs, q) {
            Ok(id) => pairs.iter().filter(|p| p.0 == id).cloned().collect(),
            Err(msg) => return Ok(usage_error("sync-now", &msg)),
        },
        None => pairs,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let mut results = Vec::new();
    for (id, name) in targets {
        let outcome = if device.is_none() && peers.host_of(&id).is_none() {
            Outcome::Skipped
        } else {
            match rt.sync_with(&id, &cancel) {
                Ok(s) => Outcome::Ok(Box::new(s)),
                Err(e) => Outcome::Failed(reason(&e).to_string()),
            }
        };
        results.push((id, name, outcome));
    }
    if device.is_none() {
        for (id, name) in admitting {
            results.push((id, name, Outcome::Admitting));
        }
    }

    let mut fork = false;
    let mut failed = false;
    let mut attempted = false;
    let mut array = Vec::new();
    for (id, name, outcome) in &results {
        let shown = plain(name);
        match outcome {
            Outcome::Skipped => {
                if !json_out {
                    println!("{shown}: skipped (no host known); {HOW_TO_RECORD_A_HOST}");
                }
                array.push(json!({
                    "device": id, "name": name, "status": "skipped",
                    "reason": format!("no host known; {HOW_TO_RECORD_A_HOST}"),
                }));
            }
            Outcome::Admitting => {
                if !json_out {
                    println!("{shown}: not admitted yet (waiting for approval: sshub sync-review)");
                }
                array.push(json!({
                    "device": id, "name": name, "status": "pending_admission",
                    "reason": "waiting for approval: sshub sync-review",
                }));
            }
            Outcome::Failed(r) => {
                attempted = true;
                failed = true;
                if !json_out {
                    println!("{shown}: offline/failed: {r}");
                }
                array.push(json!({"device": id, "name": name, "status": "failed", "error": r}));
            }
            Outcome::Ok(s) => {
                attempted = true;
                if !s.forks.is_empty() {
                    fork = true;
                    if !json_out {
                        println!("FORK DETECTED: {shown}");
                    }
                }
                if !json_out {
                    let mut line = format!(
                        "{shown}: ok applied {}, pending {}, received {}, sent {}",
                        s.applied, s.pending_arrived, s.received, s.sent
                    );
                    if !s.behind.is_empty() {
                        line.push_str(&format!(", behind: {}", ids(&s.behind)));
                    }
                    if !s.suspect.is_empty() {
                        line.push_str(&format!(", suspect: {}", ids(&s.suspect)));
                    }
                    println!("{line}");
                }
                array.push(json!({
                    "device": id, "name": name,
                    "status": if s.forks.is_empty() { "ok" } else { "fork" },
                    "applied": s.applied, "pending": s.pending_arrived,
                    "received": s.received, "sent": s.sent,
                    "rejected": s.rejected, "duplicates": s.duplicates,
                    "forks": s.forks, "behind": s.behind, "suspect": s.suspect,
                }));
            }
        }
    }
    if json_out {
        println!("{}", serde_json::to_string_pretty(&array)?);
    } else if !attempted {
        println!("Nothing synced: no ring device with a known host.");
    }
    Ok(if fork {
        3
    } else if failed {
        1
    } else {
        0
    })
}

fn age(last_seen: i64) -> String {
    if last_seen <= 0 {
        return "never".into();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let s = (now - last_seen).max(0);
    match s {
        0..=59 => format!("{s}s ago"),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}

fn lower<T: std::fmt::Debug>(v: T) -> String {
    format!("{v:?}").to_lowercase()
}

/// Read-only: never creates the device key.
fn verified_str(v: VerifiedBy) -> &'static str {
    match v {
        VerifiedBy::Sas => "sas",
        VerifiedBy::SshHostKey => "ssh_host_key",
    }
}

fn status(paths: &ProfilePaths, json_out: bool) -> Result<i32> {
    let store = open_store(paths)?;
    let rows = crate::sync::ceremony::effective_devices(&store, None)?;
    let pending = store.sync_pending_with_reasons()?.len();
    let mode = lower(crate::sync::ceremony::effective_cred_mode(&store)?);
    let peers = Peers::load(paths)?;
    if json_out {
        let devices: Vec<_> = rows
            .iter()
            .map(|r| {
                json!({
                    "id": r.id, "name": r.name, "role": lower(r.role),
                    "status": lower(r.status), "last_seen": r.last_seen,
                    "head_seq": r.head_seq, "host": peers.host_of(&r.id),
                    "verified_by": verified_str(r.verified_by),
                })
            })
            .collect();
        let doc = json!({
            "devices": devices, "pending": pending, "credential_mode": mode,
            "companion_fingerprint": crate::sync::companion::COMPANION_FINGERPRINT,
        });
        println!("{}", serde_json::to_string_pretty(&doc)?);
        return Ok(0);
    }
    if rows.is_empty() && pending == 0 {
        println!("{EMPTY_RING}");
        return Ok(0);
    }
    for r in &rows {
        println!(
            "{} ({}) role {} status {} last seen {} head {} host {} verified {}",
            plain(&r.name),
            short(&r.id),
            lower(r.role),
            lower(r.status),
            age(r.last_seen),
            r.head_seq,
            peers.host_of(&r.id).unwrap_or("-"),
            verified_str(r.verified_by),
        );
    }
    println!("Pending changes: {pending}");
    println!("Credential mode: {mode}");
    println!(
        "Phone page of this build: {} ({})",
        crate::sync::companion::COMPANION_FINGERPRINT,
        crate::sync::companion::COMPARE_HINT
    );
    Ok(0)
}

/// Approve the pending `ShareRelease` `hash` as this holder: refused when stale (the requester's
/// wait is over), otherwise the grants for the shares this device holds are written and the
/// request is resolved. The one function behind `sync-review --approve` on a release.
pub(crate) fn approve_release(
    store: &LauncherStore,
    identity: &DeviceIdentity,
    hash: &[u8; 32],
    now: i64,
) -> Result<()> {
    let op = store
        .sync_pending()?
        .into_iter()
        .find(|o| o.hash() == *hash)
        .context("that request is no longer waiting")?;
    anyhow::ensure!(
        matches!(op.body, crate::sync::oplog::OpBody::ShareRelease { .. }),
        "that change is not a share release"
    );
    release_fresh(&op, now)?;
    // The grants are built from an approved request: resolve first, as the TUI does.
    crate::sync::ceremony::resolve(store, hash, true, now)?;
    for body in crate::sync::quorum::grant_bodies(store, identity, &op)? {
        store.sync_append_local(identity, body)?;
    }
    Ok(())
}

/// A share release is answered only while its requester can still be waiting.
fn release_fresh(op: &crate::sync::oplog::Op, now: i64) -> Result<()> {
    let age = now - op.timestamp;
    anyhow::ensure!(
        age.abs() <= crate::sync::limits::CHALLENGE_EXPIRY_SECS,
        "this request is {age} s old: too old to answer, the requester has to ask again"
    );
    Ok(())
}

/// The 12-hex-digit id `sync-review` shows for a pending op.
pub(crate) fn op_id(hash: &[u8; 32]) -> String {
    hex(hash)[..12].to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The one full hash among `hashes` that starts with `prefix` (hex, at least
/// four characters).
fn resolve_prefix(hashes: &[[u8; 32]], prefix: &str) -> Result<[u8; 32], String> {
    let p = prefix.to_lowercase();
    if p.len() < 4 || !p.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "'{prefix}' is not an op id (at least 4 hex characters)"
        ));
    }
    let hits: Vec<&[u8; 32]> = hashes.iter().filter(|h| hex(*h).starts_with(&p)).collect();
    match hits.as_slice() {
        [] => Err(format!("no pending op matches '{prefix}'")),
        [one] => Ok(**one),
        _ => Err(format!(
            "'{prefix}' matches several pending ops; use more characters"
        )),
    }
}

/// Review pending changes. Read-only apart from the one resolution; never
/// loads or creates the device identity.
fn review(paths: &ProfilePaths, json_out: bool, resolve: Option<&(bool, String)>) -> Result<i32> {
    use crate::app::sync_view::pending_view;
    let store = open_store(paths)?;
    let rows = store.sync_devices()?;
    // Every pending op with its reason (`unknown-author` ones included).
    let views: Vec<_> = store
        .sync_pending_with_reasons()?
        .iter()
        .map(|(op, reason)| pending_view(&store, op, &rows, Some(reason.clone())))
        .collect();
    if let Some((approve, prefix)) = resolve {
        let hashes: Vec<[u8; 32]> = views.iter().map(|v| v.hash).collect();
        let hash = match resolve_prefix(&hashes, prefix) {
            Ok(h) => h,
            Err(msg) => return Ok(usage_error("sync-review", &msg)),
        };
        let view = views
            .iter()
            .find(|v| v.hash == hash)
            .expect("resolved above");
        let id = hex(&hash)[..12].to_string();
        // A share release is answered with the grants the TUI's confirmation would write: the
        // explicit approval is this `--approve` (the listing names the requester, its full id and
        // the request's age).
        let is_release = *approve
            && store.sync_pending()?.iter().any(|o| {
                o.hash() == hash
                    && matches!(o.body, crate::sync::oplog::OpBody::ShareRelease { .. })
            });
        if is_release {
            approve_release(
                &store,
                &open_identity(paths)?,
                &hash,
                crate::store::now_ts(),
            )?;
        } else {
            crate::sync::ceremony::resolve(&store, &hash, *approve, crate::store::now_ts())?;
        }
        let verb = if *approve { "Approved" } else { "Rejected" };
        if json_out {
            let doc = json!({
                "id": id, "result": verb.to_lowercase(), "summary": view.summary,
            });
            println!("{}", serde_json::to_string_pretty(&doc)?);
        } else {
            println!("{verb} {id}: {}", plain(&view.summary));
        }
        return Ok(0);
    }
    if json_out {
        let list: Vec<_> = views
            .iter()
            .map(|v| {
                json!({
                    "id": &hex(&v.hash)[..12], "author": v.device, "summary": v.summary,
                    "reason": v.reason,
                    "ceremony": v.red.as_ref().map(|r| json!({
                        "have": r.have, "missing": r.missing, "expired": r.expired,
                        "blocked": r.blocked, "single_machine": r.single_machine,
                    })),
                    "changes": v.changes.iter().map(|c| json!({
                        "field": c.field, "from": c.from, "to": c.to, "dangerous": c.dangerous,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(0);
    }
    if views.is_empty() {
        println!("Nothing waiting for review.");
        return Ok(0);
    }
    for v in &views {
        println!(
            "{}  {}  {}",
            &hex(&v.hash)[..12],
            plain(&v.device),
            plain(&v.summary)
        );
        for c in &v.changes {
            let mark = if c.dangerous { "[!]" } else { "   " };
            println!(
                "  {mark} {}: {} -> {}",
                c.field,
                plain(&c.from),
                plain(&c.to)
            );
        }
        if let Some(red) = &v.red {
            println!("  Red change, three proofs needed:");
            for h in &red.have {
                println!("    present: {h}");
            }
            for m in &red.missing {
                println!("    missing: {m}");
            }
            if let Some(why) = &red.blocked {
                println!("    impossible as it stands: {}", plain(why));
            }
            if red.expired {
                println!("    expired: reject it and propose it again");
            }
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parse_join_needs_one_host() {
        assert!(parse_args("sync-join", &[]).is_err());
        assert!(parse_args("sync-join", &s(&["a", "b"])).is_err());
        let p = parse_args("sync-join", &s(&["me@box:2222"])).unwrap();
        assert_eq!(p.positionals, s(&["me@box:2222"]));
    }

    #[test]
    fn parse_now_format_and_device() {
        let p = parse_args("sync-now", &s(&["lap", "--format", "json"])).unwrap();
        assert_eq!(p.positionals, s(&["lap"]));
        assert!(p.json);
        assert!(parse_args("sync-now", &s(&["--format", "xml"])).is_err());
        assert!(parse_args("sync-now", &s(&["--format"])).is_err());
        assert!(parse_args("sync-now", &s(&["--bogus"])).is_err());
    }

    #[test]
    fn parse_serve_and_status_reject_extras() {
        assert!(parse_args("sync-serve", &s(&["x"])).is_err());
        assert!(parse_args("sync-status", &s(&["x"])).is_err());
        assert!(parse_args("sync-serve", &s(&["--format", "json"])).is_err());
        assert!(
            parse_args("sync-status", &s(&["--format", "json"]))
                .unwrap()
                .json
        );
    }

    #[test]
    fn device_resolution() {
        let d = vec![
            ("aabbccdd11223344".to_string(), "laptop".to_string()),
            ("aabb000011223344".to_string(), "phone".to_string()),
            ("ff00000011223344".to_string(), "phone".to_string()),
        ];
        assert_eq!(resolve_device(&d, "laptop"), Ok("aabbccdd11223344"));
        assert_eq!(resolve_device(&d, "aabbc"), Ok("aabbccdd11223344"));
        assert_eq!(resolve_device(&d, "ff"), Ok("ff00000011223344"));
        assert_eq!(
            resolve_device(&d, "aabb000011223344"),
            Ok("aabb000011223344")
        );
        assert!(resolve_device(&d, "aabb")
            .unwrap_err()
            .contains("ambiguous"));
        assert!(resolve_device(&d, "phone")
            .unwrap_err()
            .contains("ambiguous"));
        assert!(resolve_device(&d, "zzz").is_err());
        assert!(resolve_device(&d, "").is_err());
    }

    #[test]
    fn parse_review_flags() {
        let p = parse_args("sync-review", &s(&["--approve", "abcd"])).unwrap();
        assert_eq!(p.resolve, Some((true, "abcd".to_string())));
        let p = parse_args("sync-review", &s(&["--reject", "abcd", "--format", "json"])).unwrap();
        assert_eq!(p.resolve, Some((false, "abcd".to_string())));
        assert!(p.json);
        assert!(parse_args("sync-review", &s(&["--approve", "a", "--reject", "b"])).is_err());
        assert!(parse_args("sync-review", &s(&["--approve"])).is_err());
        assert!(parse_args("sync-review", &s(&["extra"])).is_err());
        assert!(parse_args("sync-now", &s(&["--approve", "abcd"])).is_err());
    }

    #[test]
    fn prefix_resolution() {
        let mut a = [0u8; 32];
        a[0] = 0xab;
        a[1] = 0xcd;
        let mut b = a;
        b[2] = 0x01;
        let mut c = [0u8; 32];
        c[0] = 0x12;
        c[1] = 0x34;
        let all = [a, b, c];
        assert_eq!(resolve_prefix(&all, "1234"), Ok(c));
        assert_eq!(
            resolve_prefix(&all, "1234AB"),
            Err("no pending op matches '1234AB'".into())
        );
        assert!(resolve_prefix(&all, "abcd")
            .unwrap_err()
            .contains("several"));
        assert_eq!(resolve_prefix(&all, "abcd01"), Ok(b));
        assert!(resolve_prefix(&all, "12").unwrap_err().contains("4 hex"));
        assert!(resolve_prefix(&all, "zzzz").is_err());
    }

    /// `sync-join` against an in-process existing device: the words go to stdout, the
    /// adoption of the ring is reported line by line on stderr.
    #[test]
    fn join_reports_the_ring_being_adopted_on_stderr() {
        use crate::sync::enroll::run_sync_join;
        use crate::sync::protocol::tests::Dev;
        let peer = Dev::new();
        crate::sync::quorum::meta_set(&peer.store, "ring_id", peer.did.as_bytes()).unwrap();
        let me = Dev::new();
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        let (peer_store, peer_id) = (peer.store.clone(), peer.id.clone());
        let hub = std::thread::spawn(move || {
            let mut conn = Conn::new(
                b.try_clone().unwrap(),
                b,
                Limits::default(),
                Arc::new(AtomicBool::new(false)),
            );
            let mut ui = |_: &SasPrompt<'_>| true;
            run_sync_enroll(&mut conn, &peer_store, &peer_id, "hub", true, Some(&mut ui))
        });
        let mut conn = Conn::new(
            a.try_clone().unwrap(),
            a,
            Limits::default(),
            Arc::new(AtomicBool::new(false)),
        );
        let mut ui = JoinPrompt {
            host: "hub",
            input: std::io::Cursor::new(b"y\n".to_vec()),
            out: Vec::new(),
            err: Vec::new(),
        };
        run_sync_join(&mut conn, &me.store, &me.id, "me", true, Some(&mut ui)).unwrap();
        hub.join().unwrap().unwrap();
        let err = String::from_utf8(ui.err).unwrap();
        let lines: Vec<&str> = err.lines().collect();
        assert!(lines[0].starts_with("receiving the ring ("), "{err}");
        assert!(
            lines[lines.len() - 2].starts_with("adopting the ring ("),
            "{err}"
        );
        assert!(
            lines[lines.len() - 1].starts_with("adopted the ring ("),
            "{err}"
        );
        let out = String::from_utf8(ui.out).unwrap();
        assert!(
            out.contains("Compare these words with the other device: "),
            "{out}"
        );
        assert!(out.ends_with("Do they match? [y/N] "), "{out}");
    }

    #[test]
    fn a_stale_share_release_is_refused_and_a_fresh_one_is_not() {
        use crate::sync::oplog::OpBody;
        let author = LauncherStore::open_in_memory().unwrap();
        let id = DeviceIdentity::generate();
        let op = author
            .sync_append_local(
                &id,
                OpBody::ShareRelease {
                    host_uid: [1; 16],
                    request_id: 1,
                    requester: id.public().device_id(),
                    eph_x25519_pub: [2; 32],
                },
            )
            .unwrap();
        assert!(release_fresh(&op, op.timestamp + 5).is_ok());
        let e = release_fresh(&op, op.timestamp + 3_600).unwrap_err();
        assert!(e.to_string().contains("too old"), "{e}");
        assert!(
            release_fresh(&op, op.timestamp - 3_600).is_err(),
            "future-dated too"
        );
    }

    #[test]
    fn yes_answers() {
        assert!(is_yes("y\n"));
        assert!(is_yes(" YES "));
        assert!(!is_yes(""));
        assert!(!is_yes("n"));
    }

    /// A profile whose store holds one pending host creation from `laptop`.
    fn profile_with_pending() -> (tempfile::TempDir, ProfilePaths, String) {
        use crate::sync::oplog::{Attendance, DeviceRole, HostFields, OpBody};
        let dir = tempfile::tempdir().unwrap();
        let paths = ProfilePaths {
            data_root: dir.path().to_path_buf(),
            id: "t".into(),
            name: "t".into(),
            root: dir.path().to_path_buf(),
            config_file: dir.path().join("config.toml"),
            ssh_config: dir.path().join("ssh_config"),
            compat: false,
        };
        let store = LauncherStore::open(paths.launcher_db()).unwrap();
        let author = LauncherStore::open_in_memory().unwrap();
        let id = DeviceIdentity::generate();
        for st in [&store, &author] {
            st.sync_register_device(
                &id.public(),
                "laptop",
                DeviceRole::Holder,
                Attendance::Attended,
                crate::sync::oplog::DeviceKind::Terminal,
                VerifiedBy::Sas,
            )
            .unwrap();
        }
        let op = author
            .sync_append_local(
                &id,
                OpBody::HostUpsert {
                    host_uid: [9; 16],
                    fields: HostFields {
                        name: Some("web".into()),
                        address: Some("10.0.0.1".into()),
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        store.sync_ingest(op.clone()).unwrap();
        (dir, paths, hex(&op.hash())[..12].to_string())
    }

    #[test]
    fn approving_by_prefix_applies_the_op_and_rejecting_drops_it() {
        let (_d, paths, id) = profile_with_pending();
        assert_eq!(review(&paths, false, None).unwrap(), 0);
        assert_eq!(
            review(&paths, false, Some(&(true, id[..6].into()))).unwrap(),
            0
        );
        let store = LauncherStore::open(paths.launcher_db()).unwrap();
        assert!(store.get_host_by_name("web").unwrap().is_some());
        assert!(store.sync_pending().unwrap().is_empty());
        // Nothing left: the same id is now unknown (usage error).
        assert_eq!(review(&paths, false, Some(&(true, id.clone()))).unwrap(), 2);

        let (_d, paths, id) = profile_with_pending();
        assert_eq!(review(&paths, true, Some(&(false, id))).unwrap(), 0);
        let store = LauncherStore::open(paths.launcher_db()).unwrap();
        assert!(store.get_host_by_name("web").unwrap().is_none());
        assert!(store.sync_pending().unwrap().is_empty());
    }
}
