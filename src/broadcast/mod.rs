//! Broadcast mode (#3): run one command across many hosts concurrently.
//!
//! This module is **pure** — no `App`, no TUI dependency. It owns the shared
//! value types ([`BroadcastTask`], [`HostState`], [`HostResult`],
//! [`BroadcastEvent`]), the [`CommandRunner`] transport seam, a real
//! [`SshCommandRunner`] (mirroring [`crate::osinfo::detect::SshProbeRunner`]'s
//! arg splice), a bounded worker pool ([`spawn_broadcast`]), and the pure
//! reducer / view helpers the UI layer drives.
//!
//! The pool rides the crate's established thread + mpsc worker shape (see
//! [`crate::ping`], [`crate::sftp::worker`]): `K = min(concurrency, tasks)`
//! worker threads pull [`BroadcastTask`]s off a shared
//! `Arc<Mutex<Receiver<BroadcastTask>>>` and emit [`BroadcastEvent`]s on a
//! single `Sender`. The UI drains those events in the poll loop and folds them
//! into a `Vec<HostResult>` via [`apply_event`].

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Default bounded pool width.
pub const DEFAULT_CONCURRENCY: usize = 8;
/// Countdown length before a finished panel auto-dismisses.
pub const DISMISS: Duration = Duration::from_millis(6500);
/// Entry slide duration (center -> corner).
pub const ENTRY_ANIM: Duration = Duration::from_millis(600);
/// Per-host ssh connect timeout (seconds) and run cap.
pub const CONNECT_TIMEOUT_SECS: u32 = 8;
pub const RUN_TIMEOUT: Duration = Duration::from_secs(60);

/// How often the [`SshCommandRunner`] poll loop wakes to check `cancel` /
/// run-timeout / child exit while a remote command is in flight.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// One host to run the broadcast command against.
#[derive(Debug, Clone)]
pub struct BroadcastTask {
    pub host_id: i64,
    pub host_name: String,
    /// `ssh_argv_for_entry(entry)`; `argv[0] == "ssh"`.
    pub argv: Vec<String>,
    /// Stored credential answered via `SSH_ASKPASS` (same path a live session
    /// uses). `Some` => `BatchMode=no`; `None` => key/agent only (`BatchMode=yes`,
    /// fail fast on a password prompt).
    pub secret: Option<crate::session::PendingSecret>,
}

/// Per-host lifecycle state, folded from [`BroadcastEvent`]s by [`apply_event`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostState {
    Pending,
    Running,
    Done { exit: i32 },
    Failed { reason: String },
}

impl HostState {
    /// Terminal = `Done` | `Failed`.
    pub fn is_terminal(&self) -> bool {
        matches!(self, HostState::Done { .. } | HostState::Failed { .. })
    }
}

/// One row in the aggregated result table.
#[derive(Debug, Clone)]
pub struct HostResult {
    pub host_id: i64,
    pub host_name: String,
    pub state: HostState,
    pub stdout: String,
    pub stderr: String,
}

/// Event emitted by a pool worker for one task. Exactly one *terminal* event
/// (`Finished` or `Failed`) is emitted per task; `Started` is optional/leading.
#[derive(Debug, Clone)]
pub enum BroadcastEvent {
    Started {
        host_id: i64,
    },
    Finished {
        host_id: i64,
        exit: i32,
        stdout: String,
        stderr: String,
    },
    Failed {
        host_id: i64,
        reason: String,
    },
}

/// How a broadcast command is executed against one host. Abstracted (like
/// [`crate::osinfo::detect::ProbeRunner`]) so tests inject canned output without
/// spawning `ssh`.
pub trait CommandRunner: Send + Sync {
    /// Run `argv` + the remote `command`; honor `cancel` and the run timeout.
    /// `Ok((exit, stdout, stderr))` when the process completed (any exit code),
    /// `Err(reason)` when it could not (spawn failure, timeout, cancel).
    fn run(
        &self,
        argv: &[String],
        command: &str,
        secret: Option<&crate::session::PendingSecret>,
        cancel: &AtomicBool,
    ) -> Result<(i32, String, String), String>;
}

