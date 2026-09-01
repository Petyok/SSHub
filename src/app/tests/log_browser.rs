use super::*;
use crate::app::LogBrowserView;
use std::io::Write;
use std::path::{Path, PathBuf};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn ch(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(ch(c)).unwrap();
    }
}

fn write_seg(dir: &Path, name: &str, body: &[u8]) {
    std::fs::create_dir_all(dir).unwrap();
    let mut f = std::fs::File::create(dir.join(name)).unwrap();
    f.write_all(body).unwrap();
}

/// Build an app plus a temp `logs/` root holding one host with a segment whose
/// content has known searchable lines.
fn app_with_logs() -> (App, tempfile::TempDir, PathBuf) {
    let app = test_app(vec![]);
    let tmp = tempfile::tempdir().unwrap();
    let logs_root = tmp.path().join("logs");
    write_seg(
        &logs_root.join("web-1"),
        "1000-1-0.log",
        b"\x1b[32m$ deploy\x1b[0m\nstarting build\nBUILD ok\ndone\n",
    );
    write_seg(&logs_root.join("web-1"), "2000-1-0.log", b"second run\n");
    (app, tmp, logs_root)
}

#[test]
fn browse_hosts_segments_and_open_viewer() {
    let (mut app, _tmp, root) = app_with_logs();
    app.open_log_browser_at(root).unwrap();
    assert_eq!(app.mode, AppMode::LogBrowser);

    let s = app.log_browser.as_ref().unwrap();
    assert_eq!(s.view, LogBrowserView::Hosts);
    assert_eq!(s.hosts.len(), 1);
    assert_eq!(s.hosts[0].dir_name, "web-1");
    assert_eq!(s.hosts[0].segment_count, 2);

    // Enter opens the host's segment list.
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let s = app.log_browser.as_ref().unwrap();
    assert_eq!(s.view, LogBrowserView::Segments);
    assert_eq!(s.segments.len(), 2);
    // Newest first.
    assert_eq!(s.segments[0].file_name, "2000-1-0.log");

    // Move to the older segment and open it.
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let s = app.log_browser.as_ref().unwrap();
    assert_eq!(s.view, LogBrowserView::Viewer);
    assert_eq!(s.current_seg.as_deref(), Some("1000-1-0.log"));
    // ANSI stripped.
    assert_eq!(s.lines[0], "$ deploy");
    assert_eq!(s.lines[1], "starting build");
}

#[test]
fn search_jumps_to_match() {
    let (mut app, _tmp, root) = app_with_logs();
    app.open_log_browser_at(root).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap(); // host -> segments
    app.handle_key(key(KeyCode::Down)).unwrap(); // older segment
    app.handle_key(key(KeyCode::Enter)).unwrap(); // -> viewer

    // Search for "build": case-insensitive, matches lines 1 and 2 (0-based).
    app.handle_key(ch('/')).unwrap();
    type_text(&mut app, "build");
    app.handle_key(key(KeyCode::Enter)).unwrap();

    let s = app.log_browser.as_ref().unwrap();
    assert_eq!(s.matches, vec![1, 2]);
    assert_eq!(s.scroll, 1); // jumped to first match
    assert!(!s.searching);

    // n advances to the next match.
    app.handle_key(ch('n')).unwrap();
    assert_eq!(app.log_browser.as_ref().unwrap().scroll, 2);
    // n wraps back to the first.
    app.handle_key(ch('n')).unwrap();
    assert_eq!(app.log_browser.as_ref().unwrap().scroll, 1);
}

#[test]
fn bookmark_a_line_and_jump_back() {
    let (mut app, _tmp, root) = app_with_logs();
    app.open_log_browser_at(root).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap(); // viewer on 1000-...

    // Scroll to line index 2 ("BUILD ok"), then bookmark it.
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(app.log_browser.as_ref().unwrap().scroll, 2);

    app.handle_key(ch('b')).unwrap();
    assert!(app.log_browser.as_ref().unwrap().naming.is_some());
    type_text(&mut app, "build point");
    app.handle_key(key(KeyCode::Enter)).unwrap();

    // Persisted to the store.
    let saved = app.store().list_log_bookmarks().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "build point");
    assert_eq!(saved[0].line, 2);
    assert_eq!(saved[0].host_dir, "web-1");

    // Scroll away, then jump back via the bookmarks list.
    app.handle_key(key(KeyCode::Home)).unwrap();
    assert_eq!(app.log_browser.as_ref().unwrap().scroll, 0);
    app.handle_key(ch('m')).unwrap();
    assert!(app.log_browser.as_ref().unwrap().show_bookmarks);
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let s = app.log_browser.as_ref().unwrap();
    assert_eq!(s.scroll, 2);
    assert!(!s.show_bookmarks);
}

#[test]
fn delete_bookmark_from_the_list() {
    let (mut app, _tmp, root) = app_with_logs();
    app.open_log_browser_at(root).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap(); // newest segment viewer
    app.handle_key(ch('b')).unwrap();
    type_text(&mut app, "mark");
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.store().list_log_bookmarks().unwrap().len(), 1);

    app.handle_key(ch('m')).unwrap();
    app.handle_key(ch('d')).unwrap();
    assert!(app.store().list_log_bookmarks().unwrap().is_empty());
}

#[test]
fn esc_walks_back_out_of_the_browser() {
    let (mut app, _tmp, root) = app_with_logs();
    app.open_log_browser_at(root).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap(); // segments
    app.handle_key(key(KeyCode::Enter)).unwrap(); // viewer
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(
        app.log_browser.as_ref().unwrap().view,
        LogBrowserView::Segments
    );
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(
        app.log_browser.as_ref().unwrap().view,
        LogBrowserView::Hosts
    );
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.log_browser.is_none());
}

#[test]
fn empty_logs_root_opens_with_no_hosts() {
    let mut app = test_app(vec![]);
    let tmp = tempfile::tempdir().unwrap();
    app.open_log_browser_at(tmp.path().join("logs")).unwrap();
    assert_eq!(app.mode, AppMode::LogBrowser);
    assert!(app.log_browser.as_ref().unwrap().hosts.is_empty());
    // Enter on an empty list is a no-op, stays on Hosts.
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(
        app.log_browser.as_ref().unwrap().view,
        LogBrowserView::Hosts
    );
}
