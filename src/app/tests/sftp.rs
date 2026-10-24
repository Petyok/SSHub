use super::*;

/// Regression: connecting from the SFTP picker's search must connect to the
/// *filtered* host, not whatever sits at the same index once the filter clears.
///
/// `sftp_connect_selected` used to clear the search query (rebuilding the
/// visible list) *before* reading the selection, which remapped the selected
/// index onto an unfiltered host and connected to the wrong one. The fix reads
/// the selection first. Here we filter down to the last host and assert that is
/// exactly what we connect to (`sftp_host` records the target's name).
#[test]
pub(crate) fn sftp_picker_search_connects_to_filtered_host() {
    let mut app = test_app(vec![
        ("alpha", host("alpha")),
        ("bravo", host("bravo")),
        ("charlie", host("charlie")),
    ]);
    app.active_tab = 1; // SFTP tab

    // Open picker search and narrow to the last host only.
    app.handle_key(key_char('/')).unwrap();
    for c in "charlie".chars() {
        app.handle_key(key_char(c)).unwrap();
    }

    // Enter connects. The worker thread will fail to reach charlie.example.com
    // in the background, but `sftp_host` is set synchronously to the chosen
    // target before any event is drained.
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.sftp_host.as_deref(), Some("charlie"));
}

/// The SFTP progress bar sweeps toward the worker's chunked figure (#35),
/// settles on it, and resets outright when the queue moves to the next file.
#[test]
fn sftp_progress_bar_chases_the_reported_figure() {
    let app = test_app(vec![]);
    let tick = |app: &App| {
        app.sftp_progress_at.set(Some(
            std::time::Instant::now() - std::time::Duration::from_millis(16),
        ));
    };

    // First frame adopts the figure: the bar doesn't sweep in from empty.
    assert_eq!(app.sftp_progress_advance(0.4), 0.4);
    assert!(!app.sftp_progress_moving.get());

    // A chunk lands: the bar closes on it over several frames.
    tick(&app);
    let stepped = app.sftp_progress_advance(0.9);
    assert!(app.sftp_progress_moving.get());
    assert!(
        (0.4..0.9).contains(&stepped),
        "expected a partial sweep, got {stepped}"
    );
    for _ in 0..200 {
        tick(&app);
        app.sftp_progress_advance(0.9);
    }
    assert_eq!(app.sftp_progress_advance(0.9), 0.9);
    assert!(!app.sftp_progress_moving.get());

    // The next (smaller) file reports less progress: snap back rather than
    // sweeping backwards.
    tick(&app);
    assert_eq!(app.sftp_progress_advance(0.05), 0.05);
    assert!(!app.sftp_progress_moving.get());
}

/// A pane changing directory is noticed centrally and stamped with the way it
/// went (#35), whether the change came from the local filesystem or an async
/// remote listing.
#[test]
fn sftp_directory_change_stamps_its_direction() {
    use crate::sftp::model::SftpState;

    let mut app = test_app(vec![]);
    app.sftp = Some(SftpState::new("/srv", "/home/me"));

    // The first listing of a session is not a navigation.
    app.detect_sftp_navigation();
    assert!(app.sftp_nav.iter().all(|n| n.is_none()));

    // Descending into a child is stamped as going deeper.
    app.sftp.as_mut().unwrap().local.cwd = "/home/me/work".into();
    app.detect_sftp_navigation();
    assert_eq!(app.sftp_nav[0].map(|(deeper, _)| deeper), Some(true));
    assert!(app.sftp_nav[1].is_none(), "the other pane stays put");

    // Stepping back out goes the other way.
    app.sftp.as_mut().unwrap().local.cwd = "/home".into();
    app.detect_sftp_navigation();
    assert_eq!(app.sftp_nav[0].map(|(deeper, _)| deeper), Some(false));

    // A remote listing landing later is stamped just the same.
    app.sftp.as_mut().unwrap().remote.cwd = "/srv/www".into();
    app.detect_sftp_navigation();
    assert_eq!(app.sftp_nav[1].map(|(deeper, _)| deeper), Some(true));

    // Disconnecting forgets the paths, so reconnecting isn't a navigation.
    app.sftp = None;
    app.detect_sftp_navigation();
    assert!(app.sftp_nav.iter().all(|n| n.is_none()));
    app.sftp = Some(SftpState::new("/srv", "/home/me"));
    app.detect_sftp_navigation();
    assert!(app.sftp_nav.iter().all(|n| n.is_none()));
}

