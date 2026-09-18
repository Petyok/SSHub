//! Data-layer tests for session command history, capture guards, and
//! insertion byte contracts. The Ctrl+Space picker these once drove is gone
//! (ghost text replaced it); what remains here is everything that still holds
//! against current production: the input tracker, the capture guards, the
//! persisted store, and the exact PTY bytes insertion sends.

use super::*;
use crate::suggestions::SuggestionProvider;
use std::time::{Duration, Instant};

fn fixture(local: bool) -> App {
    let mut app = test_app(vec![]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
    let config = crate::session::SessionConfig {
        argv: vec![
            "sh".into(),
            "-c".into(),
            "stty raw -echo; printf READY; cat".into(),
        ],
        display_name: "fixture".into(),
        meta: Default::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "fixture".into(),
    };
    let mut session = crate::session::Session::spawn(config, 30, 100, None).unwrap();
    session.phase = crate::session::SessionPhase::Running {
        started_at: Instant::now(),
    };
    session.local_shell = local;
    let until = Instant::now() + Duration::from_secs(3);
    while !session.parser.screen().contents().contains("READY") {
        assert!(Instant::now() < until, "fixture did not become ready");
        session.drain();
        std::thread::sleep(Duration::from_millis(5));
    }
    session.parser.process(b"\x1b[2J\x1b[H");
    app.sessions.push(session);
    app.active_session = Some(0);
    app.mode = AppMode::Session;
    app
}

fn fixture_capture(path: &std::path::Path, local: bool) -> App {
    let mut app = test_app(vec![]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
    let config = crate::session::SessionConfig {
        argv: vec![
            "sh".into(),
            "-c".into(),
            "stty raw -echo; printf READY; tee \"$1\"".into(),
            "fixture".into(),
            path.to_str().unwrap().into(),
        ],
        display_name: "fixture".into(),
        meta: Default::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "fixture".into(),
    };
    let mut session = crate::session::Session::spawn(config, 30, 100, None).unwrap();
    session.phase = crate::session::SessionPhase::Running {
        started_at: Instant::now(),
    };
    session.local_shell = local;
    let until = Instant::now() + Duration::from_secs(3);
    while !session.parser.screen().contents().contains("READY") {
        assert!(Instant::now() < until, "fixture did not become ready");
        session.drain();
        std::thread::sleep(Duration::from_millis(5));
    }
    session.parser.process(b"\x1b[2J\x1b[H");
    app.sessions.push(session);
    app.active_session = Some(0);
    app.mode = AppMode::Session;
    app
}

fn type_text(app: &mut App, text: &str) {
    for ch in text.chars() {
        app.handle_key(key_char(ch)).unwrap();
    }
}

fn submit(app: &mut App, command: &str) {
    app.handle_paste(command).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
}

fn received(app: &mut App, path: &std::path::Path, expected: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        app.active_session_mut().unwrap().drain();
        let bytes = std::fs::read(path).unwrap_or_default();
        if bytes.len() >= expected.len() {
            assert_eq!(bytes, expected);
            return;
        }
        assert!(
            Instant::now() < deadline,
            "PTY fixture did not receive expected bytes"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn screen_text(app: &mut App) -> String {
    let until = Instant::now() + Duration::from_millis(200);
    while Instant::now() < until {
        app.active_session_mut().unwrap().drain();
        std::thread::sleep(Duration::from_millis(5));
    }
    app.active_session().unwrap().parser.screen().contents()
}

fn managed_host(app: &mut App, name: &str) -> i64 {
    app.store
        .create_host(&crate::store::NewHost::launcher(name, "10.0.0.7"))
        .unwrap()
        .id
}

fn setting(app: &mut App, item: SettingItem) {
    app.mode = AppMode::Settings;
    app.settings_selected = crate::app::SETTINGS_ITEMS
        .iter()
        .position(|d| d.item == item)
        .expect("setting row exists");
    app.handle_key(key(KeyCode::Enter)).unwrap();
}

const FAILED_SPAWN_CHILD: &str = "SSHUB_TEST_FAILED_LOCAL_SHELL";
const FAILED_SPAWN_SHELL: &str = "/nonexistent-sshub-test-shell";

fn spawn_self_filtered(test: &str) -> std::process::ExitStatus {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("app::tests::suggestions::{test}"),
            "--nocapture",
        ])
        .env(FAILED_SPAWN_CHILD, "1")
        .env("SHELL", FAILED_SPAWN_SHELL)
        .status()
        .unwrap()
}

#[test]
fn failed_local_shell_launch_preserves_prior_session_classification() {
    // Oracle: the real OS process machinery — `$SHELL` names a binary that
    // cannot exist, so `which` (or the spawn itself) must refuse it, and the
    // only honest assertion is on the prior session afterward. `$SHELL` is
    // process-global, so the failure runs in a child process (same binary,
    // filtered to this test) to stay hermetic under parallel parents.
    if std::env::var(FAILED_SPAWN_CHILD).is_err() {
        assert!(spawn_self_filtered(
            "failed_local_shell_launch_preserves_prior_session_classification"
        )
        .success());
        return;
    }
    let mut app = fixture(false);
    // Prior session never authenticated: still connecting, no live shell.
    app.active_session_mut().unwrap().phase = crate::session::SessionPhase::Connecting {
        started_at: Instant::now(),
    };
    assert!(
        !app.active_session().unwrap().is_live_authenticated(),
        "setup: prior session is not live"
    );
    app.open_local_shell().unwrap();
    assert_eq!(app.sessions.len(), 1, "failed spawn pushes no tab");
    assert_eq!(app.active_session, Some(0), "failed spawn moves no focus");
    assert!(
        !app.active_session().unwrap().is_live_authenticated(),
        "failed spawn must not classify the prior session as live"
    );
}

#[test]
fn failed_local_shell_creation_over_remote_leaves_prior_session_untouched() {
    // Oracle: same real-`which`/real-spawn refusal as its sibling, in a child
    // process for `$SHELL` isolation. The audited contract: session count,
    // active index, the remote (non-local) flag, and the full classification
    // triple are byte-identical across the failed spawn.
    if std::env::var(FAILED_SPAWN_CHILD).is_err() {
        assert!(spawn_self_filtered(
            "failed_local_shell_creation_over_remote_leaves_prior_session_untouched"
        )
        .success());
        return;
    }
    let mut app = fixture(false);
    // Remote-style prior session: Running, explicitly not a local shell.
    // (Not connected — no ssh here — so the triple pins to its actual
    // values; the assertion is equality across the call, not liveness.)
    app.active_session_mut().unwrap().local_shell = false;
    let was_live = app.active_session().unwrap().is_live_authenticated();
    let was_connected = app.active_session().unwrap().is_connected();
    let was_name = app.active_session().unwrap().display_name.clone();
    app.open_local_shell().unwrap();
    assert_eq!(app.sessions.len(), 1, "failed spawn pushes no tab");
    assert_eq!(app.active_session, Some(0), "failed spawn moves no focus");
    assert!(
        !app.active_session().unwrap().local_shell,
        "prior remote session stays non-local"
    );
    let after = app.active_session().unwrap();
    assert_eq!(
        after.is_live_authenticated(),
        was_live,
        "live flag untouched"
    );
    assert_eq!(after.is_connected(), was_connected, "connected untouched");
    assert_eq!(after.display_name, was_name, "identity untouched");
}

#[test]
fn suggestion_query_cancel_and_remote_output_stay_local() {
    // Oracle: a real PTY child (`cat` round trip). Typed input must never
    // reach the remote before Enter, and remote output must never leak into
    // the tracker or the history.
    let mut app = fixture(true);
    submit(&mut app, "local-query");
    type_text(&mut app, "loc");
    // Remote speaks while the line is still being typed.
    app.active_session_mut().unwrap().write(b"REMOTE").unwrap();
    assert_eq!(
        app.active_session().unwrap().history.input.typed(),
        "loc",
        "remote output must not corrupt the pending line"
    );
    assert_eq!(
        app.active_session().unwrap().history.entries.len(),
        1,
        "remote output is never indexed"
    );
    assert_eq!(
        screen_text(&mut app).matches("REMOTE").count(),
        1,
        "remote output still reaches the screen verbatim"
    );
}

#[test]
fn indexing_rejects_prompt_context_unauthenticated_and_multiline_input() {
    // Oracle: the vt100 crate for the prompt-line case (the guard reads real
    // grid state); none exists for the tracker rules, which are sshub's own.
    // Unauthenticated: nothing may be captured at all.
    let mut app = fixture(false);
    assert!(
        !app.active_session().unwrap().is_live_authenticated(),
        "setup: timeout reveal is not live"
    );
    assert!(app
        .active_session_mut()
        .unwrap()
        .observe_key(key_char('x'))
        .is_none());
    assert_eq!(
        app.active_session().unwrap().history.input.typed(),
        "",
        "unauthenticated keystrokes leave no trace"
    );
    // Password-prompt context: the cursor line names a secret.
    let mut app = fixture(true);
    app.active_session_mut()
        .unwrap()
        .parser
        .process(b"Password: ");
    assert!(
        app.active_session_mut()
            .unwrap()
            .observe_key(key_char('x'))
            .is_none(),
        "prompt context captures nothing"
    );
    assert_eq!(
        app.active_session().unwrap().history.input.typed(),
        "",
        "prompt context leaves no trace"
    );
    // Multiline input: a later line could answer a prompt not rendered yet.
    let mut app = fixture(true);
    app.active_session_mut().unwrap().observe_paste("one\ntwo");
    assert!(
        app.active_session_mut()
            .unwrap()
            .observe_key(key(KeyCode::Enter))
            .is_none(),
        "multiline input records nothing"
    );
    assert!(app.active_session().unwrap().history.entries.is_empty());
    // Positive control: the same paths capture on a clean live line.
    let mut app = fixture(true);
    app.active_session_mut().unwrap().observe_key(key_char('a'));
    assert_eq!(
        app.active_session().unwrap().history.input.typed(),
        "a",
        "clean live input still captures"
    );
}

#[test]
fn persistence_is_opt_in_managed_only_and_disabling_keeps_rows() {
    // Oracle: the real LauncherStore (in-memory SQLite) — rows must exist
    // there exactly when the opt-in says so, and disabling must keep them.
    let mut app = fixture(true);
    let id = managed_host(&mut app, "db");
    app.active_session_mut().unwrap().meta.host_id = Some(id);
    app.config.command_history.enabled = true;
    submit(&mut app, "pwd");
    assert_eq!(
        app.store.command_history(id, 200).unwrap().len(),
        1,
        "opted-in managed host persists"
    );
    // Disabling stops writes but keeps every row.
    app.config.command_history.enabled = false;
    submit(&mut app, "df");
    assert_eq!(
        app.store.command_history(id, 200).unwrap().len(),
        1,
        "disabling keeps prior rows and writes none"
    );
    assert!(
        app.active_session().unwrap().history.entries.len() == 2,
        "in-memory session history is independent of the opt-in"
    );
    // Unmanaged sessions (no host row) persist nothing even when enabled.
    let mut app = fixture(true);
    app.config.command_history.enabled = true;
    submit(&mut app, "pwd");
    assert!(
        app.active_session().unwrap().history.entries.len() == 1,
        "session history still records"
    );
}

#[test]
fn suggestions_require_authenticated_session_not_timeout_reveal() {
    // Oracle: a real PTY child — the exact-bytes proof that insertion into a
    // non-live session sends nothing and records nothing.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    let mut app = fixture_capture(&path, false);
    assert!(
        !app.active_session().unwrap().is_live_authenticated(),
        "setup: Running without connect evidence is not live"
    );
    app.insert_session_command("ls", true, true);
    assert!(
        app.active_session().unwrap().history.entries.is_empty(),
        "refused insert records nothing"
    );
    app.active_session_mut().unwrap().drain();
    assert_eq!(
        std::fs::read(&path).unwrap_or_default(),
        Vec::<u8>::new(),
        "refused insert writes no PTY bytes"
    );
}

#[test]
fn suggestion_query_and_configured_history_limits_are_bounded() {
    // Oracle: none — the bounds are sshub's own contracts
    // (`MAX_COMMAND_BYTES`, `SESSION_HISTORY_LIMIT`); this pins them.
    use crate::command_safety::MAX_COMMAND_BYTES;
    use crate::session::history::SESSION_HISTORY_LIMIT;
    let mut app = fixture(true);
    // Oversize paste invalidates rather than truncating into a suffix.
    app.active_session_mut()
        .unwrap()
        .observe_paste(&"y".repeat(MAX_COMMAND_BYTES + 1));
    assert_eq!(
        app.active_session().unwrap().history.input.typed(),
        "",
        "oversize input leaves no partial line"
    );
    // Exactly the limit is still accepted (fresh tracker: the invalidation
    // above is sticky by design, so this needs a clean line).
    let mut app = fixture(true);
    app.active_session_mut()
        .unwrap()
        .observe_paste(&"x".repeat(MAX_COMMAND_BYTES));
    assert_eq!(
        app.active_session().unwrap().history.input.typed().len(),
        MAX_COMMAND_BYTES
    );
    // History truncates to the newest 100.
    app.active_session_mut().unwrap().history.clear();
    for i in 0..SESSION_HISTORY_LIMIT + 50 {
        app.active_session_mut()
            .unwrap()
            .history
            .record(&format!("command-{i:03}"), i as i64);
    }
    assert_eq!(
        app.active_session().unwrap().history.entries.len(),
        SESSION_HISTORY_LIMIT,
        "session history is bounded"
    );
}

#[test]
fn legacy_multiline_snippet_keeps_bytes_but_is_not_a_history_suggestion() {
    // Oracle: a real PTY child for the byte proof;
    // `command_safety::classify_command` for the exclusion proof.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    let mut app = fixture_capture(&path, true);
    let multiline = "echo a\necho b";
    app.snippets.push(crate::store::Snippet {
        id: 0,
        name: "legacy".into(),
        command: multiline.into(),
        description: None,
        tags: Vec::new(),
        created_at: 0,
        updated_at: 0,
    });
    let ranked = crate::suggestions::LocalSuggestions.suggestions(
        &crate::suggestions::SuggestionContext {
            session_history: &[],
            host_history: &[],
            snippets: &app.snippets,
        },
        "echo",
    );
    assert!(
        ranked
            .iter()
            .all(|s| s.text == s.text.trim() && !s.text.contains('\n')),
        "provider never suggests multiline text"
    );
    app.record_session_command(Some(multiline.into()));
    assert!(
        app.active_session().unwrap().history.entries.is_empty(),
        "multiline input is never indexed"
    );
    // …but explicit insertion keeps the bytes verbatim (unsafety-checked off).
    app.insert_session_command(multiline, false, false);
    received(&mut app, &path, multiline.as_bytes());
    // …while the safety-checked path still refuses it.
    let len_before = std::fs::read(&path).unwrap().len();
    app.insert_session_command(multiline, false, true);
    app.active_session_mut().unwrap().drain();
    assert_eq!(
        std::fs::read(&path).unwrap().len(),
        len_before,
        "safety-checked insert refuses multiline"
    );
}

#[test]
fn suggestions_and_snippets_send_exact_bracketed_paste_then_enter() {
    // Oracle: a real PTY child with bracketed-paste enabled (as vim does) —
    // the remote must receive one framed paste plus Enter, byte-exact.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    let mut app = fixture_capture(&path, true);
    app.active_session_mut()
        .unwrap()
        .parser
        .process(b"\x1b[?2004h");
    assert!(
        app.active_session()
            .unwrap()
            .parser
            .screen()
            .bracketed_paste(),
        "setup: remote requested bracketed paste"
    );
    app.insert_session_command("docker ps", true, true);
    let mut expected = b"\x1b[200~docker ps\x1b[201~".to_vec();
    expected.push(b'\r');
    received(&mut app, &path, &expected);
    assert_eq!(
        app.active_session().unwrap().history.entries.len(),
        1,
        "inserted command is indexed once executed"
    );
}

#[test]
fn multiline_paste_forwarding_is_unchanged_and_never_indexed() {
    // Oracle: a real PTY child — pastes forward verbatim (remote owns the
    // bytes) while the tracker refuses to model them.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    let mut app = fixture_capture(&path, true);
    app.handle_paste("echo a\necho b").unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    received(&mut app, &path, b"echo a\necho b\r");
    assert!(
        app.active_session().unwrap().history.entries.is_empty(),
        "multiline paste is never indexed"
    );
}

#[test]
fn settings_clear_selected_host_session_and_all_scopes_independently() {
    // Oracle: the real LauncherStore for scope independence; the settings
    // handler for the session scope and the unselected-host notice branch.
    let mut app = fixture(true);
    let id_a = managed_host(&mut app, "a");
    let id_b = managed_host(&mut app, "b");
    app.config.command_history.enabled = true;
    app.active_session_mut().unwrap().meta.host_id = Some(id_a);
    submit(&mut app, "pwd");
    app.store.record_command(Some(id_b), "uptime", 100).unwrap();
    // Session scope: clears the live tab only, persisted rows survive.
    setting(&mut app, SettingItem::ClearSessionHistory);
    assert!(
        app.active_session().unwrap().history.entries.is_empty(),
        "session clear empties the tab"
    );
    assert_eq!(
        app.store.command_history(id_a, 100).unwrap().len(),
        1,
        "session clear keeps persisted rows"
    );
    // Host scope via the store: one host only.
    app.store.clear_command_history(Some(id_a)).unwrap();
    assert!(
        app.store.command_history(id_a, 100).unwrap().is_empty(),
        "host clear deletes that host"
    );
    assert_eq!(
        app.store.command_history(id_b, 100).unwrap().len(),
        1,
        "host clear spares other hosts"
    );
    // All scope: every host.
    app.store.clear_command_history(None).unwrap();
    assert!(
        app.store.command_history(id_b, 100).unwrap().is_empty(),
        "clear-all deletes every host"
    );
    // Handler with no managed selection: notice, and its own store rows
    // survive (seeded in this app's store — each fixture owns a fresh DB).
    let mut app = fixture(true);
    let id_c = managed_host(&mut app, "c");
    app.store.record_command(Some(id_c), "pwd", 100).unwrap();
    setting(&mut app, SettingItem::ClearHostHistory);
    assert_eq!(
        app.host_notice.as_deref(),
        Some("Select a managed host first."),
        "unselected host clear explains itself"
    );
    assert_eq!(
        app.store.command_history(id_c, 100).unwrap().len(),
        1,
        "refused clear writes nothing"
    );
}
#[test]
fn remote_session_latches_connected_and_records_typed_commands() {
    // Oracle: a real PTY child speaking the two utterances the session
    // layer keys off — ssh -v's `Authenticated to …` on stderr (the
    // connected latch) and a shell banner on stdout (the reveal) — with
    // a live `sleep` holding the PTY open while the test types. This is
    // the real-remote path the hand-set fixtures skip: they assign
    // `phase` + `local_shell` instead of earning them through `drain`.
    if std::process::Command::new("sh")
        .arg("-c")
        .arg("exit 0")
        .output()
        .is_err()
    {
        eprintln!("skipping: no sh binary");
        return;
    }
    let mut app = test_app(vec![]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
    let config = crate::session::SessionConfig {
        argv: vec![
            "sh".into(),
            "-c".into(),
            "printf 'debug1: Authenticated to fake (127.0.0.1:22) using publickey\\n' >&2; printf 'Welcome to fakehost\\n'; exec sleep 30".into(),
        ],
        display_name: "fake".into(),
        meta: Default::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "fake".into(),
    };
    let session = crate::session::Session::spawn(config, 30, 100, None).unwrap();
    app.sessions.push(session);
    app.active_session = Some(0);
    app.mode = AppMode::Session;
    // Earn live-authenticated through drain: the -v marker latches
    // `connected`, the banner reveals Running.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.active_session_mut().unwrap().drain();
        if app.active_session().unwrap().is_live_authenticated() {
            break;
        }
        assert!(Instant::now() < deadline, "fake ssh never authenticated");
        std::thread::sleep(Duration::from_millis(10));
    }
    // Type two commands through the real key path (encode → observe →
    // write → record), exactly as a user at a remote prompt does. The
    // first one is the regression: connecting-phase output used to poison
    // the tracker, silently dropping it. Frame drains run between the
    // second command's keystrokes, like the live event loop.
    for ch in "echo hi".chars() {
        app.handle_key(key_char(ch)).unwrap();
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
    for ch in "pwd".chars() {
        app.handle_key(key_char(ch)).unwrap();
        app.active_session_mut().unwrap().drain();
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.active_session_mut().unwrap().drain();
    let recorded: Vec<String> = app
        .active_session()
        .unwrap()
        .history
        .entries
        .iter()
        .map(|e| e.command.clone())
        .collect();
    assert!(
        recorded.contains(&"echo hi".to_string()),
        "first remote command must survive connecting-phase output, got {recorded:?}"
    );
    assert!(
        recorded.contains(&"pwd".to_string()),
        "second remote command must be recorded, got {recorded:?}"
    );
}