/// Real runner: shells out to `ssh` with the same non-interactive option splice
/// as [`crate::osinfo::detect::SshProbeRunner`]
/// (`-o BatchMode=yes -o ConnectTimeout=N -o StrictHostKeyChecking=accept-new`
/// after `argv[0]`, the remote command appended as the final argv). Pipes
/// stdout/stderr, enforces `run_timeout` + `cancel` via a `try_wait` poll loop
/// that kills the child (reader threads joined after the kill). Key/agent only
/// — no `SSH_ASKPASS` in v1, so `BatchMode=yes` makes a password-needing host
/// fail fast instead of blocking on `/dev/tty`.
pub struct SshCommandRunner {
    pub connect_timeout_secs: u32,
    pub run_timeout: Duration,
}

impl SshCommandRunner {
    pub fn new() -> Self {
        Self {
            connect_timeout_secs: CONNECT_TIMEOUT_SECS,
            run_timeout: RUN_TIMEOUT,
        }
    }
}

impl Default for SshCommandRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRunner for SshCommandRunner {
    fn run(
        &self,
        argv: &[String],
        command: &str,
        secret: Option<&crate::session::PendingSecret>,
        cancel: &AtomicBool,
    ) -> Result<(i32, String, String), String> {
        if argv.is_empty() {
            return Err("empty ssh argv".to_string());
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".to_string());
        }

        // Splice non-interactive options right after the program name and append
        // the remote command as the final argument. With a stored secret we run
        // BatchMode=no and answer the prompt via SSH_ASKPASS (exactly like a live
        // session); without one, BatchMode=yes so ssh fails fast instead of
        // blocking on a /dev/tty password prompt (ConnectTimeout only bounds the
        // TCP connect).
        let batchmode = if secret.is_some() {
            "BatchMode=no"
        } else {
            "BatchMode=yes"
        };
        let connect_timeout = format!("ConnectTimeout={}", self.connect_timeout_secs);
        let mut full: Vec<String> = Vec::with_capacity(argv.len() + 8);
        full.push(argv[0].clone());
        for opt in [
            "-o",
            batchmode,
            "-o",
            &connect_timeout,
            "-o",
            "StrictHostKeyChecking=accept-new",
        ] {
            full.push(opt.to_string());
        }
        full.extend(argv[1..].iter().cloned());
        full.push(command.to_string());

        let mut cmd = Command::new(&full[0]);
        cmd.args(&full[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Stage the stored secret through SSH_ASKPASS. The guard owns an
        // owner-only temp file removed on drop, so keep it alive until the child
        // has exited (i.e. for the whole poll loop below).
        let mut _askpass = None;
        if let Some(secret) = secret {
            if let Ok(exe) = std::env::current_exe() {
                if let Ok(guard) = crate::session::askpass::AskpassSecret::new(secret.value()) {
                    for (k, v) in guard.env(&exe) {
                        cmd.env(k, v);
                    }
                    _askpass = Some(guard);
                }
            }
        }

        let mut child = cmd.spawn().map_err(|e| format!("spawn ssh: {e}"))?;

        // Drain both pipes on their own threads so a chatty host can't deadlock
        // by filling a pipe buffer while we block on the other.
        let stdout_reader = child.stdout.take().map(spawn_pipe_reader);
        let stderr_reader = child.stderr.take().map(spawn_pipe_reader);

        // Poll for exit, honoring cancel + run timeout by killing the child.
        let start = Instant::now();
        let mut fail_reason: Option<String> = None;
        let exit_status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(e) => {
                    fail_reason = Some(format!("wait ssh: {e}"));
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
            if cancel.load(Ordering::Relaxed) {
                fail_reason = Some("cancelled".to_string());
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            if start.elapsed() >= self.run_timeout {
                fail_reason = Some("timed out".to_string());
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            thread::sleep(POLL_INTERVAL);
        };

        // Killing the child closes the pipes, so the reader threads finish.
        let stdout = stdout_reader.map(join_pipe_reader).unwrap_or_default();
        let stderr = stderr_reader.map(join_pipe_reader).unwrap_or_default();

        if let Some(reason) = fail_reason {
            return Err(reason);
        }
        // `exit_status` is Some here (the only None paths set `fail_reason`).
        let exit = exit_status.and_then(|s| s.code()).unwrap_or(-1);
        Ok((exit, stdout, stderr))
    }
}

/// Spawn a thread that reads a child pipe to EOF into a `String`
/// (lossy UTF-8), returning its join handle.
fn spawn_pipe_reader<R: Read + Send + 'static>(mut pipe: R) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// Join a pipe-reader thread, tolerating a panicked reader (empty string).
fn join_pipe_reader(handle: thread::JoinHandle<String>) -> String {
    handle.join().unwrap_or_default()
}

/// Seed one `Pending` row per task, in task order (stable per-target order).
pub fn seed_results(tasks: &[BroadcastTask]) -> Vec<HostResult> {
    tasks
        .iter()
        .map(|t| HostResult {
            host_id: t.host_id,
            host_name: t.host_name.clone(),
            state: HostState::Pending,
            stdout: String::new(),
            stderr: String::new(),
        })
        .collect()
}

/// Bounded pool: `min(concurrency, tasks.len())` worker threads pull tasks from
/// a shared `Arc<Mutex<Receiver<BroadcastTask>>>`, each emitting events on one
/// `Sender<BroadcastEvent>`.
///
/// GUARANTEE: exactly one terminal event (`Finished` OR `Failed`) per task,
/// including under cancel — un-started tasks emit `Failed{reason:"cancelled"}`
/// and in-flight children are killed → `Failed`.
pub fn spawn_broadcast(
    tasks: Vec<BroadcastTask>,
    command: String,
    concurrency: usize,
    cancel: Arc<AtomicBool>,
    runner: Arc<dyn CommandRunner>,
) -> Receiver<BroadcastEvent> {
    let (event_tx, event_rx) = mpsc::channel::<BroadcastEvent>();

    let n_workers = concurrency.max(1).min(tasks.len());
    if n_workers == 0 {
        // No tasks: return an already-exhausted receiver (event_tx drops here).
        return event_rx;
    }

    // Preload every task into a channel, drop the sender: workers then get each
    // task once, and an empty+disconnected `recv()` (never blocks) ends them.
    let (task_tx, task_rx) = mpsc::channel::<BroadcastTask>();
    for task in tasks {
        // Send into an unbounded channel with a live receiver: cannot fail.
        let _ = task_tx.send(task);
    }
    drop(task_tx);
    let shared_rx = Arc::new(Mutex::new(task_rx));

    for _ in 0..n_workers {
        let shared_rx = Arc::clone(&shared_rx);
        let event_tx = event_tx.clone();
        let cancel = Arc::clone(&cancel);
        let runner = Arc::clone(&runner);
        let command = command.clone();
        thread::spawn(move || {
            broadcast_worker(&shared_rx, &event_tx, &cancel, runner.as_ref(), &command);
        });
    }
    // Drop our own handle so the channel closes once every worker exits.
    drop(event_tx);

    event_rx
}

/// One pool worker: pull tasks until the shared queue is empty, running each
/// (or short-circuiting to a `cancelled` failure once the flag is set).
fn broadcast_worker(
    shared_rx: &Arc<Mutex<Receiver<BroadcastTask>>>,
    event_tx: &Sender<BroadcastEvent>,
    cancel: &AtomicBool,
    runner: &dyn CommandRunner,
    command: &str,
) {
    loop {
        // Pull one task. `recv()` never blocks here: the sender was dropped, so
        // it returns immediately with a task or an Err (empty + disconnected).
        let task = {
            let guard = shared_rx.lock().unwrap_or_else(|e| e.into_inner());
            guard.recv()
        };
        let Ok(task) = task else {
            return; // queue drained
        };

        // Once cancelled, drain the rest of the queue marking each cancelled —
        // this keeps the one-terminal-event-per-task guarantee.
        if cancel.load(Ordering::Relaxed) {
            let ev = BroadcastEvent::Failed {
                host_id: task.host_id,
                reason: "cancelled".to_string(),
            };
            if event_tx.send(ev).is_err() {
                return; // UI gone
            }
            continue;
        }

        if event_tx
            .send(BroadcastEvent::Started {
                host_id: task.host_id,
            })
            .is_err()
        {
            return;
        }

        let ev = match runner.run(&task.argv, command, task.secret.as_ref(), cancel) {
            Ok((exit, stdout, stderr)) => BroadcastEvent::Finished {
                host_id: task.host_id,
                exit,
                stdout,
                stderr,
            },
            Err(reason) => BroadcastEvent::Failed {
                host_id: task.host_id,
                reason,
            },
        };
        if event_tx.send(ev).is_err() {
            return;
        }
    }
}

/// Pure reducer: mutate the matching row (by `host_id`) in place. Length +
/// order unchanged. `Started`→`Running`, `Finished`→`Done{exit}`+stdout/stderr,
/// `Failed`→`Failed{reason}`. A row already terminal is not resurrected by a
/// stray `Started`.
pub fn apply_event(results: &mut [HostResult], event: &BroadcastEvent) {
    match event {
        BroadcastEvent::Started { host_id } => {
            if let Some(row) = results.iter_mut().find(|r| r.host_id == *host_id) {
                if !row.state.is_terminal() {
                    row.state = HostState::Running;
                }
            }
        }
        BroadcastEvent::Finished {
            host_id,
            exit,
            stdout,
            stderr,
        } => {
            if let Some(row) = results.iter_mut().find(|r| r.host_id == *host_id) {
                row.state = HostState::Done { exit: *exit };
                row.stdout = stdout.clone();
                row.stderr = stderr.clone();
            }
        }
        BroadcastEvent::Failed { host_id, reason } => {
            if let Some(row) = results.iter_mut().find(|r| r.host_id == *host_id) {
                row.state = HostState::Failed {
                    reason: reason.clone(),
                };
            }
        }
    }
}

/// True once every row is terminal.
pub fn all_terminal(results: &[HostResult]) -> bool {
    results.iter().all(|r| r.state.is_terminal())
}

/// Render-order rank: Failed(0) < Running(1) < Pending(2) < Done(3), so
/// failures surface first and unfinished work sits above completed rows.
fn state_rank(state: &HostState) -> u8 {
    match state {
        HostState::Failed { .. } => 0,
        HostState::Running => 1,
        HostState::Pending => 2,
        HostState::Done { .. } => 3,
    }
}

/// Render-time view ordering (does NOT mutate `results`): indices into
/// `results` sorted by [`state_rank`] via a STABLE sort, so ties keep input
/// order.
pub fn failures_first(results: &[HostResult]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..results.len()).collect();
    idx.sort_by_key(|&i| state_rank(&results[i].state));
    idx
}

/// Count of `Done` + `Failed` for the "N/total" header badge.
pub fn done_count(results: &[HostResult]) -> usize {
    results.iter().filter(|r| r.state.is_terminal()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn task(id: i64, name: &str) -> BroadcastTask {
        BroadcastTask {
            host_id: id,
            host_name: name.to_string(),
            argv: vec!["ssh".to_string(), name.to_string()],
            secret: None,
        }
    }

    /// Fake runner that tracks live concurrency (peak) and can be told to fail
    /// specific hosts. Sleeps briefly so overlap is observable.
    struct FakeRunner {
        live: AtomicUsize,
        peak: AtomicUsize,
        fail_ids: Vec<i64>,
        sleep: Duration,
        started: AtomicUsize,
    }

    impl FakeRunner {
        fn new() -> Self {
            Self {
                live: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                fail_ids: Vec::new(),
                sleep: Duration::from_millis(20),
                started: AtomicUsize::new(0),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            argv: &[String],
            command: &str,
            _secret: Option<&crate::session::PendingSecret>,
            cancel: &AtomicBool,
        ) -> Result<(i32, String, String), String> {
            self.started.fetch_add(1, Ordering::SeqCst);
            let now = self.live.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            thread::sleep(self.sleep);
            self.live.fetch_sub(1, Ordering::SeqCst);

            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".to_string());
            }
            // host_id is not passed to run(); key the failure off argv (the
            // host name we baked in as argv[1]).
            let name = argv.get(1).cloned().unwrap_or_default();
            if self.fail_ids.iter().any(|id| name == id.to_string()) {
                return Err(format!("boom {name}"));
            }
            Ok((0, format!("out:{name}:{command}"), String::new()))
        }
    }

    /// Collect every event from a receiver until the pool's senders all drop.
    fn drain(rx: Receiver<BroadcastEvent>) -> Vec<BroadcastEvent> {
        rx.into_iter().collect()
    }

    #[test]
    fn is_terminal_classifies_states() {
        assert!(!HostState::Pending.is_terminal());
        assert!(!HostState::Running.is_terminal());
        assert!(HostState::Done { exit: 0 }.is_terminal());
        assert!(HostState::Failed { reason: "x".into() }.is_terminal());
    }

    #[test]
    fn seed_results_one_pending_per_task_in_order() {
        let tasks = vec![task(1, "a"), task(2, "b"), task(3, "c")];
        let seeded = seed_results(&tasks);
        assert_eq!(seeded.len(), 3);
        assert_eq!(
            seeded.iter().map(|r| r.host_id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(seeded.iter().all(|r| r.state == HostState::Pending));
    }

    #[test]
    fn pool_bounds_concurrency() {
        let tasks = (0..12).map(|i| task(i, &i.to_string())).collect::<Vec<_>>();
        let runner = Arc::new(FakeRunner::new());
        let cancel = Arc::new(AtomicBool::new(false));
        let rx = spawn_broadcast(
            tasks,
            "uptime".to_string(),
            3,
            Arc::clone(&cancel),
            Arc::clone(&runner) as Arc<dyn CommandRunner>,
        );
        let events = drain(rx);
        // Every task started and finished.
        assert_eq!(runner.started.load(Ordering::SeqCst), 12);
        assert!(runner.peak.load(Ordering::SeqCst) <= 3, "peak must be <= 3");
        // 12 Started + 12 terminal.
        let terminal = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    BroadcastEvent::Finished { .. } | BroadcastEvent::Failed { .. }
                )
            })
            .count();
        assert_eq!(terminal, 12);
    }

    #[test]
    fn concurrency_clamped_to_task_count() {
        // concurrency > tasks: still one terminal per task, no panic.
        let tasks = vec![task(1, "1"), task(2, "2")];
        let runner = Arc::new(FakeRunner::new());
        let cancel = Arc::new(AtomicBool::new(false));
        let rx = spawn_broadcast(
            tasks,
            "id".to_string(),
            8,
            cancel,
            runner as Arc<dyn CommandRunner>,
        );
        let terminal = drain(rx)
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    BroadcastEvent::Finished { .. } | BroadcastEvent::Failed { .. }
                )
            })
            .count();
        assert_eq!(terminal, 2);
    }

