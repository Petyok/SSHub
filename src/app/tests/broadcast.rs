use super::*;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use crate::broadcast::{BroadcastEvent, BroadcastTask, HostState};

/// Build an `App` whose store holds a `prod` group with two managed hosts (one
/// tagged `db`), so the broadcast wizard can resolve real targets/candidates.
fn app_with_targets() -> App {
    let store = test_store();
    let g = store
        .create_group(&crate::store::NewHostGroup {
            name: "prod".into(),
            sort_order: 0,
            default_identity_id: None,
            parent_id: None,
        })
        .unwrap();

    let mut web = crate::store::NewHost::launcher("web", "10.0.0.1");
    web.group_id = Some(g.id);
    web.tags = vec!["db".into()];
    store.create_host(&web).unwrap();

    let mut api = crate::store::NewHost::launcher("api", "10.0.0.2");
    api.group_id = Some(g.id);
    store.create_host(&api).unwrap();

    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(MockResolver::new(vec![])),
            metadata: Arc::new(MetadataDb::default()),
            store: Arc::clone(&store),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    app
}

/// Hand-construct a live `BroadcastState` (no real ssh): seeds `Pending` rows
/// for the given `(host_id, host_name)` tasks and returns the state plus the
/// event `Sender` and a clone of the shared cancel flag the test drives.
fn make_state(
    command: &str,
    tasks: &[(i64, &str)],
) -> (
    BroadcastState,
    mpsc::Sender<BroadcastEvent>,
    Arc<AtomicBool>,
) {
    let (tx, rx) = mpsc::channel::<BroadcastEvent>();
    let bcast_tasks: Vec<BroadcastTask> = tasks
        .iter()
        .map(|(id, name)| BroadcastTask {
            host_id: *id,
            host_name: name.to_string(),
            argv: vec!["ssh".to_string(), name.to_string()],
            secret: None,
        })
        .collect();
    let results = crate::broadcast::seed_results(&bcast_tasks);
    let cancel = Arc::new(AtomicBool::new(false));
    let state = BroadcastState {
        target_label: "group: prod".into(),
        command: command.into(),
        results,
        rx,
        cancel: Arc::clone(&cancel),
        concurrency: 2,
        phase: BroadcastPhase::Running,
        anim: None,
        audit_written: false,
    };
    (state, tx, cancel)
}

