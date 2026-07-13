//! E2E: ad-hoc "connect without saving" flow through the `/` palette.
//!
//! Drives a headless [`App`] via public key events. The heavy pure-parsing and
//! argv-shape assertions live in the colocated unit tests in
//! `src/app/adhoc.rs`; here we verify the palette wiring end-to-end: an ad-hoc
//! target that matches no saved host populates `app.palette_adhoc`, a query that
//! matches a saved host suppresses it, and pressing Enter leaves the palette
//! (resilient to ssh spawning or failing to spawn in CI).

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sshub::app::{App, AppDeps, AppMode};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::{LauncherStore, NewHost};

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

fn key_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

/// One saved managed host named "web-prod".
fn app_with_saved_host() -> App {
    let store = Arc::new(LauncherStore::open_in_memory().unwrap());
    store
        .create_host(&NewHost::launcher("web-prod", "10.0.0.1"))
        .unwrap();

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

fn open_palette_and_type(app: &mut App, query: &str) {
    app.handle_key(key_char('/')).unwrap();
    assert_eq!(app.mode, AppMode::Palette);
    for c in query.chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    // The palette char handler calls `rebuild_palette_results()` per keystroke,
    // and the integrator wires the `palette_adhoc` refresh into that path, so
    // `app.palette_adhoc` is up to date once the query is fully typed.
}

#[test]
fn adhoc_target_offered_when_no_saved_host_matches() {
    let mut app = app_with_saved_host();
    open_palette_and_type(&mut app, "root@198.51.100.9");

    // No saved host matches this destination, so the ad-hoc row is offered.
    let adhoc = app
        .palette_adhoc
        .as_ref()
        .expect("ad-hoc target should be offered");
    assert_eq!(adhoc.label(), "root@198.51.100.9");
    // No fuzzy match either.
    assert_eq!(app.palette_results.len(), 0);
}

#[test]
fn adhoc_target_suppressed_when_query_matches_saved_host() {
    let mut app = app_with_saved_host();
    open_palette_and_type(&mut app, "web-prod");

    // The query names a saved host, so no ad-hoc "connect without saving" row.
    assert!(app.palette_adhoc.is_none());
}

#[test]
fn adhoc_target_suppressed_for_unparseable_query() {
    let mut app = app_with_saved_host();
    open_palette_and_type(&mut app, "has space");

    assert!(app.palette_adhoc.is_none());
}

#[test]
fn enter_on_adhoc_row_leaves_palette() {
    let mut app = app_with_saved_host();
    open_palette_and_type(&mut app, "root@198.51.100.9");
    assert!(app.palette_adhoc.is_some());

    // With zero fuzzy results, the selection index (0) already equals the
    // ad-hoc virtual index (filtered.len() == 0), so Enter targets it.
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    // Whether ssh actually spawns (unlikely in CI) or fails, the palette must
    // close — connect_adhoc sets mode = Normal before spawning, and a spawn
    // failure leaves it Normal rather than reverting to Palette.
    assert_ne!(app.mode, AppMode::Palette);
}
