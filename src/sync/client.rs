//! Sync client side: the connection to a peer, the ssh transport that carries
//! it, and the worker thread the UI talks to.
//!
//! The protocol itself (messages, the round, enrollment) belongs to the
//! protocol package: it supplies one [`RoundFn`] and this module runs it on a
//! worker thread, with deadlines, cancellation and a clean shutdown.

use std::io::{self, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SendError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::transport::{Conn, Limits, CANCEL_POLL};
use crate::app::sync::{OnboardingSecrets, UnlockPathKind};
use crate::ssh::{build_ssh_argv, SshHost};
pub use crate::sync::runtime::{JobBox, JobEnv, JobFn};

// The worker speaks the protocol's own types.
pub use crate::sync::oplog::DeviceId;
pub use crate::sync::protocol::RoundSummary;

/// The error event text for a worker thread that is gone.
pub const WORKER_STOPPED: &str = "sync worker stopped";

// ── ssh transport ────────────────────────────────────────

/// Why a round over ssh did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// No hello: the peer is offline, unreachable or refused us.
    Offline(String),
    /// The session started, then failed.
    Failed(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::Offline(r) => write!(f, "offline: {r}"),
            TransportError::Failed(r) => write!(f, "failed: {r}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// Reaches a peer by running `<remote_command>` over ssh, stdin and stdout
/// piped, stderr read separately.
pub struct SshTransport {
    host: SshHost,
    remote_command: String,
    /// A program to run instead of `ssh` from `PATH`.
    program: Option<String>,
}

impl SshTransport {
    pub fn new(host: SshHost, remote_command: impl Into<String>) -> Self {
        Self {
            host,
            remote_command: remote_command.into(),
            program: None,
        }
    }

    /// Run `program` where the argv names `ssh` (an ssh at a known path).
    pub fn with_program(mut self, program: impl Into<String>) -> Self {
        self.program = Some(program.into());
        self
    }

    /// The ssh argv: the existing builder's, with `BatchMode=yes` and
    /// `ConnectTimeout=4` placed last among the options. OpenSSH takes the
    /// first value it sees, so a user's `-o` or `~/.ssh/config` can neither
    /// re-enable prompts nor lengthen the connect; no prompt can ever appear.
    pub fn argv(&self) -> Vec<String> {
        let mut host = self.host.clone();
        host.remote_command = None;
        let mut argv = build_ssh_argv(&host);
        let target = argv.pop().expect("the builder always ends with the target");
        for opt in [
            "-T",
            "-o",
            "ServerAliveInterval=5",
            "-o",
            "ServerAliveCountMax=2",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=4",
        ] {
            argv.push(opt.into());
        }
        argv.push(target);
        argv.push("--".into());
        argv.push(self.remote_command.clone());
        argv
    }

    /// Spawn ssh and run `round` over its pipes. A watchdog kills ssh when the
    /// hello misses [`Limits::hello`] or the round passes [`Limits::round`].
    pub fn run<T>(
        &self,
        cancel: &Arc<AtomicBool>,
        limits: Limits,
        round: impl FnOnce(&mut Conn) -> anyhow::Result<T>,
    ) -> Result<T, TransportError> {
        let argv = self.argv();
        let mut child = Command::new(self.program.as_deref().unwrap_or(&argv[0]))
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| TransportError::Offline(format!("spawn ssh: {e}")))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let child = Arc::new(Mutex::new(child));

        let stderr_thread = thread::spawn(move || {
            let mut stderr = stderr;
            let mut head = Vec::new();
            let _ = (&mut stderr).take(4096).read_to_end(&mut head);
            let _ = io::copy(&mut stderr, &mut io::sink()); // never block ssh on a full pipe
            String::from_utf8_lossy(&head).into_owned()
        });

        let mut conn = Conn::new(stdout, stdin, limits, cancel.clone());
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let watchdog = {
            let (child, hello_seen) = (child.clone(), conn.hello_flag());
            thread::spawn(move || watch(&child, &hello_seen, limits, &stop_rx))
        };

        let result = round(&mut conn);
        let hello_seen = conn.hello_seen();
        drop(conn); // closes ssh's stdin: the peer sees end of input

        let _ = stop_tx.send(());
        let _ = watchdog.join();
        let status = reap(&child);
        let err = stderr_thread.join().unwrap_or_default();

        result.map_err(|e| {
            let mut reason = format!("{e:#}");
            if let Some(first) = err.lines().next().map(str::trim).filter(|l| !l.is_empty()) {
                reason.push_str(&format!(" | ssh stderr: {first}"));
            }
            if let Some(hint) = host_key_hint(&err) {
                reason.push_str(&format!(" | {hint}"));
            }
            reason.push_str(&format!(" | exit {status:?}"));
            if hello_seen {
                TransportError::Failed(reason)
            } else {
                TransportError::Offline(reason)
            }
        })
    }
}

/// What to tell the user when ssh refused the peer's host key. Sync forces
/// `StrictHostKeyChecking=yes` and `BatchMode`, so nothing can ask the user:
/// an unknown key must fail loudly and say how to fix it.
fn host_key_hint(stderr: &str) -> Option<&'static str> {
    if stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED") {
        Some("its host key has changed: check ~/.ssh/known_hosts before syncing with it")
    } else if stderr.contains("Host key verification failed") {
        Some("connect to this host once to verify its host key")
    } else {
        None
    }
}

