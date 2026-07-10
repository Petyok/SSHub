//! Background SFTP worker thread.
//!
//! Mirrors the ping worker ([`crate::ping`]): a dedicated thread owns the
//! blocking transport, connects once, then services commands off an mpsc
//! channel until the command `Sender` is dropped (the thread self-terminates
//! when an event `send` fails or the command channel closes). All I/O lives
//! here so the synchronous UI event loop never blocks.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use super::model::{QueuedTransfer, Side};
use super::transport::{SftpTransport, Ssh2Transport};
use crate::session::PendingSecret;
use crate::ssh::agent::AgentInfo;
use crate::ssh::SshHost;

/// Emit a `Progress` event at most this often (bytes) during a transfer.
const PROGRESS_STEP: u64 = 64 * 1024;

/// Commands the UI sends to the worker thread.
pub enum SftpCommand {
    /// List a directory. Only the `Remote` side is serviced here; the UI reads
    /// the local filesystem itself via `std::fs`.
    ListDir(Side, PathBuf),
    /// Run the given transfers in order.
    RunQueue(Vec<QueuedTransfer>),
    /// Abort a running queue at the next transfer boundary.
    Cancel,
}

/// Events the worker sends back to the UI (drained in the poll loop).
#[derive(Debug)]
pub enum SftpEvent {
    /// Connection + auth + SFTP subsystem all succeeded.
    Connected,
    /// Connection/auth failed; the worker thread then exits.
    ConnectFailed(String),
    /// A directory listing completed.
    DirListing(Side, PathBuf, Vec<super::model::FileEntry>),
    /// Progress for the transfer at `index` of `total`.
    Progress {
        index: usize,
        total: usize,
        transferred: u64,
        size: u64,
    },
    /// The transfer at `index` finished.
    TransferDone(usize),
    /// The whole queue finished (or was cancelled).
    QueueDone,
    /// A recoverable error for the last command (listing/transfer).
    Error(String),
}

/// Spawn the worker thread. Returns the command sender and event receiver.
/// Dropping the returned `Sender` makes the thread exit after its current
/// command; the thread also exits if it can't deliver an event (UI gone).
pub fn spawn_sftp_worker(
    target: SshHost,
    secret: Option<PendingSecret>,
    agent: AgentInfo,
) -> (Sender<SftpCommand>, Receiver<SftpEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<SftpCommand>();
    let (evt_tx, evt_rx) = mpsc::channel::<SftpEvent>();

    thread::spawn(move || {
        let mut transport = Ssh2Transport::new(target, secret, agent);

        if let Err(e) = transport.connect() {
            let _ = evt_tx.send(SftpEvent::ConnectFailed(format!("{e:#}")));
            return;
        }
        if evt_tx.send(SftpEvent::Connected).is_err() {
            return; // UI gone
        }

        worker_loop(&mut transport, &cmd_rx, &evt_tx);
    });

    (cmd_tx, evt_rx)
}

/// Blocking command loop. Returns (thread ends) when the command channel is
/// closed or an event can't be delivered.
fn worker_loop(
    transport: &mut dyn SftpTransport,
    cmd_rx: &Receiver<SftpCommand>,
    evt_tx: &Sender<SftpEvent>,
) {
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            SftpCommand::ListDir(side, path) => {
                if side != Side::Remote {
                    continue; // local listings are done by the UI
                }
                let evt = match transport.list_dir(&path) {
                    Ok(entries) => SftpEvent::DirListing(side, path, entries),
                    Err(e) => SftpEvent::Error(format!("{e:#}")),
                };
                if evt_tx.send(evt).is_err() {
                    return;
                }
            }
            SftpCommand::RunQueue(queue) => {
                if run_queue(transport, cmd_rx, evt_tx, queue).is_err() {
                    return;
                }
            }
            // A stray Cancel with no queue running is a no-op.
            SftpCommand::Cancel => {}
        }
    }
}

