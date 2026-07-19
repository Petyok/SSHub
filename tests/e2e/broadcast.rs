//! Offline e2e for broadcast mode (#3): drives the pre-run wizard (pick target →
//! command → preview) through the real key handlers and asserts on the rendered
//! `TestBackend` buffer. Stops BEFORE pressing `y`, which would spawn a real
//! `SshCommandRunner` (actual ssh) — everything up to the barrier is pure UI.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

use sshub::app::{App, AppDeps, AppMode};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::{LauncherStore, NewHost};

/// No ssh_config hosts: the only managed hosts come from the store below.
struct EmptyResolver;

impl HostResolver for EmptyResolver {
    fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }

    fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
        Ok(SshHost::new(name))
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn key_char(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key_char(c)).unwrap();
    }
}

/// Two managed hosts sharing the tag `web`, so the broadcast target menu offers
/// `#web` and both hosts resolve as candidates.
fn app_with_tagged_hosts() -> App {
    let store = Arc::new(LauncherStore::open_in_memory().unwrap());
    for (name, addr) in [("web-1", "10.0.0.1"), ("web-2", "10.0.0.2")] {
        store
            .create_host(&NewHost {
                tags: vec!["web".to_string()],
                ..NewHost::launcher(name, addr)
            })
            .unwrap();
    }

    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(EmptyResolver),
            metadata: Arc::new(MetadataDb::default()),
            store,
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    app
}

/// Draw the app into an offline `TestBackend` and return the rendered buffer.
fn render_to_buffer(app: &App) -> Buffer {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| sshub::tui::render(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// True if `needle` appears within any single rendered row.
fn buffer_contains(buffer: &Buffer, needle: &str) -> bool {
    let area = buffer.area;
    for y in area.y..area.y + area.height {
        let line: String = (area.x..area.x + area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if line.contains(needle) {
            return true;
        }
    }
    false
}

#[test]
fn broadcast_wizard_pick_command_preview_then_esc() {
    let mut app = app_with_tagged_hosts();
    assert_eq!(app.hosts.len(), 2);

    // 'b' opens the wizard on the target-pick stage.
    app.handle_key(key_char('b')).unwrap();
    assert_eq!(app.mode, AppMode::BroadcastPickTarget);
    assert!(app.broadcast_setup.is_some());

    // Stage 1 render: the "Broadcast to" menu lists the shared tag target.
    let pick = render_to_buffer(&app);
    assert!(
        buffer_contains(&pick, "Broadcast to"),
        "target picker title missing"
    );
    assert!(buffer_contains(&pick, "#web"), "tag target #web missing");

    // Enter picks the highlighted target (the only option, `#web`) and advances
    // to the command prompt.
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::BroadcastCommand);

    // Type the command and render the prompt.
    type_text(&mut app, "uptime");
    let cmd = render_to_buffer(&app);
    assert!(
        buffer_contains(&cmd, "uptime"),
        "typed command not echoed in the prompt"
    );
    assert!(
        buffer_contains(&cmd, "#web"),
        "target label missing from the command prompt"
    );

    // Enter advances to the preview barrier (does NOT start the run yet).
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::BroadcastPreview);
    // No run has been spawned — the wizard is still staged, panel is absent.
    assert!(app.broadcast.is_none());
    assert!(app.broadcast_setup.is_some());

    // Stage 3 render: the command, both target host names, and the [y]/[e]/[N]
    // barrier are all shown.
    let preview = render_to_buffer(&app);
    assert!(
        buffer_contains(&preview, "uptime"),
        "command missing from preview"
    );
    assert!(
        buffer_contains(&preview, "web-1"),
        "host web-1 missing from preview"
    );
    assert!(
        buffer_contains(&preview, "web-2"),
        "host web-2 missing from preview"
    );
    assert!(
        buffer_contains(&preview, "[y]"),
        "confirm barrier [y] missing"
    );
    assert!(buffer_contains(&preview, "[e]"), "edit barrier [e] missing");
    assert!(
        buffer_contains(&preview, "[N]"),
        "cancel barrier [N] missing"
    );

    // STOP before pressing 'y' (that spawns real ssh). Esc from the preview
    // closes the wizard cleanly: back to Normal, staged setup cleared, and still
    // no live run.
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.broadcast_setup.is_none());
    assert!(app.broadcast.is_none());
}