fn watch(child: &Mutex<Child>, hello_seen: &AtomicBool, limits: Limits, stop: &Receiver<()>) {
    let kill = || {
        if let Ok(mut c) = child.lock() {
            let _ = c.kill();
        }
    };
    if let Err(RecvTimeoutError::Timeout) = stop.recv_timeout(limits.hello) {
        if !hello_seen.load(Ordering::SeqCst) {
            return kill();
        }
        if let Err(RecvTimeoutError::Timeout) =
            stop.recv_timeout(limits.round.saturating_sub(limits.hello))
        {
            kill();
        }
    }
}

/// Give ssh a moment to exit on its own after its stdin closed, then make sure.
fn reap(child: &Mutex<Child>) -> Option<i32> {
    let Ok(mut c) = child.lock() else { return None };
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until {
        if let Ok(Some(status)) = c.try_wait() {
            return status.code();
        }
        thread::sleep(Duration::from_millis(10));
    }
    let _ = c.kill();
    c.wait().ok().and_then(|s| s.code())
}

// ── Worker ───────────────────────────────────────────────

/// What the UI asks the worker to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncCmd {
    SyncNow(DeviceId),
    EnrollBegin(DeviceId),
    EnrollConfirmSas(bool),
    /// Open these synced credentials (store keys).
    Unlock(Vec<String>),
    /// The user's answer to an unlock prompt: the master phrase or a code.
    UnlockCode(String),
    /// Abandon the running unlock.
    UnlockCancel,
    /// Set up the Vault ring.
    Onboard(Box<OnboardingSecrets>),
    /// The holder's answer to a pending `ShareRelease`: resolve it and, when
    /// approved, append the grants.
    Release {
        approve: bool,
        hash: [u8; 32],
    },
    /// Load the device key and report which device this is.
    Identify,
    /// Run a job with the device identity on the worker.
    Job(JobBox),
    Shutdown,
}

/// What the SAS modal needs: the words, who the peer says it is, and whether
/// a human on the other side sees the same words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SasPromptInfo {
    pub words: String,
    pub peer_name: String,
    pub peer_has_screen: bool,
}

/// What the worker reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncEvent {
    Progress(String),
    PendingArrived(usize),
    SasPrompt(SasPromptInfo),
    /// This device's id.
    ThisDevice(DeviceId),
    /// A job finished: what it did, for the notice line.
    Notice(String),
    /// What the running unlock asks the user for (shown in the unlock modal).
    UnlockPrompt(String),
    /// Where one unlock path stands (`peer device: asked laptop`, `phone: waiting`).
    UnlockPath(UnlockPathKind, String),
    /// Shares collected so far of those needed.
    UnlockProgress {
        have: usize,
        need: usize,
    },
    /// QR frames of a challenge for the phone, looped by the modal.
    UnlockChallenge(Vec<Vec<u8>>),
    ForkDetected(DeviceId),
    BehindPeer(DeviceId),
    Done,
    Error(String),
}

/// Which kind of round the worker runs for a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Round {
    Sync,
    Enroll,
    /// Open synced credentials; the device argument lists their store keys,
    /// one per line.
    Unlock,
    /// Set up the Vault ring from the secrets [`RoundCtx::take_secrets`] holds.
    Onboard,
    /// Answer a release request; the device argument is `<0|1>:<op hash hex>`.
    Release,
    /// Report this device's id.
    Identify,
    /// Run a job with the device identity (admin operations).
    Job,
}

