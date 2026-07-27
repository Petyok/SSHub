//! `App`-level ownership of the active runtime theme.
//!
//! Every test here builds its themes directory in a `tempfile::tempdir()` and
//! drives `App::load_themes_from` explicitly, so nothing touches the real HOME,
//! the real SSHub config, a database or the keyring.

use super::*;
use crate::theme::manager::ThemeManager;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// A syntactically valid user theme that extends `default`.
fn user_theme(name: &str) -> String {
    format!(
        "schema_version = 1\nname = \"{name}\"\nextends = \"default\"\n\n\
         [semantic]\naccent = \"#ff00ff\"\n"
    )
}

/// A `themes/` directory holding exactly the given `<id>.toml` files.
fn themes_dir_with(files: &[(&str, &str)]) -> TempDir {
    let root = tempfile::tempdir().unwrap();
    let themes = root.path().join("themes");
    fs::create_dir(&themes).unwrap();
    for (id, body) in files {
        fs::write(themes.join(format!("{id}.toml")), body).unwrap();
    }
    root
}

fn themes_path(root: &TempDir) -> std::path::PathBuf {
    root.path().join("themes")
}

/// An app with `appearance.active_theme` preset, still on the built-ins-only
/// manager that `new_with_deps` installs.
fn app_wanting(theme_id: &str) -> App {
    let mut app = test_app(vec![]);
    app.config.appearance.active_theme = theme_id.to_string();
    app
}

#[test]
fn new_with_deps_starts_on_the_built_ins_without_touching_the_filesystem() {
    let app = test_app(vec![]);
    assert_eq!(app.theme_manager.active_id(), "default");
    assert_eq!(
        app.theme_manager.themes_dir(),
        Path::new(""),
        "the test constructor must not point at any themes directory"
    );
    assert!(app.theme_manager.startup_diagnostics().is_empty());
    assert!(app.host_notice.is_none());
    // `App::theme()` is the accessor every renderer will use.
    assert_eq!(app.theme().id.as_str(), "default");
}

#[test]
fn a_user_theme_from_the_themes_directory_becomes_active() {
    let root = themes_dir_with(&[("mine", &user_theme("Mine"))]);
    let mut app = app_wanting("mine");
    app.load_themes_from(&themes_path(&root));

    assert_eq!(app.theme_manager.active_id(), "mine");
    assert_eq!(app.theme_manager.saved_id(), "mine");
    assert!(
        app.host_notice.is_none(),
        "a clean load must not nag: {:?}",
        app.host_notice
    );
}

#[test]
fn an_unreadable_themes_directory_still_yields_a_working_app() {
    // `themes` is a regular file, so `read_dir` fails with ENOTDIR — a real
    // `ThemeRegistryError`, and deterministic even when running as root (unlike
    // a chmod-000 directory).
    let root = tempfile::tempdir().unwrap();
    let themes = root.path().join("themes");
    fs::write(&themes, "not a directory").unwrap();

    let mut app = app_wanting("aqua");
    app.load_themes_from(&themes);

    // Degraded to the built-ins rather than failing: `default` is embedded, so
    // start-up never depends on the themes directory being readable.
    assert_eq!(
        app.theme_manager.active_id(),
        "aqua",
        "the built-ins must still satisfy a built-in active_theme"
    );
    assert!(app.theme_manager.registry().get("default").is_some());
    let notice = app
        .host_notice
        .as_deref()
        .expect("an unreadable themes directory must be explained");
    assert!(
        notice.contains("built-in"),
        "the notice must say we degraded: {notice}"
    );
}

#[test]
fn a_missing_active_theme_falls_back_without_rewriting_config() {
    let root = themes_dir_with(&[("mine", &user_theme("Mine"))]);
    let themes = themes_path(&root);
    let config_before: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();

    let mut app = app_wanting("does-not-exist");
    app.load_themes_from(&themes);

    assert_eq!(app.theme_manager.active_id(), "default");
    assert_eq!(
        app.theme_manager.saved_id(),
        "does-not-exist",
        "the configured id must survive the fallback"
    );
    assert_eq!(
        app.config.appearance.active_theme, "does-not-exist",
        "the in-memory config must not be rewritten either"
    );
    assert!(app.host_notice.is_some());

    // No config.toml was created and no file was added or removed anywhere.
    let config_after: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(config_before, config_after, "the fallback wrote to disk");
    assert!(!root.path().join("config.toml").exists());
    assert_eq!(fs::read_dir(&themes).unwrap().count(), 1);
}

#[test]
fn an_invalid_active_theme_falls_back_and_is_explained() {
    let root = themes_dir_with(&[("broken", "schema_version = 99\nname = \"Broken\"\n")]);
    let mut app = app_wanting("broken");
    app.load_themes_from(&themes_path(&root));

    assert_eq!(app.theme_manager.active_id(), "default");
    assert_eq!(app.theme_manager.saved_id(), "broken");
    let notice = app.host_notice.as_deref().expect("must be explained");
    assert!(
        notice.contains("broken"),
        "the notice must name the theme: {notice}"
    );
    // The broken file is left exactly as the user wrote it.
    assert_eq!(
        fs::read_to_string(themes_path(&root).join("broken.toml")).unwrap(),
        "schema_version = 99\nname = \"Broken\"\n"
    );
}

#[test]
fn a_directory_level_warning_reaches_the_notice_even_with_a_working_theme() {
    // A `*.toml` path that is a directory is only a *warning*, and it is
    // precisely what explains a theme missing from the picker — so it must not
    // be filtered out just because nothing failed.
    let root = themes_dir_with(&[("mine", &user_theme("Mine"))]);
    fs::create_dir(themes_path(&root).join("bundle.toml")).unwrap();

    let mut app = app_wanting("mine");
    app.load_themes_from(&themes_path(&root));

    assert_eq!(app.theme_manager.active_id(), "mine");
    let notice = app
        .host_notice
        .as_deref()
        .expect("a directory-level warning must be surfaced");
    assert!(
        notice.contains("not a readable theme file"),
        "unexpected notice: {notice}"
    );
}

#[test]
fn theme_returns_the_managers_active_theme() {
    let root = themes_dir_with(&[("mine", &user_theme("Mine"))]);
    let mut app = app_wanting("mine");
    app.load_themes_from(&themes_path(&root));

    let via_app = app.theme() as *const _;
    let via_manager = app.theme_manager.theme() as *const _;
    assert_eq!(
        via_app, via_manager,
        "App::theme() must hand out the manager's active theme, not a copy"
    );
}

#[test]
fn loading_themes_twice_replaces_the_manager_wholesale() {
    let first = themes_dir_with(&[("mine", &user_theme("Mine"))]);
    let second = themes_dir_with(&[("other", &user_theme("Other"))]);

    let mut app = app_wanting("mine");
    app.load_themes_from(&themes_path(&first));
    assert_eq!(app.theme_manager.active_id(), "mine");

    app.config.appearance.active_theme = "other".to_string();
    app.load_themes_from(&themes_path(&second));
    assert_eq!(app.theme_manager.active_id(), "other");
    assert!(
        app.theme_manager.registry().get("mine").is_none(),
        "the previous directory must not linger in the registry"
    );
}

#[test]
fn a_builtins_manager_can_be_installed_without_any_path() {
    // What `App::new` falls back to when there is no config directory at all.
    let mut app = app_wanting("fire");
    app.theme_manager = ThemeManager::builtins("fire");
    assert_eq!(app.theme_manager.active_id(), "fire");
    assert_eq!(app.theme().id.as_str(), "fire");
}
