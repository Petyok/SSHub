//! Opening synced credentials for a blocking CLI command: the Vault master
//! phrase is asked on the terminal, without echo.

use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;

use anyhow::{ensure, Context, Result};
use zeroize::Zeroizing;

use super::context::CliContext;
use crate::credentials::{needs_share_message, UnlockError};
use crate::sync::crypto::Share;
use crate::sync::identity_store::load_or_init_identity;
use crate::sync::oplog::HostUid;
use crate::sync::quorum::{parse_ring_secret, QuorumUnlocker, ShareSource};
use crate::sync::runtime::RING_SECRET_KEY;

struct PromptShares;

impl ShareSource for PromptShares {
    fn master_phrase(&self) -> Option<Zeroizing<String>> {
        prompt_secret("Master phrase (six words): ")
    }

    fn second_share(
        &self,
        _host: &HostUid,
        _epoch: u64,
        _mine: &Share,
    ) -> std::result::Result<Share, UnlockError> {
        Err(UnlockError::NeedsShares(
            "no way to ask another holder from the command line yet: open it from the TUI".into(),
        ))
    }
}

/// A line from the controlling terminal with echo off, or from stdin when
/// there is no terminal.
fn prompt_secret(label: &str) -> Option<Zeroizing<String>> {
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok();
    let mut line = Zeroizing::new(String::new());
    match tty {
        Some(mut tty) => {
            let _ = write!(tty, "{label}");
            let _ = tty.flush();
            let restore = NoEcho::on(&tty);
            let read = BufReader::new(&tty).read_line(&mut line);
            drop(restore);
            let _ = writeln!(tty);
            read.ok()?;
        }
        None => {
            std::io::stdin().lock().read_line(&mut line).ok()?;
        }
    }
    let trimmed = line.trim();
    (!trimmed.is_empty()).then(|| Zeroizing::new(trimmed.to_string()))
}

/// Echo off on a terminal until dropped.
struct NoEcho {
    fd: i32,
    old: libc::termios,
}

impl NoEcho {
    fn on(tty: &std::fs::File) -> Option<Self> {
        use std::os::fd::AsRawFd;
        let fd = tty.as_raw_fd();
        // SAFETY: plain termios calls on a descriptor this function owns a
        // borrow of; `old` is fully written by a successful tcgetattr.
        unsafe {
            let mut old: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut old) != 0 {
                return None;
            }
            let mut quiet = old;
            quiet.c_lflag &= !libc::ECHO;
            if libc::tcsetattr(fd, libc::TCSANOW, &quiet) != 0 {
                return None;
            }
            Some(Self { fd, old })
        }
    }
}

impl Drop for NoEcho {
    fn drop(&mut self) {
        // SAFETY: restores the attributes read in `on`; the descriptor
        // outlives this guard in `prompt_secret`.
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.old);
        }
    }
}

/// The unlocker a CLI command uses when none was injected: needs a ring on
/// this profile; the ring secret comes from the credential store.
pub(crate) fn default_unlocker(ctx: &CliContext) -> Result<QuorumUnlocker> {
    let gate = ctx
        .password_store()
        .shared_gate()
        .context("this credential store has no gate")?;
    ensure!(
        !ctx.store.sync_devices()?.is_empty(),
        "{}",
        needs_share_message()
    );
    let local = crate::credentials::open_ungated_for_profile(&ctx.profile);
    let identity = load_or_init_identity(&ctx.profile, local.as_ref())?;
    let ring = local
        .get(RING_SECRET_KEY)?
        .and_then(|t| parse_ring_secret(&t).ok());
    Ok(QuorumUnlocker::new(
        ctx.store.clone(),
        Arc::new(identity),
        gate,
        Box::new(PromptShares),
        ring,
    ))
}