/// A round's window onto the worker: report events, and wait for the UI's
/// answers (the enrollment SAS confirmation).
pub struct RoundCtx<'a> {
    events: &'a Sender<SyncEvent>,
    cmds: &'a Receiver<SyncCmd>,
    cancel: Arc<AtomicBool>,
    secrets: Option<OnboardingSecrets>,
    job: Option<JobBox>,
}

impl RoundCtx<'_> {
    pub fn emit(&self, event: SyncEvent) {
        let _ = self.events.send(event);
    }

    /// A sender for code that outlives a borrow of the context (the unlock
    /// thread), reporting to the same UI.
    pub fn event_sender(&self) -> Sender<SyncEvent> {
        self.events.clone()
    }

    /// The secrets an [`SyncCmd::Onboard`] command carried, once.
    pub fn take_secrets(&mut self) -> Option<OnboardingSecrets> {
        self.secrets.take()
    }

    /// The job a [`SyncCmd::Job`] carried.
    pub fn take_job(&mut self) -> Option<JobBox> {
        self.job.take()
    }

    /// The flag to hand to [`SshTransport::run`]: set when the worker is told
    /// to stop, so a blocked round returns promptly.
    pub fn cancel_flag(&self) -> &Arc<AtomicBool> {
        &self.cancel
    }

    /// The next command from the UI, `None` on timeout or shutdown.
    pub fn recv_cmd(&self, within: Duration) -> Option<SyncCmd> {
        let deadline = Instant::now() + within;
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return None;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            match self.cmds.recv_timeout(left.min(CANCEL_POLL)) {
                Ok(SyncCmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => return None,
                Ok(cmd) => return Some(cmd),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

/// One round with one device; supplied by the protocol package.
pub type RoundFn =
    Box<dyn FnMut(Round, &DeviceId, &mut RoundCtx<'_>) -> anyhow::Result<RoundSummary> + Send>;

/// The UI's end of the worker. Dropping it stops the worker: the cancel flag
/// wakes a blocked round, `Shutdown` is sent, the sender dropped, the thread
/// joined.
pub struct SyncHandle {
    cmd: Option<Sender<SyncCmd>>,
    evt: Receiver<SyncEvent>,
    join: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    stop_reported: AtomicBool,
}

/// Start the worker thread running `round` for each command.
pub fn spawn_sync_thread(mut round: RoundFn) -> SyncHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<SyncCmd>();
    let (evt_tx, evt_rx) = mpsc::channel::<SyncEvent>();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let join = thread::spawn(move || {
        while let Ok(cmd) = cmd_rx.recv() {
            if worker_cancel.load(Ordering::SeqCst) {
                break;
            }
            let mut secrets = None;
            let mut job = None;
            let (kind, device) = match cmd {
                SyncCmd::SyncNow(d) => (Round::Sync, d),
                SyncCmd::EnrollBegin(d) => (Round::Enroll, d),
                SyncCmd::Unlock(keys) => (Round::Unlock, keys.join("\n")),
                SyncCmd::Onboard(s) => {
                    secrets = Some(*s);
                    (Round::Onboard, String::new())
                }
                SyncCmd::Identify => (Round::Identify, String::new()),
                SyncCmd::Job(j) => {
                    job = Some(j);
                    (Round::Job, String::new())
                }
                SyncCmd::Release { approve, hash } => (
                    Round::Release,
                    format!("{}:{}", u8::from(approve), hex::encode(hash)),
                ),
                SyncCmd::EnrollConfirmSas(_) => {
                    let _ = evt_tx.send(SyncEvent::Error(
                        "no enrollment is waiting for a confirmation".into(),
                    ));
                    continue;
                }
                // An answer or a cancel with no unlock waiting for it.
                SyncCmd::UnlockCode(_) | SyncCmd::UnlockCancel => continue,
                SyncCmd::Shutdown => break,
            };
            let mut ctx = RoundCtx {
                events: &evt_tx,
                cmds: &cmd_rx,
                cancel: worker_cancel.clone(),
                secrets,
                job,
            };
            let done = match catch_unwind(AssertUnwindSafe(|| round(kind, &device, &mut ctx))) {
                Ok(Ok(summary)) => {
                    if summary.pending_arrived > 0 {
                        let _ = evt_tx.send(SyncEvent::PendingArrived(summary.pending_arrived));
                    }
                    for id in summary.forks {
                        let _ = evt_tx.send(SyncEvent::ForkDetected(id));
                    }
                    for id in summary.behind {
                        let _ = evt_tx.send(SyncEvent::BehindPeer(id));
                    }
                    // `suspect` is already in the store as a device status:
                    // the view re-reads it when the round is done.
                    SyncEvent::Done
                }
                Ok(Err(e)) => SyncEvent::Error(format!("{e:#}")),
                Err(_) => SyncEvent::Error("sync round panicked".into()),
            };
            let _ = evt_tx.send(done);
        }
    });
    SyncHandle {
        cmd: Some(cmd_tx),
        evt: evt_rx,
        join: Some(join),
        cancel,
        stop_reported: AtomicBool::new(false),
    }
}

impl SyncHandle {
    pub fn send(&self, cmd: SyncCmd) -> Result<(), SendError<SyncCmd>> {
        if cmd == SyncCmd::Shutdown {
            self.cancel.store(true, Ordering::SeqCst);
        }
        match &self.cmd {
            Some(tx) => tx.send(cmd),
            None => Err(SendError(cmd)),
        }
    }

    /// The next event. A worker that is gone surfaces once as
    /// `Error("sync worker stopped")`, then as `Disconnected`.
    pub fn try_recv(&self) -> Result<SyncEvent, TryRecvError> {
        match self.evt.try_recv() {
            Err(TryRecvError::Disconnected) if !self.stop_reported.swap(true, Ordering::SeqCst) => {
                Ok(SyncEvent::Error(WORKER_STOPPED.into()))
            }
            other => other,
        }
    }
}

impl Drop for SyncHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(tx) = self.cmd.take() {
            let _ = tx.send(SyncCmd::Shutdown);
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::transport::{read_frame, write_frame};
    use serde_json::json;
    use std::os::unix::net::UnixStream;

    fn sample_sas() -> SasPromptInfo {
        SasPromptInfo {
            words: "harbor-globe-lantern-mint".into(),
            peer_name: "laptop".into(),
            peer_has_screen: true,
        }
    }

    /// A connection to an in-process peer; the other end is returned raw.
    fn pair(limits: Limits) -> (Conn, UnixStream, Arc<AtomicBool>) {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let conn = Conn::new(ours.try_clone().unwrap(), ours, limits, cancel.clone());
        (conn, theirs, cancel)
    }

    fn hello_round(peer_pending: u64) -> RoundFn {
        Box::new(move |_kind, device, _ctx| {
            let (mut conn, mut peer, _cancel) = pair(Limits::default());
            let want = device.clone();
            let t = thread::spawn(move || {
                let got = read_frame(&mut peer).unwrap().unwrap();
                assert_eq!(got["to"], json!(want));
                write_frame(
                    &mut peer,
                    &json!({"type": "hello", "pending": peer_pending}),
                )
                .unwrap();
            });
            conn.send(&json!({"type": "hello", "to": device}))?;
            let hello = conn.recv_hello()?;
            t.join().unwrap();
            Ok(RoundSummary {
                pending_arrived: hello["pending"].as_u64().unwrap() as usize,
                ..Default::default()
            })
        })
    }

    fn collect(handle: &SyncHandle, until: impl Fn(&SyncEvent) -> bool) -> Vec<SyncEvent> {
        let mut seen = Vec::new();
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            match handle.try_recv() {
                Ok(e) => {
                    let stop = until(&e);
                    seen.push(e);
                    if stop {
                        return seen;
                    }
                }
                Err(_) => thread::sleep(Duration::from_millis(5)),
            }
        }
        panic!("no terminal event, saw {seen:?}");
    }

    fn terminal(e: &SyncEvent) -> bool {
        matches!(e, SyncEvent::Done | SyncEvent::Error(_))
    }

    #[test]
    fn argv_forces_batch_mode_and_connect_timeout_after_every_other_option() {
        let mut host = SshHost::new("peer");
        host.hostname = Some("peer.example".into());
        host.user = Some("me".into());
        host.port = Some(2222);
        host.identity_file = Some("/k/id".into());
        host.certificate_file = Some("/k/id-cert.pub".into());
        host.proxy_jump = Some("jump".into());
        host.forward_agent = Some(true);
        host.remote_command = Some("ignored".into());
        let argv = SshTransport::new(host, "sshub sync-serve").argv();

        let at = |s: &str| {
            argv.iter()
                .position(|a| a == s)
                .unwrap_or_else(|| panic!("{s}: {argv:?}"))
        };
        let (batch, connect, target) = (
            at("BatchMode=yes"),
            at("ConnectTimeout=4"),
            at("me@peer.example"),
        );
        for other in ["ForwardAgent=yes", "CertificateFile=/k/id-cert.pub"] {
            assert!(
                at(other) < batch && at(other) < connect,
                "{other} must come first: {argv:?}"
            );
        }
        assert!(at("-i") < batch && at("-p") < batch && at("-J") < batch);
        assert!(batch < target && connect < target);
        assert_eq!(
            connect,
            batch + 2,
            "the two forced options sit together, last"
        );
        assert_eq!(&argv[argv.len() - 2..], ["--", "sshub sync-serve"]);
        assert!(
            !argv.iter().any(|a| a == "ignored"),
            "the host's own remote command is replaced"
        );
        assert!(argv.contains(&"-T".to_string()));
        assert!(!argv.iter().any(|a| a.starts_with("BatchMode=no")));
    }

    #[test]
    fn the_worker_runs_a_round_and_reports_pending_then_done() {
        let handle = spawn_sync_thread(hello_round(2));
        handle.send(SyncCmd::SyncNow("laptop".into())).unwrap();
        let seen = collect(&handle, terminal);
        assert_eq!(seen, vec![SyncEvent::PendingArrived(2), SyncEvent::Done]);
    }

    #[test]
    fn a_rounds_forks_and_behind_peers_become_events() {
        let handle = spawn_sync_thread(Box::new(|_, _, _| {
            Ok(RoundSummary {
                pending_arrived: 1,
                forks: vec!["evil".into()],
                behind: vec!["slow".into()],
                suspect: vec!["odd".into()],
                ..Default::default()
            })
        }));
        handle.send(SyncCmd::SyncNow("laptop".into())).unwrap();
        let seen = collect(&handle, terminal);
        assert_eq!(
            seen,
            vec![
                SyncEvent::PendingArrived(1),
                SyncEvent::ForkDetected("evil".into()),
                SyncEvent::BehindPeer("slow".into()),
                SyncEvent::Done,
            ]
        );
    }

    #[test]
    fn a_failing_or_panicking_round_is_reported_and_the_worker_carries_on() {
        let mut calls = 0;
        let handle = spawn_sync_thread(Box::new(move |_k, _d, _ctx| {
            calls += 1;
            match calls {
                1 => anyhow::bail!("peer said no"),
                2 => panic!("boom"),
                _ => Ok(RoundSummary::default()),
            }
        }));
        for expect in [
            SyncEvent::Error("peer said no".into()),
            SyncEvent::Error("sync round panicked".into()),
            SyncEvent::Done,
        ] {
            handle.send(SyncCmd::SyncNow("d".into())).unwrap();
            assert_eq!(collect(&handle, terminal).pop(), Some(expect));
        }
    }

    #[test]
    fn enrollment_waits_for_the_sas_confirmation_from_the_ui() {
        let handle = spawn_sync_thread(Box::new(|kind, device, ctx| {
            assert_eq!((kind, device.as_str()), (Round::Enroll, "new"));
            ctx.emit(SyncEvent::SasPrompt(sample_sas()));
            match ctx.recv_cmd(Duration::from_secs(5)) {
                Some(SyncCmd::EnrollConfirmSas(true)) => Ok(RoundSummary::default()),
                other => anyhow::bail!("not confirmed: {other:?}"),
            }
        }));
        handle.send(SyncCmd::EnrollBegin("new".into())).unwrap();
        let first = collect(&handle, |e| matches!(e, SyncEvent::SasPrompt(_)));
        assert_eq!(first, vec![SyncEvent::SasPrompt(sample_sas())]);
        handle.send(SyncCmd::EnrollConfirmSas(true)).unwrap();
        assert_eq!(collect(&handle, terminal), vec![SyncEvent::Done]);
    }

    #[test]
    fn a_confirmation_with_no_enrollment_is_an_error_not_a_hang() {
        let handle = spawn_sync_thread(Box::new(|_, _, _| Ok(RoundSummary::default())));
        handle.send(SyncCmd::EnrollConfirmSas(true)).unwrap();
        let seen = collect(&handle, terminal);
        assert!(matches!(seen.last(), Some(SyncEvent::Error(m)) if m.contains("no enrollment")));
    }

    #[test]
    fn dropping_the_handle_joins_even_while_a_round_is_blocked_on_the_peer() {
        let handle = spawn_sync_thread(Box::new(|_k, _d, ctx| {
            let (ours, _silent_peer) = UnixStream::pair()?;
            let mut conn = Conn::new(
                ours.try_clone()?,
                ours,
                Limits::default(),
                ctx.cancel_flag().clone(),
            );
            conn.recv()?; // the peer never answers
            Ok(RoundSummary::default())
        }));
        handle.send(SyncCmd::SyncNow("d".into())).unwrap();
        thread::sleep(Duration::from_millis(200)); // the round is now blocked
        let start = Instant::now();
        drop(handle);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn a_dead_worker_surfaces_once_as_an_error_event() {
        let (cmd_tx, _cmd_rx) = mpsc::channel();
        let (evt_tx, evt_rx) = mpsc::channel::<SyncEvent>();
        let handle = SyncHandle {
            cmd: Some(cmd_tx),
            evt: evt_rx,
            join: None,
            cancel: Arc::new(AtomicBool::new(false)),
            stop_reported: AtomicBool::new(false),
        };
        assert_eq!(handle.try_recv(), Err(TryRecvError::Empty));
        drop(evt_tx);
        assert_eq!(
            handle.try_recv(),
            Ok(SyncEvent::Error("sync worker stopped".into()))
        );
        assert_eq!(handle.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn a_worker_that_exits_on_shutdown_reports_it() {
        let handle = spawn_sync_thread(Box::new(|_, _, _| Ok(RoundSummary::default())));
        handle.send(SyncCmd::Shutdown).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            match handle.try_recv() {
                Ok(SyncEvent::Error(m)) => {
                    assert_eq!(m, "sync worker stopped");
                    break;
                }
                _ if Instant::now() > end => panic!("never reported"),
                _ => thread::sleep(Duration::from_millis(5)),
            }
        }
        assert!(
            handle.send(SyncCmd::SyncNow("d".into())).is_err(),
            "nobody is listening"
        );
    }

    #[test]
    fn the_host_key_hint_covers_an_unknown_and_a_changed_key() {
        let unknown = "No ED25519 host key is known for peer and you have requested strict checking.\nHost key verification failed.\n";
        assert_eq!(
            host_key_hint(unknown),
            Some("connect to this host once to verify its host key")
        );
        let changed = "@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@\nHost key verification failed.\n";
        assert!(host_key_hint(changed).unwrap().contains("changed"));
        assert_eq!(host_key_hint("Permission denied (publickey)."), None);
    }

    #[test]
    fn an_unknown_host_key_fails_loudly_and_the_fix_reaches_the_sync_view() {
        use std::os::unix::fs::PermissionsExt;
        // An `ssh` that refuses the way OpenSSH does under strict checking.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("ssh");
        std::fs::write(
            &script,
            "#!/bin/sh\necho 'No ED25519 host key is known for peer and you have requested strict checking.' >&2\necho 'Host key verification failed.' >&2\nexit 255\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let program = script.to_string_lossy().into_owned();

        let mut app = crate::app::tests::test_app(vec![]);
        app.attach_sync_worker(spawn_sync_thread(Box::new(move |_k, _d, ctx| {
            let mut host = SshHost::new("peer");
            host.hostname = Some("peer.example".into());
            let transport =
                SshTransport::new(host, "sshub sync-serve").with_program(program.clone());
            transport.run(ctx.cancel_flag(), Limits::default(), |conn| {
                conn.recv_hello_msg::<serde_json::Value>()?;
                Ok(RoundSummary::default())
            })?;
            Ok(RoundSummary::default())
        })));
        app.sync
            .intents
            .push(crate::app::sync::SyncIntent::SyncNow {
                device: "peer".into(),
            });
        let end = Instant::now() + Duration::from_secs(10);
        while app.sync.last_error.is_none() {
            assert!(Instant::now() < end, "no error reached the view");
            app.poll_sync();
            thread::sleep(Duration::from_millis(10));
        }
        let error = app.sync.last_error.clone().unwrap();
        assert!(error.starts_with("offline:"), "no hello: {error}");
        assert!(
            error.contains("connect to this host once to verify its host key"),
            "{error}"
        );
        assert!(app
            .host_notice
            .as_deref()
            .is_some_and(|n| n.contains("verify its host key")));
    }
}
