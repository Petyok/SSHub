//! `sshub sftp …`: one-shot SFTP operations over a direct host.
//!
//! Each subcommand drives the existing background SFTP worker
//! ([`crate::sftp::worker`]) synchronously: connect, send a single command,
//! then block on the event channel until the worker reports the terminal event
//! (`QueueDone` for transfers, `OpDone` for remote ops, `DirListing` for `ls`).
//! There is no TUI and no queue building here; the worker's own recursion
//! handles directory trees when a transfer is flagged `is_dir`.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use anyhow::Result;
use serde::Serialize;

use super::parse;
use super::CliContext;
use crate::sftp::model::{Direction, QueuedTransfer, Side};
use crate::sftp::{SftpCommand, SftpEvent};

pub fn run(ctx: &mut CliContext, args: &[String]) -> Result<i32> {
    match args.first().map(String::as_str) {
        Some("ls") => cmd_ls(ctx, &args[1..]),
        Some("get") => cmd_get(ctx, &args[1..]),
        Some("put") => cmd_put(ctx, &args[1..]),
        Some("rm") => cmd_rm(ctx, &args[1..]),
        Some("mkdir") => cmd_mkdir(ctx, &args[1..]),
        Some("rename") => cmd_rename(ctx, &args[1..]),
        Some("chmod") => cmd_chmod(ctx, &args[1..]),
        Some(other) => {
            eprintln!(
                "sshub: unknown sftp subcommand '{other}' (try: ls, get, put, rm, mkdir, rename, chmod)"
            );
            Ok(2)
        }
        None => parse::usage("sftp needs a subcommand (ls|get|put|rm|mkdir|rename|chmod)"),
    }
}

/// Resolve `host`, spawn the SFTP worker, and block until the connection
/// result arrives. On success returns the live channels; on any failure prints
/// a diagnostic to stderr and returns the process exit code to use.
///
/// ProxyJump hosts are rejected up front: the libssh2 transport cannot chain a
/// jump, so a connection attempt would only hang and then fail.
fn connect_worker(
    ctx: &CliContext,
    host: &str,
) -> Result<(Sender<SftpCommand>, Receiver<SftpEvent>), i32> {
    let entry = match ctx.host_by_name(host) {
        Ok(e) => e.clone(),
        Err(e) => {
            eprintln!("sshub: {e}");
            return Err(1);
        }
    };
    let ssh_host = entry.ssh_host();
    if ssh_host.proxy_jump.is_some() {
        eprintln!("sshub: SFTP via ProxyJump is not supported; pick a direct host");
        return Err(1);
    }

    let (secret, _diag) = crate::app::resolve_pending_secret(&entry, ctx.password_store.as_ref());
    let agent = crate::ssh::agent::detect_agent();
    let (tx, rx) = crate::sftp::spawn_sftp_worker(ssh_host, secret, agent);

    match rx.recv() {
        Ok(SftpEvent::Connected) => Ok((tx, rx)),
        Ok(SftpEvent::ConnectFailed(e)) => {
            eprintln!("sshub: connect failed: {e}");
            Err(1)
        }
        other => {
            eprintln!("sshub: unexpected sftp event: {other:?}");
            Err(1)
        }
    }
}

/// One JSON row for `ls --format json`.
#[derive(Serialize)]
struct LsEntryJson {
    name: String,
    is_dir: bool,
    size: u64,
}

