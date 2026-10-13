//! Split view (#73): the layout bound to live sessions, the resize contract
//! measured on real PTYs, rendering and input routing.

use super::*;
use crate::app::panes::{PaneTree, Panes, SplitDir};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::time::{Duration, Instant};

const W: u16 = 120;
const H: u16 = 40;

fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
}

/// A child that counts the SIGWINCHes it receives and prints the count and
/// its terminal size on its first row: `W<count> <rows> <cols>`. A plain
/// `trap`, so nothing beyond `/bin/sh` and `stty` is needed.
const WINCH_COUNTER: &str = r#"n=0
trap 'n=$((n+1)); printf "\033[H\033[2KW%d %s" "$n" "$(stty size)"' WINCH
printf '\033[2J\033[HW0 %s' "$(stty size)"
while :; do sleep 0.02; done"#;

fn spawn(app: &mut App, name: &str, script: &str) {
    app.spawn_embedded_session(
        vec!["/bin/sh".into(), "-c".into(), script.into()],
        name.into(),
        crate::session::SessionMeta::default(),
        None,
        name,
        true,
    )
    .unwrap();
}

fn app_in_session() -> App {
    let mut app = test_app(vec![]);
    app.terminal_area = Rect::new(0, 0, W, H);
    app
}

/// One frame of the event loop as `run_terminal_loop` runs it: reconcile the
/// split, then drain and resize every session.
fn frame(app: &mut App) {
    app.sync_panes();
    let grids = app.session_grids();
    for (s, grid) in app.sessions.iter_mut().zip(grids) {
        s.drain();
        if let Some(grid) = grid {
            s.resize(grid);
        }
    }
}

fn first_row(app: &App, i: usize) -> String {
    let screen = app.sessions[i].parser.screen();
    screen.rows(0, screen.size().1).next().unwrap_or_default()
}

/// Run frames until session `i`'s first row satisfies `done`, then a little
/// longer so a late signal would show up. Returns the row.
fn settle(app: &mut App, i: usize, done: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(&first_row(app, i)) && Instant::now() < deadline {
        frame(app);
        std::thread::sleep(Duration::from_millis(5));
    }
    let quiet = Instant::now() + Duration::from_millis(150);
    while Instant::now() < quiet {
        frame(app);
        std::thread::sleep(Duration::from_millis(5));
    }
    first_row(app, i).trim_end().to_string()
}

fn index_of(app: &App, name: &str) -> usize {
    app.sessions
        .iter()
        .position(|s| s.display_name == name)
        .unwrap()
}

#[test]
fn a_session_spawned_into_a_pane_starts_at_the_pane_size_and_the_other_pane_is_untouched() {
    let mut app = app_in_session();
    spawn(&mut app, "a", WINCH_COUNTER);
    spawn(&mut app, "b", WINCH_COUNTER);
    // Classic view of `b` (body 38x120): `a` was spawned at that size too.
    assert_eq!(settle(&mut app, 1, |r| r.starts_with('W')), "W0 38 120");
    assert_eq!(settle(&mut app, 0, |r| r.starts_with('W')), "W0 38 120");

    // Split side by side: `b` keeps the first pane (59 wide), `a` joins in the
    // second (60 wide). One resize each, once.
    app.handle_key(alt('v')).unwrap();
    let (a, b) = (index_of(&app, "a"), index_of(&app, "b"));
    assert_eq!(settle(&mut app, b, |r| r.starts_with("W1")), "W1 38 59");
    assert_eq!(settle(&mut app, a, |r| r.starts_with("W1")), "W1 38 60");

    // A new session opened while split lands in the focused pane (the second)
    // and is born at its size: no SIGWINCH at all.
    spawn(&mut app, "c", WINCH_COUNTER);
    let c = index_of(&app, "c");
    assert_eq!(settle(&mut app, c, |r| r.starts_with('W')), "W0 38 60");
    // `a` left the view and keeps its size; `b` was never touched.
    assert_eq!(settle(&mut app, a, |_| false), "W1 38 60");
    assert_eq!(settle(&mut app, b, |_| false), "W1 38 59");

    // Zoom is one real resize of the focused session, each way; the other
    // pane keeps its size while hidden and gets nothing.
    app.handle_key(alt('m')).unwrap();
    assert_eq!(settle(&mut app, c, |r| r.starts_with("W1")), "W1 38 120");
    app.handle_key(alt('m')).unwrap();
    assert_eq!(settle(&mut app, c, |r| r.starts_with("W2")), "W2 38 60");
    assert_eq!(settle(&mut app, b, |_| false), "W1 38 59");

    // Frames with nothing changed send nothing.
    for _ in 0..20 {
        frame(&mut app);
    }
    assert_eq!(settle(&mut app, c, |_| false), "W2 38 60");
}

