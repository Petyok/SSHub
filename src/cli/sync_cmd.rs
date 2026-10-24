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
use crate::sync::enroll::run_sync_enroll;
use crate::sync::identity_store::load_or_init_identity;
use crate::sync::protocol::{run_sync_serve, RoundSummary};
use crate::sync::runtime::{device_name, enroll_limits, Peers, SyncRuntime};
use crate::sync::transport::{Conn, Limits};

use super::help;

const COMMANDS: [&str; 5] = [
    "sync-serve",
    "sync-enroll",
    "sync-join",
    "sync-now",
    "sync-status",
];

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
}

/// Arguments after the (already extracted) global profile flags.
fn parse_args(cmd: &str, args: &[String]) -> Result<Parsed, String> {
    let takes_format = matches!(cmd, "sync-now" | "sync-status");
    let mut out = Parsed {
        positionals: Vec::new(),
        json: false,
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
    run_sync_serve(&store, &identity, &mut conn)?;
    Ok(0)
}

fn is_yes(line: &str) -> bool {
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}

/// Ask on the controlling terminal; with none, report the words on stderr and
/// accept (the human comparison happens on the joining side).
fn confirm_on_tty(sas: &str, peer: &str) -> bool {
    let tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .and_then(|w| w.try_clone().map(|r| (w, r)));
    let Ok((mut w, r)) = tty else {
        eprintln!("SAS words: {sas} (peer {peer})");
        return true;
    };
    let _ = write!(
        w,
        "Enrolling {peer}. Compare these words with the other device: {sas}\nDo they match? [y/N] "
    );
    let _ = w.flush();
    let mut line = String::new();
    BufReader::new(r).read_line(&mut line).is_ok() && is_yes(&line)
}

fn enroll(paths: &ProfilePaths) -> Result<i32> {
    let out = take_protocol_stdout()?;
    let store = open_store(paths)?;
    let identity = open_identity(paths)?;
    let mut conn = Conn::new(
        std::io::stdin(),
        out,
        enroll_limits(),
        Arc::new(AtomicBool::new(false)),
    );
    run_sync_enroll(
        &mut conn,
        &store,
        &identity,
        &device_name(),
        true,
        &mut confirm_on_tty,
    )?;
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

fn join(paths: &ProfilePaths, host: &str) -> Result<i32> {
    let rt = runtime(paths)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let mut ui = |sas: &str, peer: &str| {
        println!("Compare these words with the other device: {sas}");
        println!("Peer: {peer}");
        print!("Do they match? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).is_ok() && is_yes(&line)
    };
    match rt.join(host, &mut ui, &cancel) {
        Ok(o) => {
            println!(
                "Joined the ring of {} (device {}). This device is a mirror until the ring approves more.",
                o.peer_name,
                short(&o.peer.device_id())
            );
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
    Ok(RoundSummary),
    Skipped,
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
                Ok(s) => Outcome::Ok(s),
                Err(e) => Outcome::Failed(reason(&e).to_string()),
            }
        };
        results.push((id, name, outcome));
    }

    let mut fork = false;
    let mut failed = false;
    let mut attempted = false;
    let mut array = Vec::new();
    for (id, name, outcome) in &results {
        match outcome {
            Outcome::Skipped => {
                if !json_out {
                    println!("{name}: skipped (no host known)");
                }
                array.push(json!({"device": id, "name": name, "status": "skipped"}));
            }
            Outcome::Failed(r) => {
                attempted = true;
                failed = true;
                if !json_out {
                    println!("{name}: offline/failed: {r}");
                }
                array.push(json!({"device": id, "name": name, "status": "failed", "error": r}));
            }
            Outcome::Ok(s) => {
                attempted = true;
                if !s.forks.is_empty() {
                    fork = true;
                    if !json_out {
                        println!("FORK DETECTED: {name}");
                    }
                }
                if !json_out {
                    let mut line = format!(
                        "{name}: ok applied {}, pending {}, received {}, sent {}",
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
fn status(paths: &ProfilePaths, json_out: bool) -> Result<i32> {
    let store = open_store(paths)?;
    let rows = store.sync_devices()?;
    let pending = store.sync_pending()?.len();
    let mode = lower(store.sync_cred_mode()?);
    let peers = Peers::load(paths)?;
    if json_out {
        let devices: Vec<_> = rows
            .iter()
            .map(|r| {
                json!({
                    "id": r.id, "name": r.name, "role": lower(r.role),
                    "status": lower(r.status), "last_seen": r.last_seen,
                    "head_seq": r.head_seq, "host": peers.host_of(&r.id),
                })
            })
            .collect();
        let doc = json!({"devices": devices, "pending": pending, "credential_mode": mode});
        println!("{}", serde_json::to_string_pretty(&doc)?);
        return Ok(0);
    }
    if rows.is_empty() && pending == 0 {
        println!("{EMPTY_RING}");
        return Ok(0);
    }
    for r in &rows {
        println!(
            "{} ({}) role {} status {} last seen {} head {} host {}",
            r.name,
            short(&r.id),
            lower(r.role),
            lower(r.status),
            age(r.last_seen),
            r.head_seq,
            peers.host_of(&r.id).unwrap_or("-"),
        );
    }
    println!("Pending changes: {pending}");
    println!("Credential mode: {mode}");
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
    fn yes_answers() {
        assert!(is_yes("y\n"));
        assert!(is_yes(" YES "));
        assert!(!is_yes(""));
        assert!(!is_yes("n"));
    }
}