/// A transfer between two servers is relayed in two legs through a local temp
/// file: the source worker pulls it down, the destination worker pushes it up,
/// and only then does the item leave the queue.
#[test]
fn server_to_server_transfer_relays_in_two_legs() {
    use crate::sftp::model::{Direction, FileEntry, SftpState, Side};
    use crate::sftp::SftpCommand;
    use std::path::PathBuf;

    let mut app = test_app(vec![]);
    let (tx_right, right) = std::sync::mpsc::channel::<SftpCommand>();
    let (tx_left, left) = std::sync::mpsc::channel::<SftpCommand>();
    app.sftp_tx = Some(tx_right);
    app.sftp_tx2 = Some(tx_left);

    let mut state = SftpState::new("/srv", "/data");
    state.left_host = Some("second-host".into());
    state.remote.set_entries(vec![FileEntry {
        name: "dump.sql".into(),
        is_dir: false,
        size: 42,
        is_symlink: false,
        perm: None,
    }]);
    state.remote.selected = state
        .remote
        .entries
        .iter()
        .position(|e| e.name == "dump.sql")
        .unwrap();
    app.sftp = Some(state);

    // Stage right-to-left and run: the first leg goes to the *source* worker
    // as a download into scratch space.
    app.sftp
        .as_mut()
        .unwrap()
        .stage_toward(Side::Local)
        .unwrap();
    app.sftp_run_queue();
    let relay = app.sftp_relay.as_ref().expect("relay armed");
    assert_eq!(relay.leg, RelayLeg::Fetching);
    let scratch = relay.tmp_dir.path().to_path_buf();
    let tmp = scratch.join("dump.sql");
    // The scratch directory is owner-only: the files passing through it are the
    // user's, and /tmp is shared.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&scratch).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "scratch dir is not owner-only");
    }
    match right
        .try_recv()
        .expect("fetch leg dispatched to the source")
    {
        SftpCommand::RunQueue(q) => {
            assert_eq!(q[0].direction, Direction::Download);
            assert_eq!(q[0].src, PathBuf::from("/srv/dump.sql"));
            assert_eq!(q[0].dst, tmp, "fetched into scratch space");
        }
        _ => panic!("expected a queue run"),
    }
    assert!(left.try_recv().is_err(), "destination waits its turn");

    // The fetch lands: the push goes to the destination worker, out of scratch
    // space and into the left pane's directory.
    app.apply_sftp_event(crate::sftp::SftpEvent::QueueDone);
    assert_eq!(app.sftp_relay.as_ref().unwrap().leg, RelayLeg::Pushing);
    match left
        .try_recv()
        .expect("push leg dispatched to the destination")
    {
        SftpCommand::RunQueue(q) => {
            assert_eq!(q[0].direction, Direction::Upload);
            assert_eq!(q[0].src, tmp);
            assert_eq!(q[0].dst, PathBuf::from("/data/dump.sql"));
        }
        _ => panic!("expected a queue run"),
    }

    // The push lands: the item is done, the relay is over and the queue empty.
    app.apply_sftp_event_left(crate::sftp::SftpEvent::QueueDone);
    assert!(app.sftp_relay.is_none(), "relay finished");
    let state = app.sftp.as_ref().unwrap();
    assert!(state.queue.is_empty(), "the relayed item left the queue");
    assert_eq!(state.phase, crate::sftp::model::Phase::Browsing);
    assert!(!tmp.exists(), "scratch copy cleaned up");
    assert!(!scratch.exists(), "scratch directory goes with the relay");
}

