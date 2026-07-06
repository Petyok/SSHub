pub mod app;
pub mod config;
pub mod credentials;
pub mod import;
pub mod launcher;
pub mod metadata;
pub mod ping;
pub mod search;
pub mod secure_fs;
pub mod session;
pub mod ssh;
pub mod store;
pub mod text_input;
pub mod tui;
pub mod tunnel;
pub mod watcher;

pub use app::{
    App, AppDeps, AppMode, AuditFilter, AuditRange, DetailEditField, HostDetailEdit, HostEntry,
    HostFormEdit, HostFormField, HostGroupSection, IdentityFormEdit, IdentityFormField, SortMode,
    UNGROUPED_LABEL,
};
pub use config::AppConfig;
pub use metadata::HostMetadata;
pub use ssh::{export_launcher_hosts, import_ssh_config, HostResolver, ImportReport, SshHost};
pub use store::{
    AuthEvent, DeleteHostOutcome, DeleteIdentityOutcome, HostGroup, HostSource, Identity,
    IdentityUpdate, LauncherStore, ManagedHost, NewHost, NewHostGroup, NewIdentity,
};
pub use watcher::WatchEvent;

use std::io::{stdout, IsTerminal};
use std::panic;
use std::sync::Once;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::Terminal;

const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Delete the launcher database (and any SQLite sidecar files) under the
/// resolved data directory. Returns the paths that were actually removed.
///
/// Only the SSHub-managed database is touched — `~/.ssh/config` and the hosts
/// imported from it are left alone, and they reappear on the next launch.
/// Passwords stored in the OS keyring are not removed (they become orphaned).
pub fn purge_database() -> Result<Vec<std::path::PathBuf>> {
    let base = config::data_dir()?.join("launcher.db");
    let mut removed = Vec::new();
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let path = if suffix.is_empty() {
            base.clone()
        } else {
            let mut s = base.clone().into_os_string();
            s.push(suffix);
            std::path::PathBuf::from(s)
        };
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
            removed.push(path);
        }
    }
    Ok(removed)
}

/// Run the application (entry point for the binary).
pub fn run() -> Result<()> {
    if std::env::var("SSHUB_DRY_RUN").is_ok() || std::env::var("SSH_LAUNCHER_DRY_RUN").is_ok() {
        return Ok(());
    }
    run_app()
}

/// Load config, build [`App`], and run the main event loop.
pub fn run_app() -> Result<()> {
    let config = config::load_config()?;
    let mut app = App::new(config)?;
    attach_config_watcher(&mut app)?;

    let auto_quit = std::env::var("SSHUB_AUTO_QUIT")
        .or_else(|_| std::env::var("SSH_LAUNCHER_AUTO_QUIT"))
        .ok();

    if !stdout().is_terminal() {
        return run_headless_loop(&mut app, auto_quit.as_deref());
    }

    run_terminal_loop(&mut app, auto_quit.as_deref())
}

fn attach_config_watcher(app: &mut App) -> Result<()> {
    let ssh_config = ssh::ssh_config_path()?;
    if !ssh_config.exists() {
        return Ok(());
    }
    match watcher::spawn_config_watcher(&ssh_config) {
        Ok(rx) => app.set_watcher_rx(rx),
        Err(err) => eprintln!("warning: config watcher disabled: {err:#}"),
    }
    Ok(())
}

fn run_animation<B: ratatui::backend::Backend + std::io::Write>(
    terminal: &mut Terminal<B>,
) -> Result<()> {
    let size = terminal.size()?;
    let state = tui::animation::AnimationState::new(size.width, size.height);

    loop {
        terminal.draw(|frame| state.render(frame))?;
        if event::poll(Duration::from_millis(33))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Esc | KeyCode::Char('q') => {
                        break;
                    }
                    _ => {}
                }
            }
        }
        // After animation completes, keep rendering (blinking elements)
        // but only Enter/Space/Esc/q will exit the loop above.
    }
    Ok(())
}

