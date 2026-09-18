use super::*;
use crate::suggestions::SuggestionProvider;
use crate::theme::catalog::StyleRole;
use std::time::{Duration, Instant};

fn fixture(capture: Option<&std::path::Path>) -> App {
    let mut app = test_app(vec![]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
    let mut argv = vec!["sh".into(), "-c".into()];
    if let Some(path) = capture {
        argv.extend([
            "stty raw -echo; printf READY; tee \"$1\"".into(),
            "fixture".into(),
            path.to_str().unwrap().into(),
        ]);
    } else {
        argv.push("stty raw -echo; printf READY; cat".into());
    }
    let config = crate::session::SessionConfig {
        argv,
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
    session.local_shell = true;
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

fn rendered_buf(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::tui::render(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn rendered(app: &App, width: u16, height: u16) -> String {
    rendered_buf(app, width, height)
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

/// First cell of `needle`, searched row by row.
fn find_text(buf: &ratatui::buffer::Buffer, needle: &str) -> Option<(u16, u16)> {
    let area = buf.area;
    (area.top()..area.bottom()).find_map(|y| {
        let line: String = (area.left()..area.right())
            .map(|x| buf.cell((x, y)).unwrap().symbol())
            .collect();
        line.find(needle)
            .map(|b| (area.left() + line[..b].chars().count() as u16, y))
    })
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

fn snippet(app: &mut App, name: &str, command: &str) {
    app.snippets.push(crate::store::Snippet {
        id: 0,
        name: name.into(),
        command: command.into(),
        description: None,
        tags: Vec::new(),
        created_at: 0,
        updated_at: 0,
    });
}
#[test]
fn remote_password_session_ghosts_second_prefix_after_first_command() {
    // Oracle: a real PTY child replaying a password-auth remote transcript —
    // ssh -v's `Authenticated to … using "password".` on stderr (the exact
    // line a live host prints), a 60-line MOTD flood with password-policy
    // prose on an earlier line, then a clean `user@monitor::~$ ` prompt with
    // PTY echo left ON like a real interactive shell. Nothing is hand-set:
    // `connected`, `Running`, the recorded entry and the ghost must all be
    // earned through `drain` + the real key path, exactly as at a live host.
    let mut app = test_app(vec![]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
    let child_sh = concat!(
        "printf 'debug1: Authenticating to 10.100.1.200:22 as '\\''user'\\''\\n' >&2; ",
        "printf 'debug1: Next authentication method: password\\n' >&2; ",
        "printf 'debug1: Authentication succeeded (password: auth).\\n' >&2; ",
        "printf 'debug1: Authenticated to 10.100.1.200 ([10.100.1.200]:22) using \"password\".\\n' >&2; ",
        "i=0; while [ $i -lt 60 ]; do echo \"motd banner line $i\"; i=$((i+1)); done; ",
        "echo 'Please change your password regularly'; ",
        "echo 'Last login: Fri Sep 18 12:00:00 2026 from 10.100.1.99'; ",
        "printf 'user@monitor::~$ '; exec sleep 30",
    );
    let config = crate::session::SessionConfig {
        argv: vec!["sh".into(), "-c".into(), child_sh.into()],
        display_name: "kuma".into(),
        meta: Default::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "kuma".into(),
    };
    let session = crate::session::Session::spawn(config, 30, 100, None).unwrap();
    app.sessions.push(session);
    app.active_session = Some(0);
    app.mode = AppMode::Session;
    let state = |app: &App| {
        let s = app.active_session().unwrap();
        format!(
            "live={} connected={} phase={:?} typed={:?} capture={} scrollback={} alt={} entries={:?}",
            s.is_live_authenticated(),
            s.is_connected(),
            s.phase,
            s.history.input.typed(),
            s.history_capture_allowed(),
            s.parser.scrollback(),
            s.parser.screen().alternate_screen(),
            s.history.entries.iter().map(|e| &e.command).collect::<Vec<_>>(),
        )
    };
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        app.active_session_mut().unwrap().drain();
        if app.active_session().unwrap().is_live_authenticated() {
            break;
        }
        assert!(
            Instant::now() < until,
            "remote transcript never authenticated: {}",
            state(&app)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    for ch in "echo lol".chars() {
        app.handle_key(key_char(ch)).unwrap();
        app.active_session_mut().unwrap().drain();
        std::thread::sleep(Duration::from_millis(5));
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.active_session_mut().unwrap().drain();
    assert!(
        app.active_session()
            .unwrap()
            .history
            .entries
            .iter()
            .any(|e| e.command == "echo lol"),
        "first remote command must be recorded: {}",
        state(&app)
    );
    for ch in "ech".chars() {
        app.handle_key(key_char(ch)).unwrap();
        app.active_session_mut().unwrap().drain();
        std::thread::sleep(Duration::from_millis(5));
    }
    let ghost = app.ghost_match();
    assert!(
        ghost.as_ref().is_some_and(|g| g.suffix == "o lol"),
        "second prefix must ghost, got {ghost:?}: {}",
        state(&app)
    );
}
#[test]
fn remote_interactive_bash_ghosts_second_prefix_after_first_command() {
    // Oracle: a REAL interactive `bash -i` on a real PTY — readline redraws
    // the prompt with cursor addressing on every keystroke and echoes input
    // itself, exactly like the shell at the far end of an ssh channel. The
    // `-v` marker is replayed on stderr for the latch; the shell's own
    // stderr (where bash prints PS1) is merged back onto the PTY so the
    // grid sees the prompt exactly as a remote shell's prompt arrives over
    // the ssh channel (the siphon only ever takes the LOCAL ssh's stderr).
    // Hermetic: `--noprofile --norc`, `HOME` pointed at a tempdir, history
    // disabled — the real `~/.bashrc` / `~/.bash_history` are never touched.
    if std::process::Command::new("bash")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping: no bash binary");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(vec![]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
    let child_sh = format!(
        concat!(
            "printf 'debug1: Authenticated to 10.100.1.200 ([10.100.1.200]:22) using \"password\".\\n' >&2; ",
            "exec env HOME={home} HISTFILE={home}/hist PS1='user@monitor:~$ ' ",
            "bash --noprofile --norc -i 2>&1",
        ),
        home = home.path().display(),
    );
    let config = crate::session::SessionConfig {
        argv: vec!["sh".into(), "-c".into(), child_sh],
        display_name: "kuma".into(),
        meta: Default::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "kuma".into(),
    };
    let session = crate::session::Session::spawn(config, 30, 100, None).unwrap();
    app.sessions.push(session);
    app.active_session = Some(0);
    app.mode = AppMode::Session;
    let state = |app: &App| {
        let s = app.active_session().unwrap();
        format!(
            "live={} connected={} phase={:?} typed={:?} capture={} scrollback={} alt={} entries={:?} tail={:?}",
            s.is_live_authenticated(),
            s.is_connected(),
            s.phase,
            s.history.input.typed(),
            s.history_capture_allowed(),
            s.parser.scrollback(),
            s.parser.screen().alternate_screen(),
            s.history.entries.iter().map(|e| &e.command).collect::<Vec<_>>(),
            s.parser.screen().contents().chars().rev().take(120).collect::<String>(),
        )
    };
    // The shell must print its prompt: proves PS1 reached the grid (stderr
    // merge) and earns `Running` + the latch through `drain` alone.
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        app.active_session_mut().unwrap().drain();
        let s = app.active_session().unwrap();
        if s.is_live_authenticated() && s.parser.screen().contents().contains("user@monitor") {
            break;
        }
        assert!(
            Instant::now() < until,
            "bash never prompted: {}",
            state(&app)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    for ch in "echo lol".chars() {
        app.handle_key(key_char(ch)).unwrap();
        app.active_session_mut().unwrap().drain();
        std::thread::sleep(Duration::from_millis(10));
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        app.active_session_mut().unwrap().drain();
        let s = app.active_session().unwrap();
        if s.history.entries.iter().any(|e| e.command == "echo lol")
            && s.parser.screen().contents().contains("lol")
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "bash never ran the first command: {}",
            state(&app)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // Wait for the fresh prompt before typing the prefix: the cursor line
    // must be the new prompt, as on the live host.
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        app.active_session_mut().unwrap().drain();
        let s = app.active_session().unwrap();
        if s.history_capture_allowed() && s.history.input.typed().is_empty() {
            break;
        }
        assert!(
            Instant::now() < until,
            "no clean prompt after first command: {}",
            state(&app)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    for ch in "ech".chars() {
        app.handle_key(key_char(ch)).unwrap();
        app.active_session_mut().unwrap().drain();
        std::thread::sleep(Duration::from_millis(10));
    }
    let ghost = app.ghost_match();
    assert!(
        ghost.as_ref().is_some_and(|g| g.suffix == "o lol"),
        "second prefix must ghost over a real interactive shell, got {ghost:?}: {}",
        state(&app)
    );
}

#[test]
fn ghost_shows_first_line_suffix_in_dim_cells() {
    // Oracle: none exists per docs/oracle-tests.md (app/tui have no external
    // oracle); the TestBackend snapshot is the oracle, plus a differential
    // check against a direct provider call below.
    let mut app = fixture(None);
    submit(&mut app, "docker ps");
    type_text(&mut app, "do");
    let ghost = app.ghost_match().expect("typed prefix must ghost");
    assert_eq!(ghost.text, "docker ps");
    assert_eq!(ghost.suffix, "cker ps");
    assert_eq!(ghost.shown, "cker ps");
    // Differential: the ghost must agree with the provider's own top-1.
    let session = app.active_session().unwrap();
    let top = crate::suggestions::LocalSuggestions
        .suggestions(
            &crate::suggestions::SuggestionContext {
                session_history: &session.history.entries,
                host_history: &[],
                snippets: &app.snippets,
            },
            "do",
        )
        .into_iter()
        .next()
        .expect("provider must rank the typed command");
    assert_eq!(ghost.text, top.text, "ghost must be the provider top-1");
    // Painted inline in dim cells (echo is off in the fixture, so the grid is
    // blank and the ghost is the only "cker ps" on screen).
    let buf = rendered_buf(&app, 100, 30);
    let (x, y) = find_text(&buf, "cker ps").expect("ghost suffix painted");
    let dim_fg = app
        .theme()
        .style(StyleRole::TextDim)
        .fg
        .expect("text.dim carries a foreground");
    for (i, _) in "cker ps".chars().enumerate() {
        let cell = buf.cell((x + i as u16, y)).unwrap();
        assert_eq!(
            cell.fg, dim_fg,
            "ghost cell {i} takes the dim foreground, not the PTY's"
        );
    }
    // Footer names the ghost keys only while visible.
    assert!(rendered(&app, 100, 30).contains("Tab accept"));
}

#[test]
fn ghost_accept_writes_suffix_only() {
    // Oracle: a real PTY child (`tee` capture file) — the exact bytes the
    // remote would receive. Accept must send only the untyped remainder.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    let mut app = fixture(Some(&path));
    submit(&mut app, "docker ps");
    type_text(&mut app, "do");
    app.handle_key(key(KeyCode::Tab)).unwrap();
    // Typed "do" + accepted "cker ps": the remote sees the full command once.
    received(&mut app, &path, b"docker ps\rdocker ps");
}

#[test]
fn tab_without_ghost_forwards_to_shell() {
    // Oracle: a real PTY child — with no match, Tab must reach the shell as
    // `\t` (shell completion stays reachable whenever no ghost shows).
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    let mut app = fixture(Some(&path));
    type_text(&mut app, "zz");
    assert!(app.ghost_match().is_none(), "no candidate, no ghost");
    app.handle_key(key(KeyCode::Tab)).unwrap();
    received(&mut app, &path, b"zz\t");
}

#[test]
fn esc_dismisses_without_forwarding_and_stays_hidden_until_input_changes() {
    // Oracle: a real PTY child for the no-forward proof; the TestBackend
    // snapshot for the sticky-hidden proof (no oracle exists for app state).
    // The swallow proof needs the `tee` capture child, but `tee` echoes
    // typed input back onto the grid — so the paint-absence proof runs on
    // the echo-free `cat` child, where the ghost is the only suffix source.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    let mut app = fixture(Some(&path));
    submit(&mut app, "docker ps");
    type_text(&mut app, "do");
    assert!(app.ghost_match().is_some());
    app.handle_key(key(KeyCode::Esc)).unwrap();
    // Swallowed: the child saw the typed line but no Escape byte.
    received(&mut app, &path, b"docker ps\rdo");
    assert!(app.ghost_match().is_none(), "Esc dismisses the ghost");
    let mut quiet = fixture(None);
    submit(&mut quiet, "docker ps");
    type_text(&mut quiet, "do");
    assert!(
        find_text(&rendered_buf(&quiet, 100, 30), "cker ps").is_some(),
        "ghost paints before dismiss"
    );
    quiet.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(
        find_text(&rendered_buf(&quiet, 100, 30), "cker ps").is_none(),
        "dismissed ghost paints nothing"
    );
    assert!(
        !rendered(&quiet, 100, 30).contains("Tab accept"),
        "footer stops naming ghost keys once hidden"
    );
    // Any further input re-arms it.
    type_text(&mut quiet, "c");
    let ghost = quiet.ghost_match().expect("new input re-arms the ghost");
    assert_eq!(ghost.text, "docker ps");
}

#[test]
fn render_performs_zero_pty_writes() {
    // Oracle: a real PTY child — the no-PTY-write proof. The parser grid and
    // the child's capture file must be byte-identical across a full render
    // with a visible ghost.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    let mut app = fixture(Some(&path));
    submit(&mut app, "docker ps");
    type_text(&mut app, "do");
    assert!(app.ghost_match().is_some(), "ghost visible for this test");
    // Quiesce: block until the child has consumed every byte sent so far,
    // or late-arriving input (not the render) would fail the comparison.
    received(&mut app, &path, b"docker ps\rdo");
    // Settle the echo too: `tee` mirrors input back onto the grid, so wait
    // until two consecutive drains agree before snapshotting it.
    let deadline = Instant::now() + Duration::from_secs(3);
    let before = loop {
        app.active_session_mut().unwrap().drain();
        let a = app.active_session().unwrap().parser.screen().contents();
        std::thread::sleep(Duration::from_millis(20));
        app.active_session_mut().unwrap().drain();
        let b = app.active_session().unwrap().parser.screen().contents();
        if a == b {
            break b;
        }
        assert!(Instant::now() < deadline, "grid never settled");
    };
    let file_before = std::fs::read(&path).unwrap_or_default();
    rendered(&app, 100, 30);
    app.active_session_mut().unwrap().drain();
    assert_eq!(
        app.active_session().unwrap().parser.screen().contents(),
        before,
        "render must not mutate the PTY grid"
    );
    assert_eq!(
        std::fs::read(&path).unwrap_or_default(),
        file_before,
        "render must not write to the PTY"
    );
}

#[test]
fn ghost_hides_on_alternate_screen_short_prefix_and_cursor_move() {
    // Oracle: the vt100 crate itself for alternate-screen state (an external
    // implementation of the grid sshub mirrors); none exists for the rest.
    let mut app = fixture(None);
    submit(&mut app, "docker ps");
    // Too short: a single char matches nearly everything.
    type_text(&mut app, "d");
    assert!(app.ghost_match().is_none(), "1-char prefix never ghosts");
    type_text(&mut app, "o");
    assert!(app.ghost_match().is_some());
    // A remote full-screen app owns its grid.
    app.active_session_mut()
        .unwrap()
        .parser
        .process(b"\x1b[?1049h");
    assert!(
        app.active_session()
            .unwrap()
            .parser
            .screen()
            .alternate_screen(),
        "fixture must really be on the alternate screen"
    );
    assert!(app.ghost_match().is_none(), "no ghost over vim/tmux");
    app.active_session_mut()
        .unwrap()
        .parser
        .process(b"\x1b[?1049l");
    assert!(app.ghost_match().is_some(), "ghost returns with the shell");
    // Cursor movement invalidates the tracker: the cursor may have moved.
    app.handle_key(key(KeyCode::Left)).unwrap();
    assert!(app.ghost_match().is_none(), "cursor move hides the ghost");
}

#[test]
fn fuzzy_tier_matches_are_suppressed_but_prefix_match_shows() {
    // Oracle: LocalSuggestions as a differential — the provider DOES rank the
    // word-tier hit, so hiding it is provably a ghost-level decision.
    let mut app = fixture(None);
    submit(&mut app, "docker ps");
    type_text(&mut app, "ps");
    let ranked = crate::suggestions::LocalSuggestions.suggestions(
        &crate::suggestions::SuggestionContext {
            session_history: &app.active_session().unwrap().history.entries,
            host_history: &[],
            snippets: &app.snippets,
        },
        "ps",
    );
    assert!(
        ranked.iter().any(|s| s.text == "docker ps"),
        "provider ranks the word-tier hit"
    );
    assert!(
        app.ghost_match().is_none(),
        "ghost suppresses non-prefix (fuzzy tier) matches"
    );
    app.handle_key(key(KeyCode::Backspace)).unwrap();
    app.handle_key(key(KeyCode::Backspace)).unwrap();
    type_text(&mut app, "do");
    assert_eq!(app.ghost_match().unwrap().text, "docker ps");
}

#[test]
fn ghost_ranks_snippets_with_history_as_one_list() {
    // Oracle: LocalSuggestions differential — one merged list, ghost takes
    // exactly its head.
    let mut app = fixture(None);
    submit(&mut app, "docker ps");
    snippet(&mut app, "kubetail", "kubectl get pods");
    type_text(&mut app, "kub");
    let ghost = app.ghost_match().expect("snippet prefix must ghost");
    assert_eq!(ghost.text, "kubectl get pods");
    assert_eq!(ghost.shown, "ectl get pods");
    let session = app.active_session().unwrap();
    let top = crate::suggestions::LocalSuggestions
        .suggestions(
            &crate::suggestions::SuggestionContext {
                session_history: &session.history.entries,
                host_history: &[],
                snippets: &app.snippets,
            },
            "kub",
        )
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(ghost.text, top.text);
}

#[test]
fn unsafe_commands_never_ghost() {
    // Oracle: command_safety::classify_command — the ghost must never offer
    // what the classifier rejects, even when it prefix-matches.
    assert!(
        matches!(
            crate::command_safety::classify_command("mysql -pSECRET"),
            crate::command_safety::CommandSafety::Sensitive { .. }
        ),
        "oracle rejects the credential-like command"
    );
    let mut app = fixture(None);
    submit(&mut app, "mysql --help");
    snippet(&mut app, "cred", "mysql -pSECRET");
    type_text(&mut app, "mysql -p");
    assert!(
        app.ghost_match().is_none(),
        "credential-like commands never ghost"
    );
    let mut app = fixture(None);
    submit(&mut app, "mysql --help");
    type_text(&mut app, "mysq");
    assert_eq!(app.ghost_match().unwrap().text, "mysql --help");
}

#[test]
fn ctrl_f_accepts_like_tab() {
    // Oracle: a real PTY child — the remappable fallback writes the same
    // suffix bytes as Tab.
    use crossterm::event::KeyModifiers;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    let mut app = fixture(Some(&path));
    assert_eq!(
        app.config
            .keybinds
            .primary(crate::config::KeyAction::GhostAccept),
        "Ctrl+F",
        "fallback bind the test presses"
    );
    submit(&mut app, "docker ps");
    type_text(&mut app, "do");
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL))
        .unwrap();
    received(&mut app, &path, b"docker ps\rdocker ps");
}

#[test]
fn ghost_hides_case_only_prefix_match() {
    // Oracle: none exists per docs/oracle-tests.md (app/tui have no external
    // oracle); the typed-line/ghost assertions below are the contract.
    // Ranking folds case, but the suffix boundary must be exact-case: typed
    // `DO` off `docker ps` would otherwise accept as `DOcker ps`.
    let mut app = fixture(None);
    submit(&mut app, "docker ps");
    type_text(&mut app, "DO");
    assert!(
        app.ghost_match().is_none(),
        "case-only prefix must not ghost, got {:?}",
        app.ghost_match()
    );
}

#[test]
fn ghost_accepts_mixed_case_exact_prefix() {
    // Oracle: same as its sibling — the contract is the suffix boundary on
    // exact bytes: typed `DO` off recorded `DOcker ps` completes `cker ps`
    // (and accepting it writes exactly those bytes, not a folded rewrite).
    let mut app = fixture(None);
    submit(&mut app, "DOcker ps");
    type_text(&mut app, "DO");
    let ghost = app.ghost_match().expect("exact-case prefix must ghost");
    assert_eq!(ghost.text, "DOcker ps");
    assert_eq!(ghost.suffix, "cker ps");
    assert_eq!(ghost.shown, "cker ps");
}

#[test]
fn fully_typed_line_has_no_ghost_and_tab_forwards() {
    // Oracle: a real PTY child — nothing left to complete, so Tab must reach
    // the shell instead of being swallowed by a zero-width ghost.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    let mut app = fixture(Some(&path));
    submit(&mut app, "docker ps");
    type_text(&mut app, "docker ps");
    assert!(app.ghost_match().is_none(), "fully typed line has no ghost");
    app.handle_key(key(KeyCode::Tab)).unwrap();
    received(&mut app, &path, b"docker ps\rdocker ps\t");
}

#[test]
fn ghost_completes_from_opt_in_host_history() {
    // Oracle: the real LauncherStore (in-memory SQLite) — the row must exist
    // there AND surface as a ghost, proving the cache path end to end.
    let mut app = fixture(None);
    app.config.command_history.enabled = true;
    // Persisted history is keyed to a real managed host row (the store
    // refuses orphan ids), so create one and link the session to it.
    let created = app
        .store
        .create_host(&crate::store::NewHost::launcher("db", "10.0.0.7"))
        .unwrap();
    app.active_session_mut().unwrap().meta.host_id = Some(created.id);
    app.store
        .record_command(Some(created.id), "docker ps", 100)
        .unwrap();
    assert_eq!(
        app.store.command_history(created.id, 100).unwrap().len(),
        1,
        "oracle holds the row"
    );
    type_text(&mut app, "do");
    let ghost = app.ghost_match().expect("host history must ghost");
    assert_eq!(ghost.text, "docker ps");
    // …but not when the user never opted in.
    let mut app = fixture(None);
    app.config.command_history.enabled = false;
    app.active_session_mut().unwrap().meta.host_id = Some(7);
    type_text(&mut app, "do");
    assert!(
        app.ghost_match().is_none(),
        "persisted history is opt-in only"
    );
}

#[test]
fn ghost_host_cache_reloads_on_limit_change() {
    // Oracle: the real LauncherStore (in-memory SQLite) — the rows must exist
    // there AND the ghost must follow a limit edit with no session switch,
    // toggle, or new command, proving the limit is part of the cache key.
    let mut app = fixture(None);
    app.config.command_history.enabled = true;
    let created = app
        .store
        .create_host(&crate::store::NewHost::launcher("db", "10.0.0.7"))
        .unwrap();
    app.active_session_mut().unwrap().meta.host_id = Some(created.id);
    app.store
        .record_command(Some(created.id), "qq old", 100)
        .unwrap();
    app.store
        .record_command(Some(created.id), "qq new", 100)
        .unwrap();
    // Narrow window: only the newest row is visible, and it does not extend
    // the typed line, so no ghost — this call warms the cache at limit 1.
    app.config.command_history.max_entries_per_host = 1;
    type_text(&mut app, "qq o");
    assert!(
        app.ghost_match().is_none(),
        "limit 1 hides the older row, got {:?}",
        app.ghost_match()
    );
    // Widening the limit must reload the cache on the next frame: the older
    // row extends the typed line exactly.
    app.config.command_history.max_entries_per_host = 100;
    let ghost = app
        .ghost_match()
        .expect("widened limit must surface the older row");
    assert_eq!(ghost.text, "qq old");
    assert_eq!(ghost.suffix, "ld");
}

#[test]
fn manual_password_before_latch_still_records_first_post_auth_command() {
    // Oracle: a real PTY child shaped like a password-auth login as seen
    // through ssh — a `user@kuma's password:` prompt answered by TYPING
    // (no stored secret, no askpass), and only afterwards the `-v`
    // `Authenticated to … using "password".` marker on stderr, a banner,
    // and a clean `user@monitor:~$ ` prompt. Pre-auth keystrokes travel the
    // real app key path while capture is disallowed, so the tracker must
    // come out of authentication able to record the very first post-auth
    // command: on a host just logged into by hand, that command is the seed
    // the ghost completes from. The typed password itself must never index.
    let mut app = test_app(vec![]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
    let child_sh = concat!(
        "printf \"user@kuma's password:\"; read -r pw; ",
        "printf 'debug1: Authenticated to 10.100.1.200 ([10.100.1.200]:22) using \"password\".\\n' >&2; ",
        "echo 'Last login: Fri Sep 18 12:00:00 2026 from 10.100.1.99'; ",
        "printf 'user@monitor:~$ '; exec sleep 30",
    );
    let config = crate::session::SessionConfig {
        argv: vec!["sh".into(), "-c".into(), child_sh.into()],
        display_name: "kuma".into(),
        meta: Default::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "kuma".into(),
    };
    let session = crate::session::Session::spawn(config, 30, 100, None).unwrap();
    app.sessions.push(session);
    app.active_session = Some(0);
    app.mode = AppMode::Session;
    let state = |app: &App| {
        let s = app.active_session().unwrap();
        format!(
            "live={} connected={} phase={:?} typed={:?} capture={} entries={:?}",
            s.is_live_authenticated(),
            s.is_connected(),
            s.phase,
            s.history.input.typed(),
            s.history_capture_allowed(),
            s.history
                .entries
                .iter()
                .map(|e| &e.command)
                .collect::<Vec<_>>(),
        )
    };
    // Wait for the password prompt (still Connecting: nothing authenticated).
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        app.active_session_mut().unwrap().drain();
        let screen = app.active_session().unwrap().parser.screen().contents();
        if screen.contains("password:") {
            break;
        }
        assert!(
            Instant::now() < until,
            "password prompt never appeared: {}",
            state(&app)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !app.active_session().unwrap().is_live_authenticated(),
        "must not be authenticated at the password prompt: {}",
        state(&app)
    );
    // Answer by hand through the real key path, exactly as at a live host.
    for ch in "s3cret".chars() {
        app.handle_key(key_char(ch)).unwrap();
        app.active_session_mut().unwrap().drain();
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
    // The marker latches `connected`, the banner reveals Running.
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        app.active_session_mut().unwrap().drain();
        if app.active_session().unwrap().is_live_authenticated() {
            break;
        }
        assert!(
            Instant::now() < until,
            "never authenticated after the password: {}",
            state(&app)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // First post-auth command: every keystroke is capture-allowed, so the
    // pre-auth uncertainty must not eat it.
    for ch in "echo lol".chars() {
        app.handle_key(key_char(ch)).unwrap();
        app.active_session_mut().unwrap().drain();
        std::thread::sleep(Duration::from_millis(5));
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.active_session_mut().unwrap().drain();
    let entries: Vec<String> = app
        .active_session()
        .unwrap()
        .history
        .entries
        .iter()
        .map(|e| e.command.clone())
        .collect();
    assert!(
        !entries.iter().any(|e| e.contains("s3cret")),
        "typed password must never index: {}",
        state(&app)
    );
    assert!(
        entries.iter().any(|e| e == "echo lol"),
        "first post-auth command must be recorded: {}",
        state(&app)
    );
    for ch in "ech".chars() {
        app.handle_key(key_char(ch)).unwrap();
        app.active_session_mut().unwrap().drain();
        std::thread::sleep(Duration::from_millis(5));
    }
    let ghost = app.ghost_match();
    assert!(
        ghost.as_ref().is_some_and(|g| g.suffix == "o lol"),
        "prefix of the first command must ghost, got {ghost:?}: {}",
        state(&app)
    );
}
