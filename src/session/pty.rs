//! PTY runtime: spawns the child on a pseudo-TTY, runs a reader thread, and
//! exposes a non-blocking event stream + writer.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use anyhow::{anyhow, Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

const READ_BUF: usize = 4096;

/// Event from the PTY reader thread to the main thread.
#[derive(Debug)]
pub enum PtyEvent {
    /// Bytes read from the master side of the PTY.
    Bytes(Vec<u8>),
    /// Child exited; carries a human-readable status string.
    Exited(String),
}

pub struct PtyRuntime {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<PtyEvent>,
    /// Set when the reader has signalled EOF / child exit. Used so we don't
    /// keep spinning on a dead PTY.
    closed: Arc<AtomicBool>,
    reader_thread: Option<JoinHandle<()>>,
    child: Option<Box<dyn Child + Send + Sync>>,
}

impl PtyRuntime {
    pub fn spawn(argv: &[String], rows: u16, cols: u16) -> Result<Self> {
        let program = argv.first().ok_or_else(|| anyhow!("empty argv"))?.clone();

        let mut cmd = CommandBuilder::new(&program);
        for arg in &argv[1..] {
            cmd.arg(arg);
        }
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }
        // Override TERM. Our vt100 emulator is xterm-compatible; advertising
        // `xterm-kitty` (often inherited from the user's host kitty session)
        // leaves the remote without a matching terminfo entry — breaking
        // `clear`, `tput`, ncurses apps, etc. Force a portable default.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("openpty")?;

        let child = pair
            .slave
            .spawn_command(cmd)
            .context("spawn child on pty slave")?;
        // Slave is no longer needed in this process.
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
        let writer = pair.master.take_writer().context("take pty writer")?;

        let (tx, rx) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let closed_thread = Arc::clone(&closed);

        let reader_thread = thread::Builder::new()
            .name("sshub-pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; READ_BUF];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => {
                            let _ = tx.send(PtyEvent::Exited("eof".into()));
                            break;
                        }
                        Ok(n) => {
                            if tx.send(PtyEvent::Bytes(buf[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(PtyEvent::Exited(format!("read error: {e}")));
                            break;
                        }
                    }
                }
                closed_thread.store(true, Ordering::Relaxed);
            })
            .context("spawn pty reader thread")?;

        Ok(Self {
            master: pair.master,
            writer,
            rx,
            closed,
            reader_thread: Some(reader_thread),
            child: Some(child),
        })
    }

    /// Non-blocking poll for one event.
    pub fn try_recv(&self) -> Option<PtyEvent> {
        self.rx.try_recv().ok()
    }

    /// Write bytes to the master side. Called for each forwarded keystroke.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush().ok();
        Ok(())
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("pty resize")?;
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

impl Drop for PtyRuntime {
    fn drop(&mut self) {
        // Best-effort: kill the child, then join the reader thread with a short
        // grace period. Reader exits when the master is dropped (its clone reader
        // returns EOF / error).
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}
