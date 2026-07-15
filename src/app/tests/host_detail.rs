use super::*;

#[test]
pub(crate) fn reload_hosts_builds_entries_with_metadata_defaults() {
    let app = test_app(vec![("alpha", host("alpha")), ("beta", host("beta"))]);
    assert_eq!(app.hosts.len(), 2);
    assert_eq!(app.filtered_indices, vec![0, 1]);
    assert_eq!(app.hosts[0].name(), "alpha");
    if let HostEntry::Legacy { meta, .. } = &app.hosts[0] {
        assert_eq!(meta.host_name, "alpha");
    }
}

#[test]
pub(crate) fn favourite_toggle_updates_metadata() {
    let mut app = test_app(vec![("web", host("web"))]);
    assert!(!app.hosts[0].favorite());

    app.handle_key(key_char('f')).unwrap();
    assert!(app.hosts[0].favorite());

    app.handle_key(key_char('f')).unwrap();
    assert!(!app.hosts[0].favorite());
}

#[test]
pub(crate) fn e_enters_host_detail_with_edit_buffers() {
    let mut app = test_app(vec![("web", host("web"))]);
    legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
    legacy_meta(&mut app.hosts[0]).description = Some("Primary".into());
    legacy_meta(&mut app.hosts[0]).environment = Some("staging".into());

    // HostDetail is the fallback metadata editor (used when an ssh_config
    // alias can't be materialized). Drive it directly.
    app.enter_host_detail().unwrap();
    assert_eq!(app.mode, AppMode::HostDetail);
    let edit = app.detail_edit.as_ref().unwrap();
    assert_eq!(edit.tags, "prod");
    assert_eq!(edit.description, "Primary");
    assert_eq!(edit.environment, "staging");

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.detail_edit.is_none());
}

#[test]
pub(crate) fn host_detail_save_persists_metadata() {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let resolver = MockResolver::new(vec![("web", host("web"))]);
    let (launcher, _launched) = RecordingLauncher::new();
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata: Arc::clone(&metadata),
            store: test_store(),
            launcher: Box::new(launcher),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();

    app.enter_host_detail().unwrap();
    app.handle_key(key_char('p')).unwrap();
    app.handle_key(key_char('r')).unwrap();
    app.handle_key(key_char('o')).unwrap();
    app.handle_key(key_char('d')).unwrap();
    app.handle_key(key(KeyCode::Tab)).unwrap();
    app.handle_key(key_char('n')).unwrap();
    app.handle_key(key_char('o')).unwrap();
    app.handle_key(key_char('t')).unwrap();
    app.handle_key(key_char('e')).unwrap();
    app.handle_key(key(KeyCode::Tab)).unwrap();
    app.handle_key(key_char('d')).unwrap();
    app.handle_key(key_char('e')).unwrap();
    app.handle_key(key_char('v')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.hosts[0].tags(), &["prod".to_string()]);
    assert_eq!(app.hosts[0].description(), Some("note"));
    assert_eq!(app.hosts[0].environment(), Some("dev"));

    let stored = metadata.get("web").unwrap().unwrap();
    assert_eq!(stored.tags, vec!["prod".to_string()]);
    assert_eq!(stored.description.as_deref(), Some("note"));
    assert_eq!(stored.environment.as_deref(), Some("dev"));
}

#[test]
pub(crate) fn host_detail_save_preserves_session_logging_override() {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let resolver = MockResolver::new(vec![("web", host("web"))]);
    let (launcher, _launched) = RecordingLauncher::new();
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata: Arc::clone(&metadata),
            store: test_store(),
            launcher: Box::new(launcher),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    legacy_meta(&mut app.hosts[0]).session_logging = crate::session_log::SessionLoggingOverride::On;
    metadata.upsert(legacy_meta(&mut app.hosts[0])).unwrap();

    app.enter_host_detail().unwrap();
    app.handle_key(key_char('x')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(
        app.hosts[0].session_logging_override(),
        crate::session_log::SessionLoggingOverride::On
    );
    let stored = metadata.get("web").unwrap().unwrap();
    assert_eq!(
        stored.session_logging,
        crate::session_log::SessionLoggingOverride::On
    );
}

#[test]
pub(crate) fn host_detail_esc_discards_unsaved_edits() {
    let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
    let resolver = MockResolver::new(vec![("web", host("web"))]);
    let (launcher, _launched) = RecordingLauncher::new();
    let mut app = App::new_with_deps(
        AppConfig::default(),
        AppDeps {
            resolver: Box::new(resolver),
            metadata: Arc::clone(&metadata),
            store: test_store(),
            launcher: Box::new(launcher),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    legacy_meta(&mut app.hosts[0]).description = Some("saved".into());
    metadata.upsert(legacy_meta(&mut app.hosts[0])).unwrap();

    app.enter_host_detail().unwrap();
    app.handle_key(key_char('x')).unwrap();
    app.handle_key(key(KeyCode::Esc)).unwrap();

    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.hosts[0].description(), Some("saved"));
}

#[test]
pub(crate) fn favourite_toggle_works_in_host_detail() {
    let mut app = test_app(vec![("web", host("web"))]);
    app.enter_host_detail().unwrap();
    app.handle_key(key_char('f')).unwrap();
    assert!(app.hosts[0].favorite());
}
