use super::*;

#[test]
pub(crate) fn esc_exits_search_and_clears_query_and_tag_filter() {
    let mut app = test_app(vec![("alpha", host("alpha"))]);
    app.tag_filters = vec!["prod".into()];
    app.mode = AppMode::Search;
    app.search_query = "al".into();

    app.handle_key(key(KeyCode::Esc)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.search_query.is_empty());
    assert!(app.tag_filters.is_empty());
}

#[test]
pub(crate) fn parse_tags_splits_and_trims() {
    assert_eq!(
        parse_tags(" prod , db , , staging "),
        vec!["prod", "db", "staging"]
    );
}

#[test]
pub(crate) fn tag_filter_narrows_candidates_before_search() {
    let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.tag_filters = vec!["prod".into()];
    app.rebuild_filter();

    assert_eq!(app.filtered_indices, vec![0]);
}

#[test]
pub(crate) fn tag_filter_picker_arrow_selects_and_applies_tag() {
    let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    // Rows are ["(all)", "prod", "staging"]; row 0 selected by default.
    assert_eq!(app.tag_filter_rows(), vec!["(all)", "prod", "staging"]);
    assert_eq!(app.tag_filter_selected, 0);

    // Arrow down twice lands on "staging" and Enter toggles + applies it.
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.tag_filters, vec!["staging".to_string()]);
    assert_eq!(app.filtered_indices, vec![1]);
}

#[test]
pub(crate) fn tag_filter_picker_space_toggles_multiple_tags_and_ands_them() {
    let mut app = test_app(vec![
        ("web", host("web")),
        ("db", host("db")),
        ("both", host("both")),
    ]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["eu".into()];
    legacy_meta(&mut app.hosts[2]).tags = vec!["prod".into(), "eu".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    // Rows: ["(all)", "eu", "prod"]. Space toggles a tag and stays open.
    app.handle_key(key(KeyCode::Down)).unwrap(); // → "eu"
    app.handle_key(key_char(' ')).unwrap();
    assert_eq!(app.mode, AppMode::TagFilter, "stays open after Space");
    assert_eq!(app.tag_filters, vec!["eu".to_string()]);

    app.handle_key(key(KeyCode::Down)).unwrap(); // → "prod"
    app.handle_key(key_char(' ')).unwrap();
    assert_eq!(app.tag_filters, vec!["eu".to_string(), "prod".to_string()]);

    // AND semantics: only the host carrying both tags survives.
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.filtered_indices, vec![2]);
}

#[test]
pub(crate) fn tag_filter_picker_enter_after_multiselect_keeps_all_tags() {
    // Regression: Enter must confirm the built-up set, never remove the
    // last-highlighted tag.
    let mut app = test_app(vec![
        ("web", host("web")),
        ("db", host("db")),
        ("both", host("both")),
    ]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["eu".into()];
    legacy_meta(&mut app.hosts[2]).tags = vec!["prod".into(), "eu".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap(); // → "eu"
    app.handle_key(key_char(' ')).unwrap(); // toggle eu on
    app.handle_key(key(KeyCode::Down)).unwrap(); // → "prod"
    app.handle_key(key_char(' ')).unwrap(); // toggle prod on
                                            // Cursor still on "prod" (active). Enter must NOT toggle it off.
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.tag_filters, vec!["eu".to_string(), "prod".to_string()]);
    assert_eq!(app.filtered_indices, vec![2]);
}

#[test]
pub(crate) fn tag_filter_picker_space_toggles_tag_off() {
    let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.tag_filters = vec!["prod".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap(); // → "prod" (already active)
    app.handle_key(key_char(' ')).unwrap(); // toggle off
    assert!(app.tag_filters.is_empty());
    assert_eq!(app.filtered_indices.len(), 2);
}

#[test]
pub(crate) fn tag_filter_picker_all_row_clears_filter() {
    let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.tag_filters = vec!["prod".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    // Cursor opens on the "(all)" row.
    assert_eq!(app.tag_filter_selected, 0);

    // Enter on "(all)" clears every active filter and closes.
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert!(app.tag_filters.is_empty());
    assert_eq!(app.filtered_indices.len(), 2);
}

#[test]
pub(crate) fn tag_filter_picker_esc_keeps_active_filter() {
    let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.tag_filters = vec!["prod".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    // Esc closes the picker without touching the active filter.
    app.handle_key(key(KeyCode::Esc)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.tag_filters, vec!["prod".to_string()]);
    assert_eq!(app.filtered_indices, vec![0]);
}

#[test]
pub(crate) fn hash_enters_tag_filter_and_enter_applies() {
    let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
    app.rebuild_filter();

    app.handle_key(key_char('#')).unwrap();
    assert_eq!(app.mode, AppMode::TagFilter);

    app.handle_key(key_char('p')).unwrap();
    app.handle_key(key_char('r')).unwrap();
    app.handle_key(key_char('o')).unwrap();
    app.handle_key(key_char('d')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();

    // Enter toggles the highlighted match, applies it and returns to Normal
    // so the list can be navigated while filtered.
    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.tag_filters, vec!["prod".to_string()]);
    assert_eq!(app.filtered_indices, vec![0]);

    // Esc in Normal clears the active tag filter.
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.tag_filters.is_empty());
    assert_eq!(app.filtered_indices.len(), 2);
}