/// A failure part-way through a relay stops it and leaves the item queued, so
/// it can be retried rather than silently vanishing.
#[test]
fn failed_relay_leg_stops_and_keeps_the_item() {
    use crate::sftp::model::{FileEntry, SftpState, Side};
    use crate::sftp::SftpCommand;

    let mut app = test_app(vec![]);
    let (tx_right, _right) = std::sync::mpsc::channel::<SftpCommand>();
    let (tx_left, _left) = std::sync::mpsc::channel::<SftpCommand>();
    app.sftp_tx = Some(tx_right);
    app.sftp_tx2 = Some(tx_left);

    let mut state = SftpState::new("/srv", "/data");
    state.left_host = Some("second-host".into());
    state.remote.set_entries(vec![FileEntry {
        name: "dump.sql".into(),
        is_dir: false,
        size: 42,
        is_symlink: false,
        perm: None,
    }]);
    state.remote.selected = 1; // past the ".." row
    app.sftp = Some(state);
    app.sftp
        .as_mut()
        .unwrap()
        .stage_toward(Side::Local)
        .unwrap();
    app.sftp_run_queue();

    app.apply_sftp_event(crate::sftp::SftpEvent::Error("disk full".into()));
    assert!(app.sftp_relay.is_none(), "relay stopped");
    let state = app.sftp.as_ref().unwrap();
    assert_eq!(state.queue.len(), 1, "the item stays queued for a retry");
    assert_eq!(state.phase, crate::sftp::model::Phase::Browsing);
    assert!(state
        .notice
        .as_deref()
        .is_some_and(|n| n.contains("disk full")));
}

/// Regression: a worker finishes its run with a `QueueDone` even after an
/// error. Acting on that used to restart the transfer that had just failed --
/// which failed again, over and over, with the queue stuck on screen.
#[test]
fn failed_relay_is_not_restarted_by_the_trailing_completion() {
    use crate::sftp::model::{FileEntry, SftpState, Side};
    use crate::sftp::SftpCommand;

    let mut app = test_app(vec![]);
    let (tx_right, right) = std::sync::mpsc::channel::<SftpCommand>();
    let (tx_left, _left) = std::sync::mpsc::channel::<SftpCommand>();
    app.sftp_tx = Some(tx_right);
    app.sftp_tx2 = Some(tx_left);

    let mut state = SftpState::new("/srv", "/data");
    state.left_host = Some("second-host".into());
    state.remote.set_entries(vec![FileEntry {
        name: "test2.py".into(),
        is_dir: false,
        size: 7,
        is_symlink: false,
        perm: None,
    }]);
    state.remote.selected = 1; // past the ".." row
    app.sftp = Some(state);
    app.sftp
        .as_mut()
        .unwrap()
        .stage_toward(Side::Local)
        .unwrap();
    app.sftp_run_queue();
    while right.try_recv().is_ok() {}

    app.apply_sftp_event(crate::sftp::SftpEvent::Error("write error".into()));
    app.apply_sftp_event(crate::sftp::SftpEvent::QueueDone);

    assert!(app.sftp_relay.is_none(), "no fresh relay armed");
    assert_eq!(
        app.sftp.as_ref().unwrap().phase,
        crate::sftp::model::Phase::Browsing,
        "the browser settles instead of looking busy forever"
    );
    assert_eq!(
        app.sftp.as_ref().unwrap().queue.len(),
        1,
        "the item stays for a manual retry"
    );
}

/// A plain (non-relayed) run that reports an error stops there too, instead of
/// rolling whatever is left into another pass that fails the same way.
#[test]
fn failed_run_does_not_dispatch_another_pass() {
    use crate::sftp::model::{Direction, FileEntry, QueuedTransfer, SftpState, Side};
    use crate::sftp::SftpCommand;

    let mut app = test_app(vec![]);
    let (tx, rx) = std::sync::mpsc::channel::<SftpCommand>();
    app.sftp_tx = Some(tx);

    let mut state = SftpState::new("/srv", "/data");
    state.remote.set_entries(vec![FileEntry {
        name: "a.bin".into(),
        is_dir: false,
        size: 1,
        is_symlink: false,
        perm: None,
    }]);
    state.remote.selected = 1; // past the ".." row
    app.sftp = Some(state);
    app.sftp
        .as_mut()
        .unwrap()
        .stage_toward(Side::Local)
        .unwrap();
    app.sftp_run_queue();
    assert!(rx.try_recv().is_ok(), "the run went out");

    // Something is staged mid-run, then the run fails.
    app.sftp.as_mut().unwrap().queue.push(QueuedTransfer {
        direction: Direction::Download,
        src: "/srv/b.bin".into(),
        dst: "/data/b.bin".into(),
        name: "b.bin".into(),
        is_dir: false,
    });
    app.apply_sftp_event(crate::sftp::SftpEvent::Error("permission denied".into()));
    app.apply_sftp_event(crate::sftp::SftpEvent::QueueDone);
    // Re-listing after the failure is expected; a second RunQueue is not.
    let dispatched_again =
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, SftpCommand::RunQueue(_)));
    assert!(
        !dispatched_again,
        "a failed run waits for the user rather than dispatching another pass"
    );
}

