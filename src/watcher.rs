use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvError, RecvTimeoutError, Sender};
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
        let source = ChannelSource { rx: notify_rx };
        debounce_loop(&source, tx, &config_path, WATCHER_DEBOUNCE);
        drop(watcher);
    });

    Ok(rx)
}

/// Time and input source for the debounce loop. Abstracted so tests inject a
/// scripted event list with a virtual clock instead of sleeping on real time.
trait EventSource: Send {
    fn recv(&self) -> Result<NotifyResult<Event>, RecvError>;
    fn recv_timeout(&self, timeout: Duration) -> Result<NotifyResult<Event>, RecvTimeoutError>;
    fn now(&self) -> Instant;
}

struct ChannelSource {
    rx: Receiver<NotifyResult<Event>>,
}

impl EventSource for ChannelSource {
    fn recv(&self) -> Result<NotifyResult<Event>, RecvError> {
        self.rx.recv()
    }
    fn recv_timeout(&self, timeout: Duration) -> Result<NotifyResult<Event>, RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Fixed-window debounce: the first config-touching event opens a window;
/// all events arriving before `start + window` are swallowed; one
/// `ConfigChanged` is emitted when the window expires.
fn debounce_loop<S: EventSource>(
    source: &S,
    tx: Sender<WatchEvent>,
    config: &Path,
    window: Duration,
) {
    loop {
        match source.recv() {
            Ok(Ok(event)) if is_config_change(&event, config) => {}
            Ok(Ok(_)) => continue,
            Ok(Err(_)) => continue,
            Err(_) => break,
        }

        let deadline = source.now() + window;
        loop {
            let remaining = deadline.saturating_duration_since(source.now());
            if remaining.is_zero() {
                break;
            }
            match source.recv_timeout(remaining) {
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
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::io::Write;
    use std::sync::mpsc::RecvTimeoutError;
    use tempfile::NamedTempFile;

    struct ScriptedSource {
        script: RefCell<VecDeque<(NotifyResult<Event>, Instant)>>,
        clock: Cell<Instant>,
    }

    impl ScriptedSource {
        fn new(script: Vec<(NotifyResult<Event>, Instant)>) -> Self {
            let clock = script
                .first()
                .map(|(_, at)| *at)
                .unwrap_or_else(Instant::now);
            Self {
                script: RefCell::new(VecDeque::from(script)),
                clock: Cell::new(clock),
            }
        }

        fn take_next(&self) -> Option<(NotifyResult<Event>, Instant)> {
            let item = self.script.borrow_mut().pop_front()?;
            self.clock.set(item.1);
            Some(item)
        }

        fn peek_next_at(&self) -> Option<Instant> {
            self.script.borrow().front().map(|(_, at)| *at)
        }
    }

    impl EventSource for ScriptedSource {
        fn recv(&self) -> Result<NotifyResult<Event>, RecvError> {
            match self.take_next() {
                Some((event, _)) => Ok(event),
                None => Err(RecvError),
            }
        }

        fn recv_timeout(&self, timeout: Duration) -> Result<NotifyResult<Event>, RecvTimeoutError> {
            match self.peek_next_at() {
                None => Err(RecvTimeoutError::Disconnected),
                Some(next_at) if next_at < self.clock.get() + timeout => {
                    Ok(self.take_next().unwrap().0)
                }
                Some(_) => {
                    self.clock.set(self.clock.get() + timeout);
                    Err(RecvTimeoutError::Timeout)
                }
            }
        }

        fn now(&self) -> Instant {
            self.clock.get()
        }
    }

    fn run_debounce(
        script: Vec<(NotifyResult<Event>, Instant)>,
        config: &Path,
        window: Duration,
    ) -> Vec<WatchEvent> {
        let source = ScriptedSource::new(script);
        let (tx, rx) = mpsc::channel();
        debounce_loop(&source, tx, config, window);
        rx.try_iter().collect()
    }

    fn config_event(path: &Path) -> Event {
        Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![path.to_path_buf()],
            attrs: Default::default(),
        }
    }

    fn access_event(path: &Path) -> Event {
        Event {
            kind: EventKind::Access(notify::event::AccessKind::Read),
            paths: vec![path.to_path_buf()],
            attrs: Default::default(),
        }
    }

    fn create_event(path: &Path) -> Event {
        Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![path.to_path_buf()],
            attrs: Default::default(),
        }
    }

    fn remove_event(path: &Path) -> Event {
        Event {
            kind: EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![path.to_path_buf()],
            attrs: Default::default(),
        }
    }

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

    // Exercises real FSEvents/inotify delivery. On macOS CI, FSEvents delivery
    // for a temp file is unreliable (events arrive seconds late or not at all in
    // the sandbox), which made this flake repeatedly despite generous timeouts.
    // The watcher wiring is deterministically covered on Linux (inotify); skip
    // the real-delivery assertion on macOS. Run locally with `--ignored`.
    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "FSEvents delivery is unreliable on macOS CI; covered on Linux"
    )]
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

