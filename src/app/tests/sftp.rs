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