/// The dotfile setting survives a restart: it is written to `ui_state` on
/// toggle and read back when the next session builds its browser.
#[test]
fn hidden_setting_round_trips_through_the_store() {
    use crate::sftp::model::SftpState;

    let store = test_store();
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(MockResolver::new(vec![])),
            metadata: Arc::new(MetadataDb::default()),
            store: store.clone(),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    assert!(!app.sftp_show_hidden, "dotfiles start hidden");

    app.sftp = Some(SftpState::new("/srv", "/home/me"));
    app.active_tab = 1;
    app.handle_key(key(KeyCode::Char('.'))).unwrap();
    assert!(app.sftp_show_hidden);
    assert!(app.sftp.as_ref().unwrap().local.show_hidden);
    assert!(app.sftp.as_ref().unwrap().remote.show_hidden);

    // A fresh App over the same store comes up with dotfiles shown, and a
    // browser opened in it inherits that.
    let mut next = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(MockResolver::new(vec![])),
            metadata: Arc::new(MetadataDb::default()),
            store,
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    next.reload_hosts().unwrap();
    assert!(next.sftp_show_hidden, "setting did not survive the restart");
}

/// Regression: `.` is a toggle in the browser but an ordinary character while
/// filtering. The search guard sits at the top of the SFTP dispatch and is easy
/// to lose in a refactor, so pin it.
#[test]
fn dot_types_into_the_filter_instead_of_toggling() {
    use crate::sftp::model::SftpState;

    let mut app = test_app(vec![]);
    let mut state = SftpState::new("/srv", "/home/me");
    state.start_search();
    app.sftp = Some(state);
    app.active_tab = 1;

    app.handle_key(key(KeyCode::Char('.'))).unwrap();
    app.handle_key(key(KeyCode::Char('s'))).unwrap();
    app.handle_key(key(KeyCode::Char('s'))).unwrap();

    let state = app.sftp.as_ref().unwrap();
    assert_eq!(state.remote.filter, ".ss", "the dot went into the filter");
    assert!(!app.sftp_show_hidden, "and did not flip the setting");
}

/// Regression: the SFTP host search typed `S` and `L` into global binds
/// (`Shift+S` snippet manager, `Shift+L` log browser), which ran before the
/// tab's search handler. Every character a dashboard input can take is its own.
#[test]
fn shift_letters_type_into_the_sftp_host_search() {
    let mut app = test_app(vec![("SLOW", host("SLOW"))]);
    app.active_tab = 1;

    app.handle_key(key_char('/')).unwrap();
    for c in "SL".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT))
            .unwrap();
    }

    assert_eq!(app.mode, AppMode::Normal, "no overlay opened");
    assert!(app.sftp_picker_searching);
    assert_eq!(app.search_query, "SL");
}

/// The same guard covers the file filter inside a live SFTP browser.
#[test]
fn shift_letters_type_into_the_sftp_file_filter() {
    use crate::sftp::model::SftpState;

    let mut app = test_app(vec![]);
    let mut state = SftpState::new("/srv", "/home/me");
    state.start_search();
    app.sftp = Some(state);
    app.active_tab = 1;

    for c in "SL".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT))
            .unwrap();
    }

    assert_eq!(app.mode, AppMode::Normal, "no overlay opened");
    assert_eq!(app.sftp.as_ref().unwrap().remote.filter, "SL");
}

