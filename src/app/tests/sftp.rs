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
