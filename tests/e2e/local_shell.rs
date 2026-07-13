use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps, AppMode};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::LauncherStore;

#[path = "../support/mod.rs"]
mod support;

use support::MockLauncher;

struct EmptyResolver;

impl HostResolver for EmptyResolver {
    fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }

    fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
        Ok(SshHost::new(name))
    }
}

fn fresh_app() -> App {
    // Pin the shell so the spawn never depends on the ambient $SHELL pointing
    // at a real binary (open_local_shell reads it at call time). Both tests in
    // this file set the same value, so parallel execution is harmless, and no
    // other e2e test reads SHELL.
    std::env::set_var("SHELL", "/bin/sh");
    let store = Arc::new(LauncherStore::open_in_memory().unwrap());
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(EmptyResolver),
            metadata: Arc::new(MetadataDb::default()),
            store,
            launcher: Box::new(MockLauncher::new()),
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    app
}

/// The default `Ctrl+Shift+T` binding parses to `Char('t')` with
/// `CONTROL | SHIFT` (the explicit modifiers suppress the bare-uppercase shift
/// rule, so the code stays lowercase). `keyspec_matches` compares the char
/// case-insensitively but requires the modifier set to match exactly.
fn ctrl_shift_t() -> KeyEvent {
    KeyEvent::new(
        KeyCode::Char('t'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    )
}

/// Ctrl+Shift+T on the dashboard opens a local-shell session tab: a session is
/// pushed, labelled "local", made active, and the app enters the connecting
/// view — identical lifecycle to an ssh tab.
#[test]
fn ctrl_shift_t_opens_local_shell_tab() {
    let mut app = fresh_app();
    assert!(app.sessions.is_empty());
    assert!(app.active_session.is_none());

    app.handle_key(ctrl_shift_t()).unwrap();

    // A single session tab was created and labelled "local". `$SHELL` (or the
    // `/bin/sh` fallback) is always present, so the spawn succeeds and the app
    // moves into the connecting phase.
    assert_eq!(app.sessions.len(), 1);
    assert_eq!(app.sessions[0].display_name, "local");
    assert_eq!(app.active_session, Some(0));
    assert_eq!(app.mode, AppMode::Connecting);
}

/// Ctrl+Shift+T while already inside a session opens a *second* local-shell tab
/// rather than replacing the first, matching the multi-tab ssh behaviour.
#[test]
fn ctrl_shift_t_opens_additional_local_shell_from_session() {
    let mut app = fresh_app();
    app.handle_key(ctrl_shift_t()).unwrap();
    assert_eq!(app.sessions.len(), 1);

    // Now focused in the session view; another Ctrl+Shift+T adds a tab.
    app.handle_key(ctrl_shift_t()).unwrap();
    assert_eq!(app.sessions.len(), 2);
    assert_eq!(app.sessions[1].display_name, "local");
    assert_eq!(app.active_session, Some(1));
}

/// Closing the sole local-shell tab tears it down and returns to the dashboard —
/// identical close semantics to an ssh tab (last tab closed => no active session,
/// `AppMode::Normal`).
#[test]
fn closing_last_local_shell_returns_to_dashboard() {
    let mut app = fresh_app();
    app.handle_key(ctrl_shift_t()).unwrap();
    assert_eq!(app.sessions.len(), 1);
    assert_eq!(app.active_session, Some(0));

    app.close_active_session();

    assert!(app.sessions.is_empty());
    assert_eq!(app.active_session, None);
    assert_eq!(app.mode, AppMode::Normal);
}

/// With two local-shell tabs, closing the active one keeps the session view and
/// falls back onto the remaining tab rather than dropping to the dashboard.
#[test]
fn closing_one_of_two_local_shells_keeps_a_tab() {
    let mut app = fresh_app();
    app.handle_key(ctrl_shift_t()).unwrap();
    app.handle_key(ctrl_shift_t()).unwrap();
    assert_eq!(app.sessions.len(), 2);
    assert_eq!(app.active_session, Some(1));

    app.close_active_session();

    assert_eq!(app.sessions.len(), 1);
    assert_eq!(app.sessions[0].display_name, "local");
    assert_eq!(app.active_session, Some(0));
    // A tab remains, so we stay out of the plain dashboard-Normal-with-no-session
    // state; the session lifecycle is preserved.
    assert!(app.active_session.is_some());
}

/// Detaching a live local-shell tab drops back to the dashboard while KEEPING the
/// session alive (tab still present), matching ssh detach behaviour.
#[test]
fn detach_local_shell_keeps_session_alive() {
    let mut app = fresh_app();
    app.handle_key(ctrl_shift_t()).unwrap();
    assert_eq!(app.sessions.len(), 1);

    app.detach_to_dashboard();

    assert_eq!(app.mode, AppMode::Normal);
    // Detach does not tear down the session — the tab is still there to re-enter.
    assert_eq!(app.sessions.len(), 1);
    assert_eq!(app.sessions[0].display_name, "local");
}