    #[test]
    fn empty_tasks_yields_no_events() {
        let runner = Arc::new(FakeRunner::new());
        let cancel = Arc::new(AtomicBool::new(false));
        let rx = spawn_broadcast(
            Vec::new(),
            "x".to_string(),
            8,
            cancel,
            runner as Arc<dyn CommandRunner>,
        );
        assert!(drain(rx).is_empty());
    }

    #[test]
    fn exactly_one_terminal_event_per_task() {
        let tasks = (0..6).map(|i| task(i, &i.to_string())).collect::<Vec<_>>();
        let mut runner = FakeRunner::new();
        runner.fail_ids = vec![2, 4];
        let runner = Arc::new(runner);
        let cancel = Arc::new(AtomicBool::new(false));
        let rx = spawn_broadcast(
            tasks,
            "cmd".to_string(),
            4,
            cancel,
            runner as Arc<dyn CommandRunner>,
        );
        let events = drain(rx);

        // Each host id has exactly one terminal event.
        for id in 0..6 {
            let terminal = events
                .iter()
                .filter(|e| match e {
                    BroadcastEvent::Finished { host_id, .. } => *host_id == id,
                    BroadcastEvent::Failed { host_id, .. } => *host_id == id,
                    _ => false,
                })
                .count();
            assert_eq!(terminal, 1, "host {id} must have one terminal event");
        }
        // The two failing hosts reported Failed, not Finished.
        for id in [2, 4] {
            assert!(events.iter().any(|e| matches!(
                e,
                BroadcastEvent::Failed { host_id, reason } if *host_id == id && reason.starts_with("boom")
            )));
        }
    }