/// Run every transfer in `queue`, emitting throttled progress. Checks for a
/// `Cancel` command between transfers. Returns `Err(())` only when an event
/// can't be delivered (UI gone) so the caller can end the thread.
fn run_queue(
    transport: &mut dyn SftpTransport,
    cmd_rx: &Receiver<SftpCommand>,
    evt_tx: &Sender<SftpEvent>,
    queue: Vec<QueuedTransfer>,
) -> Result<(), ()> {
    use super::model::Direction;

    let total = queue.len();
    for (index, item) in queue.into_iter().enumerate() {
        // Honour a Cancel queued between transfers.
        if drain_cancel(cmd_rx) {
            break;
        }

        let mut last_emit: u64 = 0;
        let mut deliver_err = false;
        let result = {
            let mut on_progress = |transferred: u64, size: u64| {
                if deliver_err {
                    return;
                }
                let done = transferred >= size && size > 0;
                if transferred.saturating_sub(last_emit) >= PROGRESS_STEP
                    || transferred == 0
                    || done
                {
                    last_emit = transferred;
                    if evt_tx
                        .send(SftpEvent::Progress {
                            index,
                            total,
                            transferred,
                            size,
                        })
                        .is_err()
                    {
                        deliver_err = true;
                    }
                }
            };
            match item.direction {
                Direction::Download => transport.download(&item.src, &item.dst, &mut on_progress),
                Direction::Upload => transport.upload(&item.src, &item.dst, &mut on_progress),
            }
        };
        if deliver_err {
            return Err(());
        }

        match result {
            Ok(()) => {
                if evt_tx.send(SftpEvent::TransferDone(index)).is_err() {
                    return Err(());
                }
            }
            Err(e) => {
                if evt_tx.send(SftpEvent::Error(format!("{e:#}"))).is_err() {
                    return Err(());
                }
            }
        }
    }

    if evt_tx.send(SftpEvent::QueueDone).is_err() {
        return Err(());
    }
    Ok(())
}

/// Non-blocking check for a pending `Cancel` command. Returns true if one was
/// seen. Non-cancel commands drained here are dropped (a run is in flight).
fn drain_cancel(cmd_rx: &Receiver<SftpCommand>) -> bool {
    let mut cancelled = false;
    loop {
        match cmd_rx.try_recv() {
            Ok(SftpCommand::Cancel) => cancelled = true,
            Ok(_) => {} // ignore other commands mid-run
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
        }
    }
    cancelled
}

#[cfg(test)]
mod tests {
    //! Offline worker/reducer tests. A [`MockTransport`] stands in for the
    //! libssh2 backend (mirrors `tests/support::MockLauncher`): it serves canned
    //! listings and fake transfers that emit progress through the worker's
    //! callback, so the real private `worker_loop` / `run_queue` reducers run
    //! without a network, filesystem, or SSH session.

    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use anyhow::{anyhow, Result};

    use super::*;
    use crate::sftp::model::{Direction, FileEntry};
    use crate::sftp::transport::SftpTransport;