/// The SFTP tab reaches its themed renderer through the real frame dispatch.
///
/// The screen's own tests drive `render_browser` directly; this one proves the
/// tab wiring — that `render` selects the SFTP body and hands it the active
/// theme — with a marker no other role carries.
#[test]
fn the_sftp_tab_renders_through_the_active_theme() {
    use crate::sftp::model::{FileEntry, Focus, SftpState};
    use crate::test_support::{fg, marker, role_marker_theme};

    const DIRECTORY: u32 = 0xb1_0001;

    let mut app = test_app(vec![]);
    app.activate_resolved_theme(std::rc::Rc::new(role_marker_theme(
        "sftp-tab",
        &[fg("components.sftp.remote", DIRECTORY)],
    )));
    app.active_tab = 1;

    let mut state = SftpState::new("/srv", "/data");
    state.focus = Focus::Remote;
    state.remote.set_entries(vec![FileEntry {
        name: "uploads".into(),
        is_dir: true,
        size: 0,
        is_symlink: false,
        perm: None,
    }]);
    // Row 0 is the synthetic ".." entry, so `uploads` is not the selected one
    // and reads its directory role rather than the selection bar.
    state.remote.selected = 0;
    app.sftp = Some(state);

    let area = ratatui::layout::Rect::new(0, 0, 100, 30);
    let buf = crate::test_support::frame_at(area, |frame| crate::tui::render(frame, &app));
    assert_eq!(
        crate::test_support::fg_at_text_from(&buf, "uploads", 1),
        marker(DIRECTORY),
        "a remote directory row wears `components.sftp.remote`"
    );
}

fn dir_entry(name: &str) -> crate::sftp::model::FileEntry {
    crate::sftp::model::FileEntry {
        name: name.into(),
        is_dir: true,
        size: 0,
        is_symlink: false,
        perm: None,
    }
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key_char(c)).unwrap();
    }
}

/// A browser whose focused (left) pane is a real temp directory holding
/// `alpha/`, `alps/`, `beta/` and a plain file.
fn local_browser() -> (App, tempfile::TempDir) {
    use crate::sftp::model::{Focus, SftpState};
    let tmp = tempfile::tempdir().unwrap();
    for d in ["alpha", "alps", "beta"] {
        std::fs::create_dir(tmp.path().join(d)).unwrap();
    }
    std::fs::write(tmp.path().join("file.txt"), b"x").unwrap();
    let mut state = SftpState::new("/srv", tmp.path());
    state.focus = Focus::Local;
    state
        .local
        .set_entries(["alpha", "alps", "beta"].map(dir_entry).to_vec());
    let mut app = test_app(vec![]);
    app.active_tab = 1;
    app.sftp = Some(state);
    (app, tmp)
}

#[test]
fn goto_tab_completes_one_match_and_extends_several_locally() {
    let (mut app, tmp) = local_browser();

    // Named in the pane's own listing: one match fills in with a slash.
    app.handle_key(key_char('g')).unwrap();
    type_text(&mut app, "b");
    app.handle_key(key(KeyCode::Tab)).unwrap();
    let p = app.sftp_prompt.as_ref().unwrap();
    assert_eq!(p.value, "beta/");
    assert_eq!(p.hint, None);

    // Several: the shared prefix, and the candidates in the hint.
    app.handle_key(key(KeyCode::Esc)).unwrap();
    app.handle_key(key_char('g')).unwrap();
    type_text(&mut app, "al");
    app.handle_key(key(KeyCode::Tab)).unwrap();
    let p = app.sftp_prompt.as_ref().unwrap();
    assert_eq!(p.value, "alp");
    assert_eq!(p.hint.as_deref(), Some("alpha/  alps/"));

    // Another directory is read off the disk, not out of the pane's listing:
    // from inside `beta`, `../al` names the temp dir's children.
    app.handle_key(key(KeyCode::Esc)).unwrap();
    app.sftp.as_mut().unwrap().local.cwd = tmp.path().join("beta");
    app.sftp.as_mut().unwrap().local.set_entries(Vec::new());
    app.handle_key(key_char('g')).unwrap();
    type_text(&mut app, "../al");
    app.handle_key(key(KeyCode::Tab)).unwrap();
    let p = app.sftp_prompt.as_ref().unwrap();
    assert_eq!(p.value, "../alp");
    assert_eq!(p.hint.as_deref(), Some("alpha/  alps/"));
}

