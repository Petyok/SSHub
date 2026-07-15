use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{
    recommended_watcher, Event, EventKind, RecursiveMode, Result as NotifyResult, Watcher,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchEvent {
    ConfigChanged,
}

/// Start file watcher on SSH config. Implemented in phase F6.
pub fn spawn_config_watcher(ssh_config_path: &Path) -> Result<Receiver<WatchEvent>> {
    let config_path = ssh_config_path.to_path_buf();

    // Editors save by writing a temp file and renaming it over the config, which
    // swaps the inode and silently detaches a watch placed on the file itself.
    // Watch the *containing directory* instead and filter events down to the
    // config file, so rename-based saves keep firing. Require the file to exist
    // up front to preserve the "missing config errors out" contract.
    if !config_path.exists() {
        anyhow::bail!(
            "watch SSH config at {}: file not found",
            config_path.display()
        );
    }
    let watch_dir = config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(".").to_path_buf());

    let (notify_tx, notify_rx) = mpsc::channel();
    let mut watcher = recommended_watcher(notify_tx).context("create config file watcher")?;
    watcher
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("watch SSH config at {}", config_path.display()))?;

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        debounce_loop(notify_rx, tx, &config_path);
        drop(watcher);
    });

    Ok(rx)
}

fn debounce_loop(notify_rx: Receiver<NotifyResult<Event>>, tx: Sender<WatchEvent>, config: &Path) {
    loop {
        match notify_rx.recv() {
            Ok(Ok(event)) if is_config_change(&event, config) => {}
            Ok(Ok(_)) => continue,
            Ok(Err(_)) => continue,
            Err(_) => break,
        }

        let deadline = Instant::now() + WATCHER_DEBOUNCE;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match notify_rx.recv_timeout(remaining) {
                Ok(Ok(event)) if is_config_change(&event, config) => continue,
                Ok(Ok(_)) | Ok(Err(_)) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        if tx.send(WatchEvent::ConfigChanged).is_err() {
            break;
        }
    }
}

fn is_config_change(event: &Event, config: &Path) -> bool {
    if !matches!(
        event.kind,
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
    ) {
        return false;
    }
    // We watch the whole directory, so keep only events that touch the config
    // file (matched by name, since a rename-in swaps the inode/path target).
    let name = config.file_name();
    event
        .paths
        .iter()
        .any(|p| p == config || p.file_name() == name)
}

/// Debounce duration for watcher thread.
pub const WATCHER_DEBOUNCE: Duration = Duration::from_millis(300);

/// No-op channel for bootstrap / tests before F6.
pub fn dummy_watcher() -> Receiver<WatchEvent> {
    let (_tx, rx) = mpsc::channel();
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::mpsc::RecvTimeoutError;
    use tempfile::NamedTempFile;

    /// How long to wait for a debounced `ConfigChanged` after config writes.
    fn watcher_event_timeout() -> Duration {
        WATCHER_DEBOUNCE + Duration::from_secs(2)
    }

    /// After a burst of rapid writes, FSEvents on macOS CI can deliver notifications
    /// seconds later; each delivery resets debounce, so use a generous bound.
    fn debounce_burst_timeout() -> Duration {
        if cfg!(target_os = "macos") {
            Duration::from_secs(60)
        } else {
            watcher_event_timeout()
        }
    }

    #[test]
    fn spawn_config_watcher_emits_after_write() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "Host alpha").unwrap();
        file.flush().unwrap();

        let rx = spawn_config_watcher(file.path()).unwrap();
        writeln!(file, "Host beta").unwrap();
        file.flush().unwrap();

        match rx.recv_timeout(watcher_event_timeout()) {
            Ok(WatchEvent::ConfigChanged) => {}
            other => panic!("expected ConfigChanged, got {other:?}"),
        }
    }

    #[test]
    fn debounce_coalesces_rapid_writes() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "Host one").unwrap();
        file.flush().unwrap();

        let rx = spawn_config_watcher(file.path()).unwrap();
        let settle = if cfg!(target_os = "macos") {
            Duration::from_millis(500)
        } else {
            Duration::from_millis(100)
        };
        thread::sleep(settle);
        for i in 0..5 {
            writeln!(file, "Host line-{i}").unwrap();
            file.flush().unwrap();
            let _ = file.as_file().sync_all();
            thread::sleep(Duration::from_millis(50));
        }

        let window = debounce_burst_timeout();
        let deadline = Instant::now() + window;
        let mut events = 0u32;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(WatchEvent::ConfigChanged) => {
                    events += 1;
                    assert!(
                        events <= 1,
                        "expected single debounced event, got at least {events}"
                    );
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        assert_eq!(
            events, 1,
            "expected exactly one debounced event within {window:?}"
        );
    }

    #[test]
    fn spawn_config_watcher_missing_path_errors() {
        let path = std::env::temp_dir().join(format!("sshub-missing-{}", std::process::id()));
        let err = spawn_config_watcher(&path).unwrap_err();
        assert!(
            err.to_string().contains("watch SSH config"),
            "unexpected error: {err}"
        );
    }
}