#[test]
pub(crate) fn wizard_group_target_flow_pick_command_preview() {
    let mut app = app_with_targets();

    app.open_broadcast();
    assert_eq!(app.mode, AppMode::BroadcastPickTarget);
    let setup = app.broadcast_setup.as_ref().expect("wizard opened");
    assert!(
        setup
            .options
            .iter()
            .any(|o| matches!(o, BroadcastTarget::Group { label, .. } if label == "prod")),
        "group option present"
    );
    assert!(
        setup
            .options
            .iter()
            .any(|o| matches!(o, BroadcastTarget::Tag { name } if name == "db")),
        "tag option present"
    );

    // Enter on the first option (group prod) resolves both managed hosts.
    app.handle_key_broadcast_pick(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::BroadcastCommand);
    let setup = app.broadcast_setup.as_ref().unwrap();
    assert_eq!(setup.target_label, "group: prod");
    assert_eq!(setup.candidates.len(), 2);
    assert!(
        setup.candidates.iter().all(|c| c.selected),
        "candidates selected by default"
    );

    // Type a command and cross the preview barrier.
    for c in "uptime".chars() {
        app.handle_key_broadcast_command(key_char(c)).unwrap();
    }
    assert_eq!(app.broadcast_setup.as_ref().unwrap().command, "uptime");
    app.handle_key_broadcast_command(key(KeyCode::Enter))
        .unwrap();
    assert_eq!(app.mode, AppMode::BroadcastPreview);

    // 'e' enters edit-targets; Space deselects the highlighted host.
    app.handle_key_broadcast_preview(key_char('e')).unwrap();
    assert!(app.broadcast_setup.as_ref().unwrap().edit_targets);
    app.handle_key_broadcast_preview(key_char(' ')).unwrap();
    assert!(
        !app.broadcast_setup.as_ref().unwrap().candidates[0].selected,
        "Space toggled the highlighted candidate off"
    );

    // 'e' leaves edit mode again; 'n' closes the whole wizard.
    app.handle_key_broadcast_preview(key_char('e')).unwrap();
    assert!(!app.broadcast_setup.as_ref().unwrap().edit_targets);
    app.handle_key_broadcast_preview(key_char('n')).unwrap();
    assert!(app.broadcast_setup.is_none());
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
pub(crate) fn wizard_tag_target_resolves_only_tagged_hosts() {
    let mut app = app_with_targets();
    app.open_broadcast();

    // options == [Group prod, Tag db]; move down onto the tag row, then Enter.
    app.handle_key_broadcast_pick(key(KeyCode::Down)).unwrap();
    app.handle_key_broadcast_pick(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.mode, AppMode::BroadcastCommand);
    let setup = app.broadcast_setup.as_ref().unwrap();
    assert_eq!(setup.target_label, "#db");
    assert_eq!(setup.candidates.len(), 1, "only the #db host matches");
    assert_eq!(setup.candidates[0].host_name, "web");
}

#[test]
pub(crate) fn wizard_pick_esc_closes() {
    let mut app = app_with_targets();
    app.open_broadcast();
    app.handle_key_broadcast_pick(key(KeyCode::Esc)).unwrap();
    assert!(app.broadcast_setup.is_none());
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
pub(crate) fn wizard_preview_esc_closes() {
    let mut app = app_with_targets();
    app.open_broadcast();
    app.handle_key_broadcast_pick(key(KeyCode::Enter)).unwrap();
    for c in "ls".chars() {
        app.handle_key_broadcast_command(key_char(c)).unwrap();
    }
    app.handle_key_broadcast_command(key(KeyCode::Enter))
        .unwrap();
    assert_eq!(app.mode, AppMode::BroadcastPreview);

    app.handle_key_broadcast_preview(key(KeyCode::Esc)).unwrap();
    assert!(app.broadcast_setup.is_none());
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
pub(crate) fn open_broadcast_refuses_while_run_in_progress() {
    let mut app = app_with_targets();
    let (state, _tx, _cancel) = make_state("echo hi", &[(1, "web"), (2, "api")]);
    app.broadcast = Some(state);

    app.open_broadcast();

    assert!(
        app.broadcast_setup.is_none(),
        "wizard must not open over a live run"
    );
    assert!(app.host_notice.is_some(), "surfaces a notice");
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
pub(crate) fn tick_broadcast_folds_events_settles_and_audits_once() {
    let mut app = test_app(vec![]);
    // Keep the panel unfocused so the settle countdown can't flip to Paused.
    app.focused_panel = PanelId::Hosts;

    let (state, tx, _cancel) = make_state("systemctl restart nginx", &[(1, "web"), (2, "api")]);
    app.broadcast = Some(state);

    tx.send(BroadcastEvent::Finished {
        host_id: 1,
        exit: 0,
        stdout: "ok".into(),
        stderr: String::new(),
    })
    .unwrap();
    tx.send(BroadcastEvent::Failed {
        host_id: 2,
        reason: "connection refused".into(),
    })
    .unwrap();

    app.tick_broadcast().unwrap();

    let bc = app
        .broadcast
        .as_ref()
        .expect("run stays docked while settling");
    assert_eq!(bc.results[0].state, HostState::Done { exit: 0 });
    assert!(matches!(bc.results[1].state, HostState::Failed { .. }));
    assert!(
        matches!(bc.phase, BroadcastPhase::Settling { .. }),
        "all-terminal arms the settle countdown"
    );
    assert!(bc.audit_written, "audit guard flipped");

    let events = app.store.list_auth_events(10).unwrap();
    let bcast: Vec<_> = events
        .iter()
        .filter(|e| e.via.as_deref() == Some("broadcast"))
        .collect();
    assert_eq!(bcast.len(), 2, "one audit row per host");

    let web = bcast.iter().find(|e| e.host_name == "web").unwrap();
    assert_eq!(web.status, "launched");
    assert!(
        web.note
            .as_deref()
            .unwrap()
            .contains("systemctl restart nginx"),
        "command echoed in the note"
    );

    let api = bcast.iter().find(|e| e.host_name == "api").unwrap();
    assert_eq!(api.status, "fail");
    let api_note = api.note.as_deref().unwrap();
    assert!(api_note.contains("systemctl restart nginx"));
    assert!(
        api_note.contains("connection refused"),
        "reason in the note"
    );

    // A second tick must not duplicate the audit rows.
    app.tick_broadcast().unwrap();
    let again = app.store.list_auth_events(10).unwrap();
    assert_eq!(
        again
            .iter()
            .filter(|e| e.via.as_deref() == Some("broadcast"))
            .count(),
        2,
        "audit is written exactly once"
    );
}

#[test]
pub(crate) fn cancel_broadcast_sets_the_shared_flag() {
    let mut app = test_app(vec![]);
    let (state, _tx, cancel) = make_state("id", &[(1, "web")]);
    app.broadcast = Some(state);

    assert!(!cancel.load(Ordering::Relaxed));
    app.cancel_broadcast();
    assert!(
        cancel.load(Ordering::Relaxed),
        "cancel flips the shared AtomicBool"
    );
}
