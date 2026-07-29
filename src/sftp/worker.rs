//! Background SFTP worker thread.
//!
//! Mirrors the ping worker ([`crate::ping`]): a dedicated thread owns the
//! blocking transport, connects once, then services commands off an mpsc
//! channel until the command `Sender` is dropped (the thread self-terminates
//! when an event `send` fails or the command channel closes). All I/O lives
//! here so the synchronous UI event loop never blocks.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use anyhow::{Context, Result};

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
    /// Delete a remote path (recursively if `is_dir`).
    Remove(PathBuf, bool),
    /// Create a remote directory.
    Mkdir(PathBuf),
    /// Rename / move a remote path.
    Rename(PathBuf, PathBuf),
    /// Set permission bits of a remote path (chmod).
    Chmod(PathBuf, u32),
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
    /// A remote file operation (remove/mkdir/rename) succeeded; the UI should
    /// refresh its panes.
    OpDone,
    /// A recoverable error for the last command (listing/transfer/op).
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
            SftpCommand::Remove(path, is_dir) => {
                // Op errors name the operation and target: they can surface
                // long after dispatch (e.g. while a transfer queue is running),
                // so a bare message would be impossible to attribute.
                let evt = match transport.remove(&path, is_dir) {
                    Ok(()) => SftpEvent::OpDone,
                    Err(e) => SftpEvent::Error(format!("delete {}: {e:#}", path.display())),
                };
                if evt_tx.send(evt).is_err() {
                    return;
                }
            }
            SftpCommand::Mkdir(path) => {
                let evt = match transport.mkdir(&path) {
                    Ok(()) => SftpEvent::OpDone,
                    Err(e) => SftpEvent::Error(format!("mkdir {}: {e:#}", path.display())),
                };
                if evt_tx.send(evt).is_err() {
                    return;
                }
            }
            SftpCommand::Rename(from, to) => {
                let evt = match transport.rename(&from, &to) {
                    Ok(()) => SftpEvent::OpDone,
                    Err(e) => SftpEvent::Error(format!("rename {}: {e:#}", from.display())),
                };
                if evt_tx.send(evt).is_err() {
                    return;
                }
            }
            SftpCommand::Chmod(path, mode) => {
                let evt = match transport.chmod(&path, mode) {
                    Ok(()) => SftpEvent::OpDone,
                    Err(e) => SftpEvent::Error(format!("chmod {}: {e:#}", path.display())),
                };
                if evt_tx.send(evt).is_err() {
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
            // Cumulative progress emitter: `transferred`/`size` are totals for
            // this queue item (a whole directory tree, or a single file).
            let mut emit = |transferred: u64, size: u64| {
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
            if item.is_dir {
                transfer_dir(transport, &item, cmd_rx, &mut emit)
            } else {
                match item.direction {
                    Direction::Download => transport
                        .download(&item.src, &item.dst, &mut emit)
                        .map(|_| false),
                    Direction::Upload => transport
                        .upload(&item.src, &item.dst, &mut emit)
                        .map(|_| false),
                }
            }
        };
        if deliver_err {
            return Err(());
        }

        match result {
            // `true` = a Cancel landed mid-directory → stop the whole queue.
            Ok(true) => break,
            Ok(false) => {
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

/// Transfer a whole directory recursively. Plans the file list + total byte
/// count first (so progress is a real fraction of the tree), then streams each
/// file, reporting cumulative bytes against the total via `emit`.
/// Returns `Ok(true)` if a `Cancel` was seen mid-tree (so the caller stops the
/// queue), `Ok(false)` on normal completion.
fn transfer_dir(
    transport: &mut dyn SftpTransport,
    item: &QueuedTransfer,
    cmd_rx: &Receiver<SftpCommand>,
    emit: &mut dyn FnMut(u64, u64),
) -> Result<bool> {
    use super::model::Direction;

    let mut files: Vec<(PathBuf, PathBuf, u64)> = Vec::new();
    let mut total: u64 = 0;
    // Planning walks the whole tree (potentially slow on a deep remote dir), so
    // it polls Cancel too — otherwise an abort wouldn't be honoured until the
    // enumeration finished.
    let mut cancelled = false;
    match item.direction {
        Direction::Download => plan_download(
            transport,
            &item.src,
            &item.dst,
            &mut files,
            &mut total,
            cmd_rx,
            &mut cancelled,
        )?,
        Direction::Upload => plan_upload(
            transport,
            &item.src,
            &item.dst,
            &mut files,
            &mut total,
            cmd_rx,
            &mut cancelled,
        )?,
    }
    if cancelled {
        return Ok(true);
    }

    let mut base: u64 = 0;
    let mut failed = 0usize;
    let mut first_err: Option<String> = None;
    emit(0, total);
    for (src, dst, size) in files {
        // Honour Cancel between individual files, not just between queue items —
        // a directory tree can be thousands of files / many GB.
        if drain_cancel(cmd_rx) {
            return Ok(true);
        }
        let r = match item.direction {
            Direction::Download => {
                transport.download(&src, &dst, &mut |t, _| emit(base + t, total))
            }
            Direction::Upload => transport.upload(&src, &dst, &mut |t, _| emit(base + t, total)),
        };
        // One unreadable file must not abort the rest of the tree — skip it,
        // keep going, and report a summary at the end.
        if let Err(e) = r {
            failed += 1;
            if first_err.is_none() {
                first_err = Some(format!("{e:#}"));
            }
        }
        base += size;
        emit(base, total);
    }
    if let Some(first) = first_err {
        anyhow::bail!("{failed} file(s) failed (first: {first})");
    }
    Ok(false)
}

/// Walk a remote directory tree, creating the mirrored local directories and
/// collecting `(remote_file, local_file, size)` for every file. `readdir`
/// yields file names, so child paths are `dir.join(name)`. Sets `*cancelled`
/// and returns early if a `Cancel` arrives mid-walk.
fn plan_download(
    transport: &mut dyn SftpTransport,
    remote_dir: &Path,
    local_dir: &Path,
    out: &mut Vec<(PathBuf, PathBuf, u64)>,
    total: &mut u64,
    cmd_rx: &Receiver<SftpCommand>,
    cancelled: &mut bool,
) -> Result<()> {
    if *cancelled {
        return Ok(());
    }
    if drain_cancel(cmd_rx) {
        *cancelled = true;
        return Ok(());
    }
    std::fs::create_dir_all(local_dir)
        .with_context(|| format!("failed to create {}", local_dir.display()))?;
    for e in transport.list_dir(remote_dir)? {
        if *cancelled {
            return Ok(());
        }
        let rpath = remote_dir.join(&e.name);
        let lpath = local_dir.join(&e.name);
        // A symlink is never descended into (cycle / escape risk). Follow-stat
        // classifies it instead: a symlink-to-file transfers with the TARGET's
        // size (the readdir lstat size is just the link text length, which
        // would skew the progress total); a symlink-to-dir or broken link is
        // skipped entirely — opening it as a file would fail the whole queue.
        if e.is_symlink {
            if let Ok((false, size)) = transport.stat(&rpath) {
                *total += size;
                out.push((rpath, lpath, size));
            }
        } else if e.is_dir {
            plan_download(transport, &rpath, &lpath, out, total, cmd_rx, cancelled)?;
        } else {
            *total += e.size;
            out.push((rpath, lpath, e.size));
        }
    }
    Ok(())
}

/// Walk a local directory tree, creating the mirrored remote directories and
/// collecting `(local_file, remote_file, size)` for every file. `mkdir` errors
/// are tolerated only when the directory already exists (verified via
/// `list_dir`), so a genuinely failed dir creation — e.g. an empty subdir that
/// has no files to surface the error later — is not swallowed. Polls Cancel.
fn plan_upload(
    transport: &mut dyn SftpTransport,
    local_dir: &Path,
    remote_dir: &Path,
    out: &mut Vec<(PathBuf, PathBuf, u64)>,
    total: &mut u64,
    cmd_rx: &Receiver<SftpCommand>,
    cancelled: &mut bool,
) -> Result<()> {
    if *cancelled {
        return Ok(());
    }
    if drain_cancel(cmd_rx) {
        *cancelled = true;
        return Ok(());
    }
    if let Err(e) = transport.mkdir(remote_dir) {
        // Only tolerate "already exists": if the dir isn't there afterwards,
        // the mkdir genuinely failed.
        if transport.list_dir(remote_dir).is_err() {
            return Err(e)
                .with_context(|| format!("failed to create remote {}", remote_dir.display()));
        }
    }
    let entries = std::fs::read_dir(local_dir)
        .with_context(|| format!("failed to read {}", local_dir.display()))?;
    for entry in entries {
        if *cancelled {
            return Ok(());
        }
        let entry = entry?;
        // file_type() does not follow symlinks; same classification as
        // plan_download — a symlink is never descended into, a symlink-to-file
        // transfers with the target's size (fs::metadata follows the link),
        // a symlink-to-dir or broken link is skipped.
        let ftype = entry.file_type()?;
        let name = entry.file_name();
        let lpath = local_dir.join(&name);
        let rpath = remote_dir.join(&name);
        if ftype.is_symlink() {
            match std::fs::metadata(&lpath) {
                Ok(m) if !m.is_dir() => {
                    let size = m.len();
                    *total += size;
                    out.push((lpath, rpath, size));
                }
                _ => {}
            }
        } else if ftype.is_dir() {
            plan_upload(transport, &lpath, &rpath, out, total, cmd_rx, cancelled)?;
        } else {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            *total += size;
            out.push((lpath, rpath, size));
        }
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
    //! libssh2 backend: it serves canned
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

    /// A record of one remove/mkdir/rename op the mock was asked to perform.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum OpCall {
        Remove(PathBuf, bool),
        Mkdir(PathBuf),
        Rename(PathBuf, PathBuf),
        Chmod(PathBuf, u32),
    }

    /// Canned transport: `listings` maps a path → entries; every transfer emits
    /// `progress_bytes` bytes in `PROGRESS_STEP`-sized chunks. Optionally fails
    /// a given transfer index to exercise the recoverable-error path.
    struct MockTransport {
        listings: HashMap<PathBuf, Vec<FileEntry>>,
        /// Canned follow-stat results for symlink classification.
        stats: HashMap<PathBuf, (bool, u64)>,
        /// Total "size" every fake transfer reports/moves.
        transfer_size: u64,
        /// Records of every download/upload requested, in order.
        calls: Arc<Mutex<Vec<TransferCall>>>,
        /// Records of every remove/mkdir/rename requested, in order.
        ops: Arc<Mutex<Vec<OpCall>>>,
        /// Transfer ordinal (0-based) that should fail, if any.
        fail_on: Option<usize>,
        seen: usize,
        connected: bool,
    }

    impl MockTransport {
        fn new(transfer_size: u64) -> Self {
            Self {
                listings: HashMap::new(),
                stats: HashMap::new(),
                transfer_size,
                calls: Arc::new(Mutex::new(Vec::new())),
                ops: Arc::new(Mutex::new(Vec::new())),
                fail_on: None,
                seen: 0,
                connected: false,
            }
        }

        fn with_listing(mut self, path: impl Into<PathBuf>, entries: Vec<FileEntry>) -> Self {
            self.listings.insert(path.into(), entries);
            self
        }

        fn with_stat(mut self, path: impl Into<PathBuf>, is_dir: bool, size: u64) -> Self {
            self.stats.insert(path.into(), (is_dir, size));
            self
        }

        fn calls_handle(&self) -> Arc<Mutex<Vec<TransferCall>>> {
            Arc::clone(&self.calls)
        }

        fn ops_handle(&self) -> Arc<Mutex<Vec<OpCall>>> {
            Arc::clone(&self.ops)
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

        fn stat(&mut self, path: &Path) -> Result<(bool, u64)> {
            self.stats
                .get(path)
                .copied()
                .ok_or_else(|| anyhow!("no canned stat for {}", path.display()))
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

        fn remove(&mut self, path: &Path, is_dir: bool) -> Result<()> {
            self.ops
                .lock()
                .unwrap()
                .push(OpCall::Remove(path.to_path_buf(), is_dir));
            Ok(())
        }

        fn mkdir(&mut self, path: &Path) -> Result<()> {
            self.ops
                .lock()
                .unwrap()
                .push(OpCall::Mkdir(path.to_path_buf()));
            Ok(())
        }

        fn rename(&mut self, from: &Path, to: &Path) -> Result<()> {
            self.ops
                .lock()
                .unwrap()
                .push(OpCall::Rename(from.to_path_buf(), to.to_path_buf()));
            Ok(())
        }

        fn chmod(&mut self, path: &Path, mode: u32) -> Result<()> {
            self.ops
                .lock()
                .unwrap()
                .push(OpCall::Chmod(path.to_path_buf(), mode));
            Ok(())
        }
    }

    fn entry(name: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            name: name.into(),
            is_dir,
            size: if is_dir { 0 } else { 42 },
            is_symlink: false,
            perm: None,
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
            is_dir: false,
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
            is_dir: false,
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

    #[test]
    fn remote_ops_emit_opdone_and_record() {
        let mut t = MockTransport::new(0);
        let ops = t.ops_handle();
        let events = drive(
            &mut t,
            vec![
                SftpCommand::Mkdir(PathBuf::from("/srv/newdir")),
                SftpCommand::Rename(PathBuf::from("/srv/a.txt"), PathBuf::from("/srv/b.txt")),
                SftpCommand::Chmod(PathBuf::from("/srv/x.sh"), 0o755),
                SftpCommand::Remove(PathBuf::from("/srv/old"), true),
            ],
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, SftpEvent::OpDone))
                .count(),
            4
        );
        let recorded = ops.lock().unwrap().clone();
        assert_eq!(
            recorded,
            vec![
                OpCall::Mkdir(PathBuf::from("/srv/newdir")),
                OpCall::Rename(PathBuf::from("/srv/a.txt"), PathBuf::from("/srv/b.txt")),
                OpCall::Chmod(PathBuf::from("/srv/x.sh"), 0o755),
                OpCall::Remove(PathBuf::from("/srv/old"), true),
            ]
        );
    }

    #[test]
    fn recursive_download_transfers_whole_tree() {
        // Canned nested remote tree; a directory queue item must download every
        // file to a mirrored local path and create the local subdirectories.
        let base = std::env::temp_dir().join(format!("sshub-sftp-v2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dst = base.join("proj");

        let mut t = MockTransport::new(PROGRESS_STEP)
            .with_listing("/srv/proj", vec![entry("sub", true), entry("a.txt", false)])
            .with_listing("/srv/proj/sub", vec![entry("b.txt", false)]);
        let calls = t.calls_handle();

        let item = QueuedTransfer {
            direction: Direction::Download,
            src: PathBuf::from("/srv/proj"),
            dst: dst.clone(),
            name: "proj".into(),
            is_dir: true,
        };
        let events = drive(&mut t, vec![SftpCommand::RunQueue(vec![item])]);

        let recorded = calls.lock().unwrap().clone();
        let dl_dsts: Vec<_> = recorded.iter().map(|c| c.dst.clone()).collect();
        assert_eq!(recorded.len(), 2, "both files in the tree downloaded");
        assert!(dl_dsts.contains(&dst.join("a.txt")));
        assert!(dl_dsts.contains(&dst.join("sub").join("b.txt")));
        assert!(dst.join("sub").is_dir(), "local mirror dir created");
        assert!(events
            .iter()
            .any(|e| matches!(e, SftpEvent::TransferDone(0))));
        assert!(matches!(events.last(), Some(SftpEvent::QueueDone)));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn recursive_download_classifies_symlinks_via_stat() {
        // Symlinks are never descended into. Follow-stat classifies them:
        // "dirlink" (target is a directory) is skipped — downloading it as a
        // file would fail the queue with an open/EISDIR error; "filelink"
        // (target is a file) transfers with the TARGET's size, not the lstat
        // link-text length; "broken" (stat fails) is skipped too.
        let base = std::env::temp_dir().join(format!("sshub-sftp-v2-sym-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dst = base.join("proj");

        let sym = |name: &str| FileEntry {
            name: name.into(),
            is_dir: false,
            size: 9, // lstat size: the link text length, must NOT be used
            is_symlink: true,
            perm: None,
        };
        let mut t = MockTransport::new(PROGRESS_STEP)
            .with_listing(
                "/srv/proj",
                vec![
                    sym("dirlink"),
                    sym("filelink"),
                    sym("broken"),
                    entry("real.txt", false),
                ],
            )
            .with_stat("/srv/proj/dirlink", true, 0)
            .with_stat("/srv/proj/filelink", false, 1000);
        let calls = t.calls_handle();

        let item = QueuedTransfer {
            direction: Direction::Download,
            src: PathBuf::from("/srv/proj"),
            dst: dst.clone(),
            name: "proj".into(),
            is_dir: true,
        };
        let events = drive(&mut t, vec![SftpCommand::RunQueue(vec![item])]);

        let recorded = calls.lock().unwrap().clone();
        let dsts: Vec<_> = recorded.iter().map(|c| c.dst.clone()).collect();
        // Only the file-symlink and the real file transfer; the dir-symlink
        // and the broken link are skipped, and the dir-symlink target is
        // never listed (which would error — no canned listing).
        assert_eq!(recorded.len(), 2);
        assert!(dsts.contains(&dst.join("filelink")));
        assert!(dsts.contains(&dst.join("real.txt")));
        assert!(
            !events.iter().any(|e| matches!(e, SftpEvent::Error(_))),
            "no error — symlinks never opened as dirs"
        );
        // The planned tree size counts the filelink TARGET size (1000) +
        // real.txt (42), not the 9-byte link texts.
        let planned = events
            .iter()
            .find_map(|e| match e {
                SftpEvent::Progress { size, .. } => Some(*size),
                _ => None,
            })
            .expect("progress emitted");
        assert_eq!(planned, 1042);
        assert!(matches!(events.last(), Some(SftpEvent::QueueDone)));

        let _ = std::fs::remove_dir_all(&base);
    }
}
