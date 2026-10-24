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
pub(crate) fn prompt_secret(label: &str) -> Option<Zeroizing<String>> {
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

/// Echo off on a terminal until dropped, and restored if SIGINT, SIGTERM, SIGHUP or
/// SIGQUIT arrives first (the handler restores the terminal, then re-raises the
/// signal with its previous disposition).
struct NoEcho {
    fd: i32,
    old: libc::termios,
    previous: Vec<(libc::c_int, libc::sigaction)>,
}

/// The attributes the signal handler puts back (null when no prompt is open).
static SAVED: std::sync::atomic::AtomicPtr<libc::termios> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static SAVED_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
const GUARDED_SIGNALS: [libc::c_int; 4] =
    [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];

/// Put the saved attributes back. Async-signal-safe: atomics and `tcsetattr` only.
fn restore_saved() {
    use std::sync::atomic::Ordering;
    let p = SAVED.load(Ordering::SeqCst);
    if !p.is_null() {
        // SAFETY: `p` is a leaked Box<termios> stored by `NoEcho::on`, never freed.
        unsafe {
            libc::tcsetattr(SAVED_FD.load(Ordering::SeqCst), libc::TCSANOW, &*p);
        }
    }
}

extern "C" fn restore_on_signal(sig: libc::c_int) {
    restore_saved();
    // SAFETY: back to the default action, then the same signal again.
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

impl NoEcho {
    fn on(tty: &std::fs::File) -> Option<Self> {
        use std::os::fd::AsRawFd;
        use std::sync::atomic::Ordering;
        let fd = tty.as_raw_fd();
        // SAFETY: plain termios and sigaction calls on a descriptor this function owns a
        // borrow of; `old` is fully written by a successful tcgetattr.
        unsafe {
            let mut old: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut old) != 0 {
                return None;
            }
            let mut quiet = old;
            quiet.c_lflag &= !libc::ECHO;
            // The handler may fire the moment echo is off: publish the way back first.
            SAVED_FD.store(fd, Ordering::SeqCst);
            SAVED.store(Box::into_raw(Box::new(old)), Ordering::SeqCst);
            let mut previous = Vec::new();
            for sig in GUARDED_SIGNALS {
                let mut act: libc::sigaction = std::mem::zeroed();
                act.sa_sigaction = restore_on_signal as extern "C" fn(libc::c_int) as usize;
                libc::sigemptyset(&mut act.sa_mask);
                let mut prev: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(sig, &act, &mut prev) == 0 {
                    previous.push((sig, prev));
                }
            }
            let guard = Self { fd, old, previous };
            if libc::tcsetattr(fd, libc::TCSANOW, &quiet) != 0 {
                return None; // the guard's Drop puts everything back
            }
            Some(guard)
        }
    }
}

impl Drop for NoEcho {
    fn drop(&mut self) {
        // SAFETY: restores the attributes and handlers read in `on`; the descriptor
        // outlives this guard in `prompt_secret`.
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.old);
            for (sig, prev) in &self.previous {
                libc::sigaction(*sig, prev, std::ptr::null_mut());
            }
        }
        // The leaked attributes stay allocated (a few hundred bytes per prompt); the
        // handler can no longer be reached once the old handlers are back.
        SAVED.store(std::ptr::null_mut(), std::sync::atomic::Ordering::SeqCst);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::FromRawFd;

    fn echo_on(fd: i32) -> bool {
        // SAFETY: tcgetattr on a pty descriptor owned by the test.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            assert_eq!(libc::tcgetattr(fd, &mut t), 0);
            t.c_lflag & libc::ECHO != 0
        }
    }

    #[test]
    fn echo_comes_back_when_a_signal_interrupts_the_prompt() {
        let (mut master, mut slave) = (0, 0);
        // SAFETY: openpty fills two descriptors the test then owns.
        let ok = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(ok, 0);
        // SAFETY: both descriptors are fresh and owned here.
        let (_master, tty) = unsafe {
            (
                std::fs::File::from_raw_fd(master),
                std::fs::File::from_raw_fd(slave),
            )
        };
        assert!(echo_on(slave));
        let guard = NoEcho::on(&tty).expect("echo off");
        assert!(!echo_on(slave), "the prompt turned echo off");
        // What the SIGINT/SIGTERM/SIGHUP handler does first.
        restore_saved();
        assert!(echo_on(slave), "the handler puts echo back");
        drop(guard);
        assert!(echo_on(slave));
    }
}