    #[test]
    fn cancel_before_start_marks_all_terminal() {
        let tasks = (0..8).map(|i| task(i, &i.to_string())).collect::<Vec<_>>();
        let runner = Arc::new(FakeRunner::new());
        let cancel = Arc::new(AtomicBool::new(true)); // cancelled from the start
        let rx = spawn_broadcast(
            tasks,
            "cmd".to_string(),
            2,
            Arc::clone(&cancel),
            runner as Arc<dyn CommandRunner>,
        );
        let events = drain(rx);

        // No task actually ran; every one produced a terminal Failed(cancelled).
        for id in 0..8 {
            let terminal = events
                .iter()
                .filter(|e| matches!(e, BroadcastEvent::Failed { host_id, .. } if *host_id == id))
                .count();
            assert_eq!(terminal, 1);
        }
        assert!(!events
            .iter()
            .any(|e| matches!(e, BroadcastEvent::Finished { .. })));
    }

    #[test]
    fn cancel_reflected_across_all_rows_via_reducer() {
        // Fold cancel events through seed + apply_event → every row terminal.
        let tasks = (0..5).map(|i| task(i, &i.to_string())).collect::<Vec<_>>();
        let runner = Arc::new(FakeRunner::new());
        let cancel = Arc::new(AtomicBool::new(true));
        let mut results = seed_results(&tasks);
        let rx = spawn_broadcast(
            tasks,
            "cmd".to_string(),
            3,
            cancel,
            runner as Arc<dyn CommandRunner>,
        );
        for ev in drain(rx).iter() {
            apply_event(&mut results, ev);
        }
        assert!(all_terminal(&results));
        assert_eq!(done_count(&results), 5);
    }