/// `sshub sftp ls <host> [remote-path] [--format plain|json]`
fn cmd_ls(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let fmt = match parse::parse_format(&rest) {
        Ok(f) => f,
        Err(e) => parse::usage(&e),
    };
    let _ = parse::take_opt(&mut rest, "--format");
    let pos = parse::positional(&rest);
    let host = match pos.first() {
        Some(h) => *h,
        None => parse::usage("sftp ls needs a <host>"),
    };
    let path = PathBuf::from(pos.get(1).copied().unwrap_or("."));

    let (tx, rx) = match connect_worker(ctx, host) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    if tx
        .send(SftpCommand::ListDir(Side::Remote, path.clone()))
        .is_err()
    {
        eprintln!("sshub: sftp worker went away");
        return Ok(1);
    }

    loop {
        match rx.recv() {
            Ok(SftpEvent::DirListing(_, _, entries)) => {
                match fmt {
                    parse::OutputFormat::Plain => {
                        for e in &entries {
                            println!("{}", e.name);
                        }
                    }
                    parse::OutputFormat::Json => {
                        let rows: Vec<LsEntryJson> = entries
                            .iter()
                            .map(|e| LsEntryJson {
                                name: e.name.clone(),
                                is_dir: e.is_dir,
                                size: e.size,
                            })
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&rows)?);
                    }
                }
                return Ok(0);
            }
            Ok(SftpEvent::Error(e)) => {
                eprintln!("sshub: {e}");
                return Ok(1);
            }
            Ok(_) => continue,
            Err(_) => {
                eprintln!("sshub: sftp worker went away");
                return Ok(1);
            }
        }
    }
}

/// `sshub sftp get <host> <remote-path> [local-path] [--recursive]`
fn cmd_get(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let recursive = parse::take_flag(&mut rest, "--recursive");
    let pos = parse::positional(&rest);
    let host = match pos.first() {
        Some(h) => *h,
        None => parse::usage("sftp get needs a <host> and <remote-path>"),
    };
    let remote = match pos.get(1) {
        Some(r) => PathBuf::from(*r),
        None => parse::usage("sftp get needs a <remote-path>"),
    };
    let base = remote
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());
    let local = pos
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".").join(&base));

    let (tx, rx) = match connect_worker(ctx, host) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let item = QueuedTransfer {
        direction: Direction::Download,
        src: remote,
        dst: local,
        name: base,
        is_dir: recursive,
    };
    run_queue(&tx, &rx, item)
}

/// `sshub sftp put <host> <local-path> [remote-path] [--recursive]`
fn cmd_put(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let recursive = parse::take_flag(&mut rest, "--recursive");
    let pos = parse::positional(&rest);
    let host = match pos.first() {
        Some(h) => *h,
        None => parse::usage("sftp put needs a <host> and <local-path>"),
    };
    let local = match pos.get(1) {
        Some(l) => PathBuf::from(*l),
        None => parse::usage("sftp put needs a <local-path>"),
    };
    let base = local
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());
    let remote = pos
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&base));
    // A local directory is always transferred recursively; --recursive is also
    // honoured so a caller can force it explicitly.
    let is_dir = std::fs::metadata(&local)
        .map(|m| m.is_dir())
        .unwrap_or(false)
        || recursive;

    let (tx, rx) = match connect_worker(ctx, host) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let item = QueuedTransfer {
        direction: Direction::Upload,
        src: local,
        dst: remote,
        name: base,
        is_dir,
    };
    run_queue(&tx, &rx, item)
}

/// Send a single-item queue and block until `QueueDone`, aggregating any
/// per-transfer `Error` events. Returns exit 1 if any error was reported.
fn run_queue(
    tx: &Sender<SftpCommand>,
    rx: &Receiver<SftpEvent>,
    item: QueuedTransfer,
) -> Result<i32> {
    if tx.send(SftpCommand::RunQueue(vec![item])).is_err() {
        eprintln!("sshub: sftp worker went away");
        return Ok(1);
    }
    let mut had_err = false;
    loop {
        match rx.recv() {
            Ok(SftpEvent::QueueDone) => break,
            Ok(SftpEvent::Error(e)) => {
                eprintln!("sshub: {e}");
                had_err = true;
            }
            Ok(_) => continue,
            Err(_) => {
                eprintln!("sshub: sftp worker went away");
                return Ok(1);
            }
        }
    }
    Ok(if had_err { 1 } else { 0 })
}