#[test]
fn a_session_out_of_view_keeps_its_size_until_it_is_shown() {
    let mut app = app_in_session();
    spawn(&mut app, "a", WINCH_COUNTER);
    spawn(&mut app, "b", WINCH_COUNTER);
    assert_eq!(settle(&mut app, 0, |r| r.starts_with('W')), "W0 38 120");
    // The terminal shrinks twice while `a` sits in the strip: only `b`, on
    // screen, follows.
    for height in [30, 20] {
        app.terminal_area = Rect::new(0, 0, W, height);
        settle(&mut app, 1, |_| false);
    }
    assert_eq!(settle(&mut app, 1, |_| false), "W2 18 120");
    assert_eq!(settle(&mut app, 0, |_| false), "W0 38 120");
    // Shown again: one resize, straight to the current size.
    app.switch_session(-1);
    assert_eq!(settle(&mut app, 0, |r| r.starts_with("W1")), "W1 18 120");
}

#[test]
fn an_exited_session_is_never_resized() {
    let mut app = app_in_session();
    spawn(&mut app, "gone", "printf 'last words'");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.sessions[0].phase.is_terminal() && Instant::now() < deadline {
        frame(&mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.sessions[0].phase.is_terminal());
    app.terminal_area = Rect::new(0, 0, 60, 12);
    frame(&mut app);
    assert_eq!(app.sessions[0].parser.screen().size(), (38, 120));
    assert!(first_row(&app, 0).starts_with("last words"));
}

/// Three idle sessions in session mode, `[a, b, c]`, with `c` and `a` side
/// by side and `a` focused.
fn three_sessions_split() -> App {
    let mut app = app_in_session();
    for name in ["a", "b", "c"] {
        spawn(&mut app, name, "sleep 30");
    }
    app.mode = AppMode::Session;
    let id = |i: usize| Some(app.sessions[i].id);
    app.panes = Some(Panes {
        tree: PaneTree::split_of(SplitDir::Vertical, id(2), id(0)),
        focused: 1,
        zoomed: false,
    });
    app.active_session = Some(0);
    app
}

fn shown(app: &App) -> Vec<Option<String>> {
    app.pane_views(app.terminal_area)
        .iter()
        .map(|v| v.session.map(|i| app.sessions[i].display_name.clone()))
        .collect()
}

#[test]
fn closing_a_session_empties_its_pane_and_keeps_the_others_bound() {
    let mut app = three_sessions_split();
    // Closing `a` (index 0) shifts every index; the first pane must still
    // show `c`, now at index 1, and the focused pane is left empty.
    app.close_active_session();
    assert_eq!(app.sessions.len(), 2);
    assert_eq!(shown(&app), vec![Some("c".into()), None]);
    assert_eq!(app.active_session, None);
    assert_eq!(app.mode, AppMode::Session);
    // Keys in an empty pane go nowhere and do not leave the view.
    app.handle_key(key_char('x')).unwrap();
    assert_eq!(app.mode, AppMode::Session);
    // Closing again closes the empty pane: back to the classic view of `c`.
    app.close_active_session();
    assert!(app.panes.is_none());
    assert_eq!(app.active_session().unwrap().display_name, "c");
}

fn focus(app: &App) -> usize {
    app.panes.as_ref().unwrap().focused
}
#[test]
fn the_switcher_fills_the_focused_pane_and_swaps_a_session_shown_elsewhere() {
    let mut app = three_sessions_split();
    // `b` is in no pane: it replaces `a` in the focused pane.
    app.show_in_focused_pane(1);
    app.sync_panes();
    assert_eq!(shown(&app), vec![Some("c".into()), Some("b".into())]);
    // `c` is already in the first pane: the panes swap, nothing is doubled.
    app.show_in_focused_pane(2);
    app.sync_panes();
    assert_eq!(shown(&app), vec![Some("b".into()), Some("c".into())]);
    assert_eq!(focus(&app), 1);
    // Cycling onto a session another pane shows moves the focus there.
    app.active_session = Some(1);
    app.sync_panes();
    assert_eq!(focus(&app), 0);
    assert_eq!(shown(&app), vec![Some("b".into()), Some("c".into())]);
}

#[test]
fn splits_stop_at_four_panes_and_unsplit_keeps_the_focused_session() {
    let mut app = app_in_session();
    for name in ["a", "b", "c", "d", "e"] {
        spawn(&mut app, name, "sleep 30");
    }
    app.mode = AppMode::Session;
    app.active_session = Some(0);
    app.handle_key(alt('v')).unwrap(); // a | b, focus b
    app.handle_key(alt('j')).unwrap(); // b over c, focus c
    app.handle_key(alt('o')).unwrap(); // focus wraps to a
    assert_eq!(focus(&app), 0);
    app.handle_key(alt('j')).unwrap(); // a over d, focus d
    assert_eq!(
        shown(&app),
        ["a", "d", "b", "c"].map(|n| Some(n.to_string())).to_vec()
    );
    // Every pane is at depth two now: a fifth is refused.
    app.handle_key(alt('v')).unwrap();
    assert_eq!(app.pane_views(app.terminal_area).len(), 4);
    // Zoomed, focus does not move (as on the dashboard, #18).
    app.handle_key(alt('m')).unwrap();
    app.handle_key(alt('o')).unwrap();
    assert_eq!(shown(&app), vec![Some("d".into())]);
    app.handle_key(alt('m')).unwrap();
    // Closing a pane keeps its session open, just out of view.
    app.handle_key(alt('k')).unwrap();
    assert_eq!(app.sessions.len(), 5);
    assert_eq!(
        shown(&app),
        ["a", "b", "c"].map(|n| Some(n.to_string())).to_vec()
    );
    // Unsplit: the classic view of the focused session.
    let focused = app.active_session().unwrap().display_name.clone();
    app.handle_key(alt('i')).unwrap();
    assert!(app.panes.is_none());
    assert_eq!(app.active_session().unwrap().display_name, focused);
}

// ── Rendering and input ─────────────────────────────────────────────

/// A running `cat`: whatever reaches its PTY is echoed on its grid, so the
/// grid shows which pane got the input.
fn two_cats() -> App {
    let mut app = app_in_session();
    spawn(&mut app, "left", "printf READY; exec cat");
    spawn(&mut app, "right", "printf READY; exec cat");
    for s in &mut app.sessions {
        s.phase = crate::session::SessionPhase::Running {
            started_at: Instant::now(),
        };
    }
    app.mode = AppMode::Session;
    app.active_session = Some(0);
    app.handle_key(alt('v')).unwrap();
    frame(&mut app);
    for i in 0..2 {
        settle(&mut app, i, |r| r.contains("READY"));
    }
    app
}

/// Run frames until session `i`'s grid shows `needle`, then a little longer
/// so input that went to the wrong pane would show up too.
fn wait_text(app: &mut App, i: usize, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !grid_text(app, i).contains(needle) && Instant::now() < deadline {
        frame(app);
        std::thread::sleep(Duration::from_millis(5));
    }
    let quiet = Instant::now() + Duration::from_millis(150);
    while Instant::now() < quiet {
        frame(app);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn grid_text(app: &App, i: usize) -> String {
    app.sessions[i].parser.screen().contents()
}

fn render(app: &App) -> ratatui::buffer::Buffer {
    let backend = ratatui::backend::TestBackend::new(W, H);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| crate::tui::render(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn both_panes_keep_their_ground_and_only_the_focused_one_shows_a_cursor() {
    let mut app = two_cats();
    // `aqua` sweeps a gradient over the app background: a theme pass that
    // reached a remote cell would leave its row uneven.
    assert!(app.activate_theme("aqua"));
    app.config.appearance.transparent_sshub_background = false;
    let views = app.pane_views(app.terminal_area);
    let buf = render(&app);
    let ground = app.theme().semantic().pty_background;
    for view in &views {
        let r = view.rect;
        // Row 3 of each pane: below READY, never written by `cat`.
        let row: Vec<_> = (r.x..r.right()).map(|x| buf[(x, r.y + 3)].bg).collect();
        assert!(row.iter().all(|&bg| bg == ground), "{r:?}: {row:?}");
        assert_eq!(buf[(r.x, r.y)].symbol(), "R", "each pane at its own origin");
    }
    // The rule between them is SSHub's, and the focused pane's side of it
    // takes the focus role.
    let rule_x = views[0].rect.right();
    assert_eq!(buf[(rule_x, views[0].rect.y + 3)].symbol(), "\u{2502}");
    assert_eq!(
        buf[(rule_x, views[0].rect.y + 3)].fg,
        app.theme()
            .style(crate::theme::catalog::StyleRole::FocusIndicator)
            .fg
            .unwrap()
    );
    // tui-term paints the cursor as a block; only the focused pane has one.
    let cursor = |i: usize| {
        let (r, c) = app.sessions[i].parser.screen().cursor_position();
        let rect = views[i].rect;
        buf[(rect.x + c, rect.y + r)].symbol().to_string()
    };
    assert_eq!(cursor(0), " ");
    assert_eq!(cursor(1), "\u{2588}");
    // The strip marks both sessions as shown.
    let header: String = (0..W).map(|x| buf[(x, 0)].symbol().to_string()).collect();
    assert!(header.contains("\u{2022}1 left") && header.contains("\u{2022}2 right"));
}

#[test]
fn an_empty_pane_says_how_to_fill_it() {
    let mut app = two_cats();
    app.close_active_session();
    let buf = render(&app);
    let text: String = (0..H)
        .map(|y| {
            (0..W)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("empty pane"));
    assert!(text.contains("Alt+S  show an open session here"));
}

fn press(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::empty(),
    }
}

#[test]
fn input_goes_to_the_focused_pane_in_its_own_coordinates() {
    let mut app = two_cats();
    let views = app.pane_views(app.terminal_area);
    let (left, right) = (views[0].rect, views[1].rect);

    // Keys reach the focused (right) pane only.
    app.handle_key(key_char('x')).unwrap();
    wait_text(&mut app, 1, "READYx");
    assert!(grid_text(&app, 1).contains("READYx"));
    assert!(!grid_text(&app, 0).contains('x'), "left pane got a key");

    // The left app takes the alternate screen and asks for SGR mouse reports.
    app.sessions[0]
        .parser
        .process(b"\x1b[?1049h\x1b[?1000h\x1b[?1006h");
    // A press at column 5, row 3 of the left pane focuses it and reaches the
    // app pane-local: SGR is 1-based, so 6;4.
    app.handle_mouse(press(left.x + 5, left.y + 3)).unwrap();
    assert_eq!(app.active_session, Some(0));
    wait_text(&mut app, 0, "<0;6;4M");
    assert!(
        grid_text(&app, 0).contains("<0;6;4M"),
        "{:?}",
        grid_text(&app, 0)
    );

    // A paste follows the focus.
    app.handle_paste("pasted").unwrap();
    wait_text(&mut app, 0, "pasted");
    assert!(grid_text(&app, 0).contains("pasted"));
    assert!(
        !grid_text(&app, 1).contains("pasted"),
        "right pane got a paste"
    );

    // OSC 52 from both apps: only the focused pane's reaches the clipboard.
    app.config.clipboard.relay_from_pty = true;
    app.sessions[0].parser.process(b"\x1b]52;c;TEVGVA==\x07"); // LEFT
    app.sessions[1].parser.process(b"\x1b]52;c;UklHSFQ=\x07"); // RIGHT
    let mut relayed = Vec::new();
    app.relay_pty_clipboard_with(|b64| {
        relayed.push(b64.to_string());
        Ok(())
    });
    assert_eq!(relayed, vec!["TEVGVA==".to_string()]);

    // A press on the rule between the panes is SSHub's: no focus change.
    app.handle_mouse(press(left.right(), left.y + 3)).unwrap();
    assert_eq!(app.active_session, Some(0));
    app.handle_mouse(press(right.x, right.y)).unwrap();
    assert_eq!(app.active_session, Some(1));
}