#[test]
fn goto_refuses_a_local_file_and_enters_a_directory() {
    let (mut app, tmp) = local_browser();
    let start = app.sftp.as_ref().unwrap().local.cwd.clone();

    app.handle_key(key_char('g')).unwrap();
    type_text(&mut app, &tmp.path().join("file.txt").display().to_string());
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::SftpPrompt, "the prompt stays open");
    let error = app.sftp_prompt.as_ref().unwrap().error.clone().unwrap();
    assert!(error.starts_with("not a directory: "), "{error}");
    assert_eq!(app.sftp.as_ref().unwrap().local.cwd, start);

    // The same prompt then takes a directory: the pane moves and the prompt closes.
    for _ in 0.."file.txt".len() {
        app.handle_key(key(KeyCode::Backspace)).unwrap();
    }
    type_text(&mut app, "alpha");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.sftp_prompt.is_none());
    assert_eq!(
        app.sftp.as_ref().unwrap().local.cwd,
        tmp.path().join("alpha")
    );
}

#[test]
fn goto_ignores_a_listing_that_is_not_the_one_asked_for() {
    use crate::sftp::model::{SftpState, Side};
    use crate::sftp::{SftpCommand, SftpEvent};
    use std::path::PathBuf;

    let mut app = test_app(vec![]);
    let (tx, rx) = std::sync::mpsc::channel::<SftpCommand>();
    app.sftp_tx = Some(tx);
    app.sftp = Some(SftpState::new("/srv", "/data")); // right pane has focus
    app.active_tab = 1;

    // `home/d` is not the pane's own directory, so the worker is asked.
    app.handle_key(key_char('g')).unwrap();
    type_text(&mut app, "home/d");
    app.handle_key(key(KeyCode::Tab)).unwrap();
    let asked = std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            SftpCommand::ListNames(p) => Some(p),
            _ => None,
        })
        .expect("a ListNames request");
    assert_eq!(asked, PathBuf::from("/srv/home"));
    let before = app.sftp_prompt.clone().unwrap();
    assert_eq!(before.hint.as_deref(), Some("completing..."));

    // An answer about another directory, or from the other pane's worker,
    // changes nothing.
    let names = || vec![dir_entry("data"), dir_entry("docs")];
    app.apply_sftp_event(SftpEvent::Names("/srv/other".into(), names()));
    assert_eq!(app.sftp_prompt.as_ref(), Some(&before));
    app.apply_sftp_event_left(SftpEvent::Names("/srv/home".into(), names()));
    assert_eq!(app.sftp_prompt.as_ref(), Some(&before));

    // The right answer fills the hint in.
    app.apply_sftp_event(SftpEvent::Names("/srv/home".into(), names()));
    let p = app.sftp_prompt.as_ref().unwrap();
    assert_eq!(
        (p.value.as_str(), p.hint.as_deref()),
        ("home/d", Some("data/  docs/"))
    );
    assert_eq!(p.side, Side::Remote);

    // Once the prompt is gone a late answer is dropped without a trace.
    app.handle_key(key(KeyCode::Esc)).unwrap();
    app.apply_sftp_event(SftpEvent::Names("/srv/home".into(), names()));
    assert!(app.sftp_prompt.is_none());
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
fn goto_without_a_worker_says_so_instead_of_completing() {
    use crate::sftp::model::SftpState;
    let mut app = test_app(vec![]);
    app.sftp = Some(SftpState::new("/srv", "/data"));
    app.active_tab = 1;
    app.handle_key(key_char('g')).unwrap();
    type_text(&mut app, "home/d");
    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(
        app.sftp_prompt.as_ref().unwrap().hint.as_deref(),
        Some("not connected")
    );
}

#[test]
fn tab_in_the_mkdir_prompt_leaves_the_name_alone() {
    let (mut app, _tmp) = local_browser();
    app.handle_key(key_char('n')).unwrap();
    type_text(&mut app, "al");
    app.handle_key(key(KeyCode::Tab)).unwrap();
    let p = app.sftp_prompt.as_ref().unwrap();
    assert_eq!(
        p.value, "al",
        "the pane lists alpha/ but this is a new name"
    );
    assert_eq!((p.hint.as_deref(), p.error.as_deref()), (None, None));
}