/// `sshub sftp rm <host> <remote-path> [--recursive] [--yes]`
///
/// DESTRUCTIVE. With `--recursive` the whole subtree is deleted and there is no
/// undo, so the operation is refused unless `--yes` confirms it.
fn cmd_rm(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let mut rest = args.to_vec();
    let recursive = parse::take_flag(&mut rest, "--recursive");
    let yes = parse::take_flag(&mut rest, parse::CONFIRM_YES);
    let pos = parse::positional(&rest);
    let host = match pos.first() {
        Some(h) => *h,
        None => parse::usage("sftp rm needs a <host> and <remote-path>"),
    };
    let remote = match pos.get(1) {
        Some(r) => PathBuf::from(*r),
        None => parse::usage("sftp rm needs a <remote-path>"),
    };
    // Deleting a tree is irreversible: demand explicit confirmation.
    if !yes {
        parse::fail_code("sftp rm is destructive; pass --yes to confirm", 1);
    }

    let (tx, rx) = match connect_worker(ctx, host) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    if tx.send(SftpCommand::Remove(remote, recursive)).is_err() {
        eprintln!("sshub: sftp worker went away");
        return Ok(1);
    }
    run_op(&rx)
}

/// `sshub sftp mkdir <host> <remote-path>`
fn cmd_mkdir(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let pos = parse::positional(args);
    let host = match pos.first() {
        Some(h) => *h,
        None => parse::usage("sftp mkdir needs a <host> and <remote-path>"),
    };
    let remote = match pos.get(1) {
        Some(r) => PathBuf::from(*r),
        None => parse::usage("sftp mkdir needs a <remote-path>"),
    };

    let (tx, rx) = match connect_worker(ctx, host) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    if tx.send(SftpCommand::Mkdir(remote)).is_err() {
        eprintln!("sshub: sftp worker went away");
        return Ok(1);
    }
    run_op(&rx)
}

/// `sshub sftp rename <host> <from> <to>`
fn cmd_rename(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let pos = parse::positional(args);
    let host = match pos.first() {
        Some(h) => *h,
        None => parse::usage("sftp rename needs a <host>, <from> and <to>"),
    };
    let from = match pos.get(1) {
        Some(f) => PathBuf::from(*f),
        None => parse::usage("sftp rename needs a <from> path"),
    };
    let to = match pos.get(2) {
        Some(t) => PathBuf::from(*t),
        None => parse::usage("sftp rename needs a <to> path"),
    };

    let (tx, rx) = match connect_worker(ctx, host) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    if tx.send(SftpCommand::Rename(from, to)).is_err() {
        eprintln!("sshub: sftp worker went away");
        return Ok(1);
    }
    run_op(&rx)
}

/// `sshub sftp chmod <host> <mode> <remote-path>` (`mode` is an octal string).
fn cmd_chmod(ctx: &CliContext, args: &[String]) -> Result<i32> {
    let pos = parse::positional(args);
    let host = match pos.first() {
        Some(h) => *h,
        None => parse::usage("sftp chmod needs a <host>, <mode> and <remote-path>"),
    };
    let mode_str = match pos.get(1) {
        Some(m) => *m,
        None => parse::usage("sftp chmod needs an octal <mode>"),
    };
    let remote = match pos.get(2) {
        Some(r) => PathBuf::from(*r),
        None => parse::usage("sftp chmod needs a <remote-path>"),
    };
    let mode = match u32::from_str_radix(mode_str, 8) {
        Ok(m) => m,
        Err(_) => parse::usage(&format!("invalid octal mode '{mode_str}'")),
    };

    let (tx, rx) = match connect_worker(ctx, host) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    if tx.send(SftpCommand::Chmod(remote, mode)).is_err() {
        eprintln!("sshub: sftp worker went away");
        return Ok(1);
    }
    run_op(&rx)
}

/// Block until a remote op reports `OpDone` (success) or `Error` (failure).
fn run_op(rx: &Receiver<SftpEvent>) -> Result<i32> {
    loop {
        match rx.recv() {
            Ok(SftpEvent::OpDone) => return Ok(0),
            Ok(SftpEvent::Error(e)) => {
                eprintln!("sshub: {e}");
                return Ok(1);
            }
            Ok(_) => continue,
            Err(_) => {
                eprintln!("sshub: sftp worker went away");
                return Ok(1);
            }
        }
    }
}