    /// A record of one download/upload the mock was asked to perform.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TransferCall {
        direction: Direction,
        src: PathBuf,
        dst: PathBuf,
    }

    /// Canned transport: `listings` maps a path → entries; every transfer emits
    /// `progress_bytes` bytes in `PROGRESS_STEP`-sized chunks. Optionally fails
    /// a given transfer index to exercise the recoverable-error path.
    struct MockTransport {
        listings: HashMap<PathBuf, Vec<FileEntry>>,
        /// Total "size" every fake transfer reports/moves.
        transfer_size: u64,
        /// Records of every download/upload requested, in order.
        calls: Arc<Mutex<Vec<TransferCall>>>,
        /// Transfer ordinal (0-based) that should fail, if any.
        fail_on: Option<usize>,
        seen: usize,
        connected: bool,
    }

    impl MockTransport {
        fn new(transfer_size: u64) -> Self {
            Self {
                listings: HashMap::new(),
                transfer_size,
                calls: Arc::new(Mutex::new(Vec::new())),
                fail_on: None,
                seen: 0,
                connected: false,
            }
        }

        fn with_listing(mut self, path: impl Into<PathBuf>, entries: Vec<FileEntry>) -> Self {
            self.listings.insert(path.into(), entries);
            self
        }

        fn calls_handle(&self) -> Arc<Mutex<Vec<TransferCall>>> {
            Arc::clone(&self.calls)
        }

        /// Common body for both transfer directions: record the call, then emit
        /// progress in PROGRESS_STEP chunks (so the reducer's throttling runs).
        fn fake_transfer(
            &mut self,
            direction: Direction,
            src: &Path,
            dst: &Path,
            progress: &mut dyn FnMut(u64, u64),
        ) -> Result<()> {
            let ordinal = self.seen;
            self.seen += 1;
            self.calls.lock().unwrap().push(TransferCall {
                direction,
                src: src.to_path_buf(),
                dst: dst.to_path_buf(),
            });
            if self.fail_on == Some(ordinal) {
                return Err(anyhow!("boom on transfer {ordinal}"));
            }
            let total = self.transfer_size;
            progress(0, total);
            let step = PROGRESS_STEP;
            let mut moved = 0u64;
            while moved < total {
                // sub-PROGRESS_STEP increments to exercise throttling
                moved = (moved + step / 2).min(total);
                progress(moved, total);
            }
            Ok(())
        }
    }

    impl SftpTransport for MockTransport {
        fn connect(&mut self) -> Result<()> {
            self.connected = true;
            Ok(())
        }

        fn list_dir(&mut self, path: &Path) -> Result<Vec<FileEntry>> {
            self.listings
                .get(path)
                .cloned()
                .ok_or_else(|| anyhow!("no canned listing for {}", path.display()))
        }

        fn download(
            &mut self,
            remote: &Path,
            local: &Path,
            progress: &mut dyn FnMut(u64, u64),
        ) -> Result<()> {
            self.fake_transfer(Direction::Download, remote, local, progress)
        }

        fn upload(
            &mut self,
            local: &Path,
            remote: &Path,
            progress: &mut dyn FnMut(u64, u64),
        ) -> Result<()> {
            self.fake_transfer(Direction::Upload, local, remote, progress)
        }
    }

    fn entry(name: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            name: name.into(),
            is_dir,
            size: if is_dir { 0 } else { 42 },
        }
    }

    fn dl(src: &str, dst: &str) -> QueuedTransfer {
        QueuedTransfer {
            direction: Direction::Download,
            src: PathBuf::from(src),
            dst: PathBuf::from(dst),
            name: PathBuf::from(src)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        }
    }

    fn up(src: &str, dst: &str) -> QueuedTransfer {
        QueuedTransfer {
            direction: Direction::Upload,
            src: PathBuf::from(src),
            dst: PathBuf::from(dst),
            name: PathBuf::from(src)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        }
    }

    /// Drive `worker_loop` with a pre-loaded command queue, then drop the sender
    /// so the loop returns, and collect every emitted event.
    fn drive(transport: &mut dyn SftpTransport, cmds: Vec<SftpCommand>) -> Vec<SftpEvent> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<SftpCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<SftpEvent>();
        for c in cmds {
            cmd_tx.send(c).unwrap();
        }
        drop(cmd_tx); // close the command channel so worker_loop terminates
        worker_loop(transport, &cmd_rx, &evt_tx);
        drop(evt_tx);
        evt_rx.into_iter().collect()
    }

    #[test]
    fn list_dir_remote_emits_listing() {
        let entries = vec![entry("docs", true), entry("a.txt", false)];
        let mut t = MockTransport::new(0).with_listing("/srv", entries.clone());
        let events = drive(
            &mut t,
            vec![SftpCommand::ListDir(Side::Remote, PathBuf::from("/srv"))],
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            SftpEvent::DirListing(side, path, got) => {
                assert_eq!(*side, Side::Remote);
                assert_eq!(path, &PathBuf::from("/srv"));
                assert_eq!(got, &entries);
            }
            other => panic!("expected DirListing, got {other:?}"),
        }
    }

    #[test]
    fn list_dir_local_is_ignored() {
        // The worker only services remote listings; a Local request emits nothing.
        let mut t = MockTransport::new(0);
        let events = drive(
            &mut t,
            vec![SftpCommand::ListDir(Side::Local, PathBuf::from("/home"))],
        );
        assert!(events.is_empty());
    }

    #[test]
    fn list_dir_error_is_recoverable() {
        // No canned listing for this path → Error event, loop keeps going.
        let mut t = MockTransport::new(0);
        let events = drive(
            &mut t,
            vec![SftpCommand::ListDir(
                Side::Remote,
                PathBuf::from("/missing"),
            )],
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], SftpEvent::Error(_)));
    }

    #[test]
    fn run_queue_emits_progress_done_and_queuedone() {
        // One transfer of exactly PROGRESS_STEP bytes.
        let mut t = MockTransport::new(PROGRESS_STEP);
        let calls = t.calls_handle();
        let events = drive(
            &mut t,
            vec![SftpCommand::RunQueue(vec![dl("/srv/a.txt", "/home/a.txt")])],
        );

        // The mock actually got asked to download the queued item.
        let recorded = calls.lock().unwrap().clone();
        assert_eq!(
            recorded,
            vec![TransferCall {
                direction: Direction::Download,
                src: PathBuf::from("/srv/a.txt"),
                dst: PathBuf::from("/home/a.txt"),
            }]
        );

        // Event shape: Progress(0) ... at least one done Progress, TransferDone(0), QueueDone.
        assert!(matches!(
            events.first(),
            Some(SftpEvent::Progress {
                index: 0,
                total: 1,
                transferred: 0,
                ..
            })
        ));
        assert!(matches!(events.last(), Some(SftpEvent::QueueDone)));
        let transfer_done = events
            .iter()
            .position(|e| matches!(e, SftpEvent::TransferDone(0)));
        let queue_done = events
            .iter()
            .position(|e| matches!(e, SftpEvent::QueueDone));
        assert!(transfer_done.is_some(), "expected a TransferDone(0)");
        assert!(
            transfer_done < queue_done,
            "TransferDone must precede QueueDone"
        );
        // A final done-progress (transferred == size) must have been emitted.
        assert!(events.iter().any(|e| matches!(
            e,
            SftpEvent::Progress { transferred, size, .. } if transferred == size && *size > 0
        )));
    }

    #[test]
    fn run_queue_multiple_transfers_indexed() {
        let mut t = MockTransport::new(PROGRESS_STEP);
        let calls = t.calls_handle();
        let queue = vec![
            dl("/srv/a.txt", "/home/a.txt"),
            up("/home/b.bin", "/srv/b.bin"),
        ];
        let events = drive(&mut t, vec![SftpCommand::RunQueue(queue)]);

        // Both directions were exercised, in order.
        let recorded = calls.lock().unwrap().clone();
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].direction, Direction::Download);
        assert_eq!(recorded[1].direction, Direction::Upload);
        assert_eq!(recorded[1].src, PathBuf::from("/home/b.bin"));
        assert_eq!(recorded[1].dst, PathBuf::from("/srv/b.bin"));

        // Each transfer index reports total == 2 and gets its own TransferDone.
        assert!(events.iter().any(|e| matches!(
            e,
            SftpEvent::Progress {
                index: 0,
                total: 2,
                ..
            }
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            SftpEvent::Progress {
                index: 1,
                total: 2,
                ..
            }
        )));
        assert!(events
            .iter()
            .any(|e| matches!(e, SftpEvent::TransferDone(0))));
        assert!(events
            .iter()
            .any(|e| matches!(e, SftpEvent::TransferDone(1))));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, SftpEvent::QueueDone))
                .count(),
            1
        );
    }

    #[test]
    fn run_queue_transfer_error_is_reported_and_queue_continues() {
        let mut t = MockTransport::new(PROGRESS_STEP);
        t.fail_on = Some(0); // first transfer fails
        let queue = vec![
            dl("/srv/a.txt", "/home/a.txt"),
            dl("/srv/c.txt", "/home/c.txt"),
        ];
        let events = drive(&mut t, vec![SftpCommand::RunQueue(queue)]);

        // The failed transfer yields an Error (not TransferDone), the second still runs.
        assert!(events.iter().any(|e| matches!(e, SftpEvent::Error(_))));
        assert!(!events
            .iter()
            .any(|e| matches!(e, SftpEvent::TransferDone(0))));
        assert!(events
            .iter()
            .any(|e| matches!(e, SftpEvent::TransferDone(1))));
        assert!(matches!(events.last(), Some(SftpEvent::QueueDone)));
    }

    #[test]
    fn run_queue_stops_when_ui_gone() {
        // If the event receiver is dropped, run_queue returns Err(()) and the
        // worker exits without panicking.
        let (_cmd_tx, cmd_rx) = mpsc::channel::<SftpCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<SftpEvent>();
        drop(evt_rx); // UI gone: every send fails
        let mut t = MockTransport::new(PROGRESS_STEP);
        let res = run_queue(
            &mut t,
            &cmd_rx,
            &evt_tx,
            vec![dl("/srv/a.txt", "/home/a.txt")],
        );
        assert!(res.is_err());
    }

    #[test]
    fn drain_cancel_detects_pending_cancel() {
        let (cmd_tx, cmd_rx) = mpsc::channel::<SftpCommand>();
        cmd_tx
            .send(SftpCommand::ListDir(Side::Remote, PathBuf::from("/x")))
            .unwrap();
        cmd_tx.send(SftpCommand::Cancel).unwrap();
        // Both a stray command and a Cancel are drained; Cancel is detected.
        assert!(drain_cancel(&cmd_rx));
        // Channel now empty → no cancel seen.
        assert!(!drain_cancel(&cmd_rx));
    }

    #[test]
    fn run_queue_honours_cancel_between_transfers() {
        // Pre-queue a Cancel; run_queue drains it before the first transfer and
        // skips the whole queue, emitting only QueueDone (no TransferDone).
        let (cmd_tx, cmd_rx) = mpsc::channel::<SftpCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<SftpEvent>();
        cmd_tx.send(SftpCommand::Cancel).unwrap();
        let mut t = MockTransport::new(PROGRESS_STEP);
        let calls = t.calls_handle();
        let res = run_queue(
            &mut t,
            &cmd_rx,
            &evt_tx,
            vec![dl("/srv/a.txt", "/home/a.txt")],
        );
        drop(evt_tx);
        assert!(res.is_ok());
        // No transfer actually ran.
        assert!(calls.lock().unwrap().is_empty());
        let events: Vec<_> = evt_rx.into_iter().collect();
        assert!(!events
            .iter()
            .any(|e| matches!(e, SftpEvent::TransferDone(_))));
        assert!(matches!(events.last(), Some(SftpEvent::QueueDone)));
    }
}