    // Same real-FSEvents caveat as spawn_config_watcher_emits_after_write: the
    // debounce coalescing logic is validated on Linux; macOS CI can't deliver
    // FSEvents reliably enough to assert anything about counts within a bound.
    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "FSEvents delivery is unreliable on macOS CI; covered on Linux"
    )]
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
        const WRITES: u32 = 5;
        for i in 0..WRITES {
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
                Ok(WatchEvent::ConfigChanged) => events += 1,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        // What the debouncer promises is that a burst does not arrive one event
        // per write, not that it always collapses to exactly one. The window is
        // wall-clock time and the writes are real inotify events, so a loaded
        // machine can straddle the boundary and deliver two: asserting `== 1`
        // failed at random on CI (issue #62).
        //
        // Both bounds still catch a break: zero means delivery stopped, WRITES
        // means nothing was coalesced at all.
        assert!(
            events >= 1,
            "expected at least one event from {WRITES} writes within {window:?}"
        );
        assert!(
            events < WRITES,
            "expected the burst coalesced, got {events} events from {WRITES} writes"
        );
    }

    #[test]
    fn debounce_burst_collapses_to_one_event() {
        let config = Path::new("/tmp/config");
        let window = Duration::from_millis(300);
        let t0 = Instant::now();

        let script: Vec<_> = (0..5)
            .map(|i| (Ok(config_event(config)), t0 + Duration::from_millis(i * 50)))
            .collect();

        let emitted = run_debounce(script, config, window);
        assert_eq!(emitted, vec![WatchEvent::ConfigChanged]);
    }

    #[test]
    fn debounce_write_after_window_opens_new_event() {
        let config = Path::new("/tmp/config");
        let window = Duration::from_millis(300);
        let t0 = Instant::now();

        let script = vec![
            (Ok(config_event(config)), t0),
            (Ok(config_event(config)), t0 + Duration::from_millis(100)),
            (Ok(config_event(config)), t0 + Duration::from_millis(500)),
        ];

        let emitted = run_debounce(script, config, window);
        assert_eq!(
            emitted,
            vec![WatchEvent::ConfigChanged, WatchEvent::ConfigChanged]
        );
    }

    #[test]
    fn debounce_write_at_exact_deadline_opens_new_event() {
        let config = Path::new("/tmp/config");
        let window = Duration::from_millis(300);
        let t0 = Instant::now();

        let script = vec![
            (Ok(config_event(config)), t0),
            (Ok(config_event(config)), t0 + window),
        ];

        let emitted = run_debounce(script, config, window);
        assert_eq!(
            emitted,
            vec![WatchEvent::ConfigChanged, WatchEvent::ConfigChanged]
        );
    }

    #[test]
    fn debounce_ignores_other_files() {
        let config = Path::new("/tmp/config");
        let other = Path::new("/tmp/other");
        let window = Duration::from_millis(300);
        let t0 = Instant::now();

        let script = vec![
            (Ok(config_event(other)), t0),
            (Ok(config_event(other)), t0 + Duration::from_millis(100)),
        ];

        let emitted = run_debounce(script, config, window);
        assert!(emitted.is_empty());
    }

    #[test]
    fn debounce_ignores_access_kind() {
        let config = Path::new("/tmp/config");
        let window = Duration::from_millis(300);
        let t0 = Instant::now();

        let script = vec![
            (Ok(access_event(config)), t0),
            (Ok(access_event(config)), t0 + Duration::from_millis(100)),
        ];

        let emitted = run_debounce(script, config, window);
        assert!(emitted.is_empty());
    }

    #[test]
    fn debounce_mixed_files_only_config_counted() {
        let config = Path::new("/tmp/config");
        let other = Path::new("/tmp/other");
        let window = Duration::from_millis(300);
        let t0 = Instant::now();

        let script = vec![
            (Ok(config_event(other)), t0),
            (Ok(config_event(config)), t0 + Duration::from_millis(10)),
            (Ok(config_event(other)), t0 + Duration::from_millis(20)),
            (Ok(config_event(config)), t0 + Duration::from_millis(30)),
        ];

        let emitted = run_debounce(script, config, window);
        assert_eq!(emitted, vec![WatchEvent::ConfigChanged]);
    }

    #[test]
    fn debounce_access_inside_window_does_not_extend_it() {
        let config = Path::new("/tmp/config");
        let window = Duration::from_millis(300);
        let t0 = Instant::now();

        let script = vec![
            (Ok(config_event(config)), t0),
            (Ok(access_event(config)), t0 + Duration::from_millis(200)),
            (Ok(config_event(config)), t0 + Duration::from_millis(400)),
        ];

        let emitted = run_debounce(script, config, window);
        assert_eq!(
            emitted,
            vec![WatchEvent::ConfigChanged, WatchEvent::ConfigChanged]
        );
    }

    #[test]
    fn debounce_chained_writes_spanning_two_windows_emit_twice() {
        let config = Path::new("/tmp/config");
        let window = Duration::from_millis(300);
        let t0 = Instant::now();

        let script = vec![
            (Ok(config_event(config)), t0),
            (Ok(config_event(config)), t0 + Duration::from_millis(200)),
            (Ok(config_event(config)), t0 + Duration::from_millis(400)),
        ];

        let emitted = run_debounce(script, config, window);
        assert_eq!(
            emitted,
            vec![WatchEvent::ConfigChanged, WatchEvent::ConfigChanged]
        );
    }

    #[test]
    fn debounce_create_and_remove_kinds_trigger_config_change() {
        let config = Path::new("/tmp/config");
        let window = Duration::from_millis(300);
        let t0 = Instant::now();

        let script = vec![
            (Ok(create_event(config)), t0),
            (Ok(remove_event(config)), t0 + Duration::from_millis(500)),
        ];

        let emitted = run_debounce(script, config, window);
        assert_eq!(
            emitted,
            vec![WatchEvent::ConfigChanged, WatchEvent::ConfigChanged]
        );
    }

    #[test]
    fn debounce_notify_error_is_nonfatal() {
        let config = Path::new("/tmp/config");
        let window = Duration::from_millis(300);
        let t0 = Instant::now();

        let notify_err = notify::Error::generic("inotify queue overflow");
        let script = vec![
            (Err(notify_err), t0),
            (Ok(config_event(config)), t0 + Duration::from_millis(100)),
        ];

        let emitted = run_debounce(script, config, window);
        assert_eq!(emitted, vec![WatchEvent::ConfigChanged]);
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
