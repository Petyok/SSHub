//! Workspaces (#73): save a split layout under a name, reopen it, delete it.

use super::*;
use crate::app::panes::{PaneTree, SplitDir};
use crate::app::workspaces::SavedPane;

const CHILD: &str = "SSHUB_TEST_WORKSPACE_CHILD";

fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key_char(c)).unwrap();
    }
}

#[test]
fn a_workspace_saves_the_layout_and_reopens_it_at_each_panes_size() {
    // Local shells come from `$SHELL`, which is process-global: run the body
    // in a child process (this binary, filtered to this test) with a known
    // shell, so a parallel test can never see the change.
    if std::env::var(CHILD).is_err() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "app::tests::workspaces::a_workspace_saves_the_layout_and_reopens_it_at_each_panes_size",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("SHELL", "/bin/sh")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let mut app = test_app(vec![("web", host("web"))]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 120, 40);
    app.open_local_shell().unwrap();
    app.open_local_shell().unwrap();
    // Stands in for a connection to the saved host `web`.
    app.spawn_embedded_session(
        vec!["sleep".into(), "30".into()],
        "web".into(),
        crate::session::SessionMeta::default(),
        None,
        "web",
        false,
    )
    .unwrap();
    app.active_session = Some(0);
    app.mode = AppMode::Session;
    app.handle_key(alt('v')).unwrap(); // local | local
    app.handle_key(alt('j')).unwrap(); // local | (local over web)

    // Save: type a name, the first row offers to save under it.
    app.handle_key(alt('e')).unwrap();
    assert_eq!(app.mode, AppMode::SessionPicker);
    type_text(&mut app, "ops");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let saved = app.store().list_workspaces().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "ops");
    let layout: PaneTree<SavedPane> = serde_json::from_str(&saved[0].layout).unwrap();
    let mut want = PaneTree::split_of(SplitDir::Vertical, SavedPane::Local, SavedPane::Local);
    assert!(want.split(1, SplitDir::Horizontal, SavedPane::Host("web".into())));
    assert_eq!(layout, want);

    // Reopen from the dashboard with nothing open. `web` is no longer in the
    // host list, so its pane comes back empty and the user is told why.
    app.shutdown_all();
    app.panes = None;
    app.mode = AppMode::Normal;
    app.hosts.clear();
    app.handle_key(alt('e')).unwrap();
    assert_eq!(
        app.session_picker_rows()[0].endpoint,
        "3 panes: local shell, local shell, web"
    );
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.sessions.len(), 2);
    let views = app.pane_views(app.terminal_area);
    assert_eq!(views.len(), 3);
    assert_eq!(views[2].session, None);
    // Each shell was spawned at its own pane's grid: nothing to resize.
    for view in &views[..2] {
        let session = &app.sessions[view.session.unwrap()];
        assert_eq!(
            session.parser.size(),
            crate::session::render::grid_of(view.rect)
        );
    }
    assert!(
        app.host_notice
            .as_deref()
            .unwrap_or_default()
            .contains("web")
            || app
                .active_session()
                .and_then(|s| s.copy_notice.as_ref())
                .is_some_and(|(m, _)| m.contains("web"))
    );

    // Delete it from the picker.
    app.mode = AppMode::Session;
    app.handle_key(alt('e')).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.store().list_workspaces().unwrap().is_empty());
}