    #[test]
    fn failures_reported_not_panicked() {
        // A runner that always errors must not bring down the pool; every row
        // ends Failed.
        struct AlwaysErr;
        impl CommandRunner for AlwaysErr {
            fn run(
                &self,
                _argv: &[String],
                _command: &str,
                _secret: Option<&crate::session::PendingSecret>,
                _cancel: &AtomicBool,
            ) -> Result<(i32, String, String), String> {
                Err("nope".to_string())
            }
        }
        let tasks = (0..4).map(|i| task(i, &i.to_string())).collect::<Vec<_>>();
        let cancel = Arc::new(AtomicBool::new(false));
        let rx = spawn_broadcast(
            tasks,
            "cmd".to_string(),
            2,
            cancel,
            Arc::new(AlwaysErr) as Arc<dyn CommandRunner>,
        );
        let events = drain(rx);
        let failed = events
            .iter()
            .filter(|e| matches!(e, BroadcastEvent::Failed { .. }))
            .count();
        assert_eq!(failed, 4);
        assert!(!events
            .iter()
            .any(|e| matches!(e, BroadcastEvent::Finished { .. })));
    }

    #[test]
    fn apply_event_transitions_the_right_row_in_place() {
        let tasks = vec![task(10, "a"), task(20, "b"), task(30, "c")];
        let mut results = seed_results(&tasks);

        apply_event(&mut results, &BroadcastEvent::Started { host_id: 20 });
        assert_eq!(results[0].state, HostState::Pending);
        assert_eq!(results[1].state, HostState::Running);
        assert_eq!(results[2].state, HostState::Pending);

        apply_event(
            &mut results,
            &BroadcastEvent::Finished {
                host_id: 20,
                exit: 0,
                stdout: "hi".into(),
                stderr: String::new(),
            },
        );
        assert_eq!(results[1].state, HostState::Done { exit: 0 });
        assert_eq!(results[1].stdout, "hi");

        apply_event(
            &mut results,
            &BroadcastEvent::Failed {
                host_id: 30,
                reason: "boom".into(),
            },
        );
        assert_eq!(
            results[2].state,
            HostState::Failed {
                reason: "boom".into()
            }
        );

        // Length + order preserved; ids untouched.
        assert_eq!(
            results.iter().map(|r| r.host_id).collect::<Vec<_>>(),
            vec![10, 20, 30]
        );
    }

