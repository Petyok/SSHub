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

    #[test]
    fn spawn_config_watcher_emits_after_write() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "Host alpha").unwrap();
        file.flush().unwrap();

        let rx = spawn_config_watcher(file.path()).unwrap();
        writeln!(file, "Host beta").unwrap();
        file.flush().unwrap();

        match rx.recv_timeout(WATCHER_DEBOUNCE + Duration::from_secs(2)) {
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
        for i in 0..5 {
            writeln!(file, "Host line-{i}").unwrap();
            file.flush().unwrap();
            thread::sleep(Duration::from_millis(20));
        }

        let first = rx
            .recv_timeout(WATCHER_DEBOUNCE + Duration::from_secs(2))
            .expect("first debounced event");
        assert_eq!(first, WatchEvent::ConfigChanged);

        match rx.recv_timeout(WATCHER_DEBOUNCE) {
            Err(RecvTimeoutError::Timeout) => {}
            other => panic!("expected single debounced event, got {other:?}"),
        }
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
