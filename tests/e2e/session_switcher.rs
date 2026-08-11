//! Dashboard → Alt+S → type → arrow → Enter → land in the right session tab.
//! No network: local `sleep` processes stand in for SSH sessions.

use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use sshub::app::{App, AppDeps, AppMode};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::session::{Session, SessionConfig, SessionMeta, SessionPhase};
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::LauncherStore;

struct EmptyResolver;

impl HostResolver for EmptyResolver {
    fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }

    fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
        Ok(SshHost::new(name))
    }
}

fn render_to_buffer(app: &App) -> Buffer {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| sshub::tui::render(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn find_on_row(buffer: &Buffer, y: u16, needle: &str) -> u16 {
    let width = needle.chars().count() as u16;
    for x in buffer.area.x..=buffer.area.right().saturating_sub(width) {
        let rendered: String = (x..x + width)
            .map(|column| buffer[(column, y)].symbol())
            .collect();
        if rendered == needle {
            return x;
        }
    }
    panic!("{needle:?} not rendered on row {y}");
}

#[test]
fn alt_s_filters_open_sessions_and_renders_the_selected_tab() {
    let mut config = AppConfig::default();
    config.appearance.disable_animation = true;

    let mut app = App::new_with_deps(
        config,
        AppDeps {
            resolver: Box::new(EmptyResolver),
            metadata: Arc::new(MetadataDb::default()),
            store: Arc::new(LauncherStore::open_in_memory().unwrap()),
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 120, 40);

    for name in ["web-prod", "db-backup", "db-staging"] {
        app.sessions.push(
            Session::spawn(
                SessionConfig {
                    argv: vec!["sleep".into(), "30".into()],
                    display_name: name.into(),
                    meta: SessionMeta::default(),
                    pending_secret: None,
                    key_push_identity: None,
                    host_name: name.into(),
                },
                40,
                120,
                None,
            )
            .unwrap(),
        );
    }
    app.sessions[2].phase = SessionPhase::Running {
        started_at: Instant::now(),
    };
    app.active_session = Some(0);
    app.mode = AppMode::Normal;

    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT))
        .unwrap();
    assert_eq!(app.mode, AppMode::SessionPicker);

    for c in "db".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()))
            .unwrap();
    }
    let rows = app.session_picker_rows();
    assert_eq!(rows.len(), 2, "db-backup and db-staging must both match");
    assert_eq!(app.session_picker.as_ref().unwrap().selected, 0);

    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::empty()))
        .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
        .unwrap();

    assert_eq!(app.active_session, Some(2));
    assert_eq!(app.mode, AppMode::Session);
    assert!(app.session_picker.is_none());

    let buffer = render_to_buffer(&app);
    let y = buffer.area.y;

    // Hand-transcribed from the frozen palette `src/tui/theme.rs` used to
    // export: `theme::inv()` was `BG_DEEP` on `BRIGHT`. The palette itself is
    // now a test-only witness inside the lib, so the two values are named here
    // rather than imported.
    const LEGACY_BG_DEEP: ratatui::style::Color = ratatui::style::Color::Rgb(0x06, 0x08, 0x0a);
    const LEGACY_BRIGHT: ratatui::style::Color = ratatui::style::Color::Rgb(0xc7, 0xe8, 0xc9);

    let target_x = find_on_row(&buffer, y, "db-staging");
    for x in target_x..target_x + "db-staging".chars().count() as u16 {
        assert_eq!(buffer[(x, y)].fg, LEGACY_BG_DEEP);
        assert_eq!(buffer[(x, y)].bg, LEGACY_BRIGHT);
    }

    let inactive_x = find_on_row(&buffer, y, "db-backup");
    for x in inactive_x..inactive_x + "db-backup".chars().count() as u16 {
        assert_ne!(buffer[(x, y)].bg, LEGACY_BRIGHT);
    }
}