fn run_terminal_loop(app: &mut App, auto_quit: Option<&str>) -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let _guard = TerminalGuard::new();
    install_panic_hook();

    // Run startup animation (skip in CI/headless or when disabled in config)
    if auto_quit.is_none() && !app.config.appearance.disable_animation {
        run_animation(&mut terminal)?;
    }

    let mut last_size: Option<(u16, u16)> = None;
    loop {
        let sz = terminal.size()?;
        app.terminal_area = ratatui::layout::Rect::new(0, 0, sz.width, sz.height);

        // Drain every session's PTY this frame so background tabs accumulate
        // output and don't fall behind. Resize all of them when the host
        // terminal changes size — every tab shares the same body area.
        let resized = last_size != Some((sz.width, sz.height));
        let mut diag_entries: Vec<(String, String)> = Vec::new();
        for s in app.sessions.iter_mut() {
            s.drain();
            if resized {
                s.resize(sz.height, sz.width);
            }
            for line in s.take_diagnostics() {
                diag_entries.push((s.display_name.clone(), line));
            }
        }
        for (host_name, line) in diag_entries {
            app.push_ssh_log(crate::ssh::probe::SshLogEntry {
                host_name,
                line,
                level: crate::ssh::probe::LogLevel::Info,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
            });
        }
        // Promote `mode` from Connecting → Session once the visible tab's
        // child has produced output, so Esc-cancel semantics flip correctly.
        if let Some(active) = app.active_session() {
            if matches!(active.phase, session::SessionPhase::Running { .. })
                && app.mode == AppMode::Connecting
            {
                app.mode = AppMode::Session;
            }
        }
        last_size = Some((sz.width, sz.height));

        // Mouse capture stays on continuously so the scroll wheel always
        // reaches sshub (driving scrollback when the remote isn't using the
        // mouse, or forwarded into the remote when vim/htop/fzf have asked
        // for mouse via DECSET). Selection works via kitty's built-in
        // override: holding Shift while dragging bypasses the app's mouse
        // capture for native text selection.

        terminal.draw(|frame| tui::render(frame, app))?;

        if auto_quit.is_some() {
            apply_auto_quit(app, auto_quit)?;
            break;
        }

        poll_keys_and_watcher(app)?;

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn run_headless_loop(app: &mut App, auto_quit: Option<&str>) -> Result<()> {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| tui::render(frame, app))?;

    match auto_quit {
        Some(mode) => {
            apply_auto_quit(app, Some(mode))?;
            Ok(())
        }
        None => anyhow::bail!(
            "sshub requires an interactive terminal (use --dry-run or SSHUB_AUTO_QUIT for CI smoke)"
        ),
    }
}

fn apply_auto_quit(app: &mut App, auto_quit: Option<&str>) -> Result<()> {
    match auto_quit {
        Some("q") => {
            app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty()))?;
            // 'q' may raise the quit-confirmation dialog; confirm it.
            if app.mode == AppMode::ConfirmQuit {
                app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()))?;
            }
            if !app.should_quit {
                anyhow::bail!("auto-quit with 'q' did not set should_quit");
            }
        }
        Some(_) => {}
        None => {}
    }
    Ok(())
}

fn poll_keys_and_watcher(app: &mut App) -> Result<()> {
    if event::poll(POLL_INTERVAL)? {
        // Drain everything already queued: one event per 50ms frame makes
        // paste into an embedded session crawl at ~20 chars/sec.
        loop {
            match event::read()? {
                Event::Key(key) => app.handle_key(key)?,
                Event::Mouse(mouse) => app.handle_mouse(mouse)?,
                _ => {}
            }
            if app.should_quit || !event::poll(std::time::Duration::ZERO)? {
                break;
            }
        }
    }

    let mut config_changed = false;
    if let Some(rx) = app.watcher_rx.as_ref() {
        while rx.try_recv().is_ok() {
            config_changed = true;
        }
    }
    if config_changed {
        app.reload_hosts()?;
    }

    // Drain ping results from background worker
    if let Some(rx) = app.ping_rx.as_ref() {
        while let Ok(result) = rx.try_recv() {
            let entry = app.ping_data.entry(result.host_name.clone()).or_default();
            if let Some(ms) = result.latency_ms {
                entry.push(ms);
                if entry.len() > 30 {
                    entry.remove(0);
                } // rolling 30 samples
            }
        }
    }

    // Drain SSH probe log entries from background worker
    if let Some(rx) = app.probe_rx.as_ref() {
        let entries: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        for entry in entries {
            app.push_ssh_log(entry);
        }
    }

    // Check tunnel health
    app.tunnel_manager.check_health();

    // Refresh auth events cache periodically
    app.refresh_auth_cache();

    Ok(())
}

struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    fn new() -> Self {
        Self { active: true }
    }

    fn restore(&mut self) -> Result<()> {
        if self.active {
            let _ = stdout().execute(DisableMouseCapture);
            disable_raw_mode()?;
            stdout().execute(LeaveAlternateScreen)?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn install_panic_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = stdout().execute(LeaveAlternateScreen);
            default_hook(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::MetadataStore;

    #[test]
    fn host_entry_pairs_host_and_metadata() {
        let entry = HostEntry::new(SshHost::new("web"));
        assert_eq!(entry.name(), "web");
        if let HostEntry::Legacy { meta, .. } = &entry {
            assert_eq!(meta.host_name, "web");
        } else {
            panic!("expected legacy entry");
        }
    }

    #[test]
    fn shared_contracts_compile() {
        use std::fs;
        use std::path::PathBuf;
        use std::sync::Arc;

        use crate::app::AppDeps;

        use crate::store::LauncherStore;

        struct FixtureResolver {
            config_path: PathBuf,
            ssh_g_dir: PathBuf,
        }

        impl HostResolver for FixtureResolver {
            fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
                let content = fs::read_to_string(&self.config_path)?;
                Ok(ssh::parse_host_aliases(&content))
            }

            fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
                let path = self.ssh_g_dir.join(format!("{name}.txt"));
                let output = fs::read_to_string(&path)?;
                Ok(ssh::parse_ssh_g_output(name, &output))
            }
        }

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let resolver = FixtureResolver {
            config_path: root.join("tests/fixtures/ssh_config"),
            ssh_g_dir: root.join("tests/fixtures/ssh_g"),
        };
        let metadata: Arc<dyn MetadataStore> = Arc::new(metadata::MetadataDb::default());
        let launcher = launcher::launcher_from_config(&AppConfig::default()).unwrap();
        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(resolver),
                metadata: Arc::clone(&metadata),
                store: Arc::new(LauncherStore::open_in_memory().unwrap()),
                launcher,
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.reload_hosts().unwrap();
        assert!(!app.hosts.is_empty());

        let _: Box<dyn HostResolver> = Box::new(ssh::SshConfigResolver::default());
        let _: Box<dyn launcher::TerminalLauncher> =
            launcher::launcher_from_config(&AppConfig::default()).unwrap();
        let _: Box<dyn MetadataStore> = Box::new(metadata::MetadataDb::default());
    }

    // Minimal resolver that returns no hosts
    struct NoopResolver;
    impl crate::ssh::HostResolver for NoopResolver {
        fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }
        fn resolve_host(&self, _name: &str) -> anyhow::Result<crate::ssh::SshHost> {
            anyhow::bail!("no hosts")
        }
    }

    // Minimal launcher that does nothing
    struct NoopLauncher;
    impl crate::launcher::TerminalLauncher for NoopLauncher {
        fn launch_ssh_argv(&self, _argv: &[String]) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn headless_auto_quit_q_sets_should_quit() {
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(
            crate::store::LauncherStore::open(dir.path().join("launcher.db")).unwrap(),
        );
        let metadata: std::sync::Arc<dyn MetadataStore> =
            std::sync::Arc::new(crate::metadata::MetadataDb::default());
        let app_deps = crate::app::AppDeps {
            resolver: Box::new(NoopResolver),
            metadata,
            store,
            launcher: Box::new(NoopLauncher),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        };
        let mut app = crate::app::App::new_with_deps(crate::config::AppConfig::default(), app_deps);
        run_headless_loop(&mut app, Some("q")).unwrap();
        assert!(app.should_quit);
    }
}