    #[test]
    fn apply_event_does_not_resurrect_terminal_row() {
        let tasks = vec![task(1, "a")];
        let mut results = seed_results(&tasks);
        apply_event(
            &mut results,
            &BroadcastEvent::Failed {
                host_id: 1,
                reason: "dead".into(),
            },
        );
        // A late/stray Started must not flip a Failed row back to Running.
        apply_event(&mut results, &BroadcastEvent::Started { host_id: 1 });
        assert_eq!(
            results[0].state,
            HostState::Failed {
                reason: "dead".into()
            }
        );
    }

    #[test]
    fn apply_event_ignores_unknown_host_id() {
        let tasks = vec![task(1, "a")];
        let mut results = seed_results(&tasks);
        apply_event(&mut results, &BroadcastEvent::Started { host_id: 999 });
        assert_eq!(results[0].state, HostState::Pending);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn failures_first_orders_by_rank_stably() {
        let mk = |id: i64, state: HostState| HostResult {
            host_id: id,
            host_name: id.to_string(),
            state,
            stdout: String::new(),
            stderr: String::new(),
        };
        let results = vec![
            mk(1, HostState::Done { exit: 0 }),
            mk(2, HostState::Running),
            mk(3, HostState::Failed { reason: "x".into() }),
            mk(4, HostState::Pending),
            mk(5, HostState::Failed { reason: "y".into() }),
            mk(6, HostState::Done { exit: 1 }),
        ];
        let order = failures_first(&results);
        let ids: Vec<i64> = order.iter().map(|&i| results[i].host_id).collect();
        // Failed(3,5) < Running(2) < Pending(4) < Done(1,6); ties keep input order.
        assert_eq!(ids, vec![3, 5, 2, 4, 1, 6]);
    }

    #[test]
    fn all_terminal_and_done_count() {
        let tasks = vec![task(1, "a"), task(2, "b")];
        let mut results = seed_results(&tasks);
        assert!(!all_terminal(&results));
        assert_eq!(done_count(&results), 0);

        apply_event(
            &mut results,
            &BroadcastEvent::Finished {
                host_id: 1,
                exit: 0,
                stdout: String::new(),
                stderr: String::new(),
            },
        );
        assert!(!all_terminal(&results));
        assert_eq!(done_count(&results), 1);

        apply_event(
            &mut results,
            &BroadcastEvent::Failed {
                host_id: 2,
                reason: "x".into(),
            },
        );
        assert!(all_terminal(&results));
        assert_eq!(done_count(&results), 2);
    }

    #[test]
    fn real_runner_reports_spawn_failure() {
        // A bogus program name can't spawn → Err, no panic. (No network.)
        let runner = SshCommandRunner {
            connect_timeout_secs: 1,
            run_timeout: Duration::from_secs(1),
        };
        let cancel = AtomicBool::new(false);
        let argv = vec![
            "definitely-not-a-real-ssh-binary-xyzzy".to_string(),
            "host".to_string(),
        ];
        let res = runner.run(&argv, "true", None, &cancel);
        assert!(res.is_err());
    }

    #[test]
    fn real_runner_short_circuits_on_precancel() {
        let runner = SshCommandRunner::new();
        let cancel = AtomicBool::new(true);
        let res = runner.run(
            &["ssh".to_string(), "host".to_string()],
            "true",
            None,
            &cancel,
        );
        assert_eq!(res, Err("cancelled".to_string()));
    }

    #[test]
    fn secret_is_threaded_per_task_to_the_runner() {
        use std::sync::atomic::AtomicUsize;

        // Records how many tasks reached the runner carrying a stored secret.
        struct SecretSpy {
            with_secret: Arc<AtomicUsize>,
        }
        impl CommandRunner for SecretSpy {
            fn run(
                &self,
                _argv: &[String],
                _command: &str,
                secret: Option<&crate::session::PendingSecret>,
                _cancel: &AtomicBool,
            ) -> Result<(i32, String, String), String> {
                if secret.is_some() {
                    self.with_secret.fetch_add(1, Ordering::SeqCst);
                }
                Ok((0, String::new(), String::new()))
            }
        }

        let with_secret = Arc::new(AtomicUsize::new(0));
        let tasks = vec![
            BroadcastTask {
                host_id: 1,
                host_name: "pw-host".into(),
                argv: vec!["ssh".into(), "pw-host".into()],
                secret: Some(crate::session::PendingSecret::Password("hunter2".into())),
            },
            BroadcastTask {
                host_id: 2,
                host_name: "key-host".into(),
                argv: vec!["ssh".into(), "key-host".into()],
                secret: None,
            },
        ];
        let rx = spawn_broadcast(
            tasks,
            "true".into(),
            2,
            Arc::new(AtomicBool::new(false)),
            Arc::new(SecretSpy {
                with_secret: Arc::clone(&with_secret),
            }),
        );
        let _drained: Vec<_> = rx.iter().collect();
        assert_eq!(
            with_secret.load(Ordering::SeqCst),
            1,
            "only the host with a stored secret hands one to the runner"
        );
    }
}
