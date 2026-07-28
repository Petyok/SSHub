//! `App`-level ownership of the active runtime theme.
//!
//! Every test here builds its themes directory in a `tempfile::tempdir()` and
//! drives `App::load_themes_from` explicitly, so nothing touches the real HOME,
//! the real SSHub config, a database or the keyring.

use super::*;
use crate::theme::manager::ThemeManager;
use std::cell::{Cell, RefCell};
use std::fs;
use std::rc::Rc;
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
        None,
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

#[test]
fn a_failed_theme_directory_is_still_the_reload_target() {
    // `themes` is a regular file, so the directory cannot be read at all.
    let root = tempfile::tempdir().unwrap();
    let themes = themes_path(&root);
    fs::write(&themes, "not a directory").unwrap();

    let mut app = app_wanting("mine");
    app.load_themes_from(&themes);

    // Degraded as designed: built-ins active, configured id preserved, notice
    // shown — but the manager must still know *which* directory failed.
    assert_eq!(app.theme_manager.active_id(), "default");
    assert_eq!(app.theme_manager.saved_id(), "mine");
    assert!(app.host_notice.is_some());
    assert_eq!(
        app.theme_manager.themes_dir(),
        Some(themes.as_path()),
        "a degraded manager must keep the directory a reload has to retry"
    );

    // Repairing the directory and reloading through that same path restores the
    // user's choice — the reload-after-repair Task 10 depends on.
    let reload_target = app.theme_manager.themes_dir().unwrap().to_path_buf();
    fs::remove_file(&themes).unwrap();
    fs::create_dir(&themes).unwrap();
    fs::write(themes.join("mine.toml"), user_theme("Mine")).unwrap();
    app.load_themes_from(&reload_target);

    assert_eq!(app.theme_manager.active_id(), "mine");
    assert_eq!(app.theme_manager.themes_dir(), Some(themes.as_path()));
}

/// Row index of the Theme action in [`SETTINGS_ITEMS`]. The spec puts it first,
/// above the boolean toggles, so this is a constant rather than a search — a
/// search would make the "Theme is the first row" assertion below tautological.
fn theme_setting_index() -> usize {
    0
}

/// Flipping a row is pure — `handle_key_settings` is what persists — so these
/// tests never write a config file and need no filesystem isolation.
#[test]
fn typed_settings_preserve_every_existing_toggle() {
    let mut app = test_app(vec![]);
    for item in [
        SettingToggle::OpaqueBackground,
        SettingToggle::OsLogo,
        SettingToggle::ConfirmQuit,
        SettingToggle::DisableAnimation,
        SettingToggle::SessionLogging,
    ] {
        let before = app.setting_value(item);
        app.toggle_setting(item);
        assert_ne!(app.setting_value(item), before);
    }
}

#[test]
fn theme_row_is_an_action_and_space_does_not_toggle_it() {
    let mut app = test_app(vec![]);
    app.mode = AppMode::Settings;
    app.settings_selected = theme_setting_index();
    let before = (
        app.config.appearance.opaque_background,
        app.config.appearance.os_logo,
        app.config.appearance.confirm_quit,
        app.config.appearance.disable_animation,
        app.config.session_logging.enabled,
    );
    app.handle_key(key_char(' ')).unwrap();
    assert_eq!(app.mode, AppMode::Settings);
    assert_eq!(
        (
            app.config.appearance.opaque_background,
            app.config.appearance.os_logo,
            app.config.appearance.confirm_quit,
            app.config.appearance.disable_animation,
            app.config.session_logging.enabled,
        ),
        before
    );
    assert!(matches!(
        SETTINGS_ITEMS[theme_setting_index()].item,
        SettingItem::Theme
    ));
    assert_eq!(
        app.setting_value(SettingItem::Theme),
        None,
        "the Theme row is an action, not a boolean"
    );
}

// ── Theme picker ───────────────────────────────────────────────────────────
//
// The picker is a pure state machine over the registry plus one injected
// persist closure, so every test below runs entirely inside a `tempfile`
// themes directory and the only writes that can happen are the ones a test
// explicitly counts.

thread_local! {
    /// Keeps every temp themes directory alive for the whole test thread.
    /// The brief's test bodies hand back a bare `App`, so the `TempDir` has
    /// nowhere else to live; parking it here means the directory is unlinked
    /// when the thread ends rather than at the end of the constructor.
    static THEME_DIRS: RefCell<Vec<TempDir>> = const { RefCell::new(Vec::new()) };
}

/// A theme that is valid but carries an unknown component section, i.e.
/// `warning` in Compatible mode: previewable and savable, but flagged.
fn warning_theme(name: &str) -> String {
    format!(
        "{}\n[components.not_a_real_section_v2]\nborder = \"semantic.accent\"\n",
        user_theme(name)
    )
}

/// A theme that is fatally invalid in both validation modes.
fn invalid_theme(name: &str) -> String {
    format!("schema_version = 99\nname = \"{name}\"\n")
}

/// An app whose themes directory holds exactly `files`, already loaded.
fn app_with_files(files: &[(&str, &str)]) -> App {
    let root = themes_dir_with(files);
    let mut app = app_wanting("default");
    app.load_themes_from(&themes_path(&root));
    THEME_DIRS.with(|dirs| dirs.borrow_mut().push(root));
    app
}

/// An app that knows `ids`; built-in ids are already embedded, every other id
/// becomes a valid user theme file of the same name.
fn app_with_themes<const N: usize>(ids: [&str; N]) -> App {
    let owned: Vec<(String, String)> = ids
        .iter()
        .filter(|id| !crate::theme::builtins::is_reserved(id))
        .map(|id| ((*id).to_string(), user_theme(id)))
        .collect();
    let files: Vec<(&str, &str)> = owned
        .iter()
        .map(|(id, body)| (id.as_str(), body.as_str()))
        .collect();
    app_with_files(&files)
}

/// A fingerprint of everything under `root`, so a test can prove that an
/// operation wrote nothing at all rather than merely that it wrote nothing
/// *visible*.
fn fingerprint(root: &std::path::Path) -> Vec<(String, u64, Option<std::time::SystemTime>)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let meta = entry.metadata().unwrap();
        out.push((
            entry.file_name().to_string_lossy().into_owned(),
            meta.len(),
            meta.modified().ok(),
        ));
        if meta.is_dir() {
            out.extend(fingerprint(&entry.path()));
        }
    }
    out.sort();
    out
}

impl App {
    /// The themes directory a test's app was built around.
    fn test_themes_dir(&self) -> std::path::PathBuf {
        self.theme_manager
            .themes_dir()
            .expect("test apps always own a themes directory")
            .to_path_buf()
    }

    /// Delete a user theme file behind the picker's back and reload, which is
    /// exactly what a user does in another terminal while the picker is open.
    fn remove_user_theme_and_reload(&mut self, id: &str) {
        fs::remove_file(self.test_themes_dir().join(format!("{id}.toml"))).unwrap();
        self.reload_theme_picker();
    }

    /// Index of the row for `id` in the picker's current list.
    fn theme_row_index(&self, id: &str) -> usize {
        self.theme_picker_rows()
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("no picker row for `{id}`"))
    }
}

#[test]
fn escape_restores_the_captured_rc_when_original_file_disappears() {
    let mut app = app_with_themes(["default", "ocean"]);
    app.activate_theme("ocean");
    let original = app.theme_manager.active_rc();
    app.open_theme_picker();
    app.remove_user_theme_and_reload("ocean");
    app.cancel_theme_picker();
    assert!(Rc::ptr_eq(&original, &app.theme_manager.active_rc()));
    assert_eq!(app.mode, AppMode::Settings);
}

#[test]
fn enter_on_theme_setting_opens_the_picker() {
    let mut app = test_app(vec![]);
    app.mode = AppMode::Settings;
    app.settings_selected = theme_setting_index();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.mode, AppMode::ThemePicker);
    assert!(app.theme_picker.is_some());
}

#[test]
fn failed_save_keeps_preview_active_and_saved_id_unchanged() {
    let mut app = app_with_themes(["default", "fire"]);
    app.open_theme_picker();
    app.preview_theme("fire");
    app.commit_theme_picker_with(|_| anyhow::bail!("read only"));
    assert_eq!(app.theme_manager.active_id(), "fire");
    assert_eq!(app.theme_manager.saved_id(), "default");
    assert_eq!(app.mode, AppMode::ThemePicker);
    assert!(app.theme_picker.as_ref().unwrap().error.is_some());
}

#[test]
fn opening_the_picker_captures_the_active_theme_without_activating_anything() {
    let mut app = app_with_themes(["default", "ocean"]);
    app.activate_theme("ocean");
    let before = app.theme_manager.active_rc();

    app.open_theme_picker();

    let state = app.theme_picker.as_ref().expect("picker opened");
    assert_eq!(state.original_id, "ocean");
    assert_eq!(state.preview_id, "ocean");
    assert!(Rc::ptr_eq(&state.original_theme, &before));
    assert!(
        Rc::ptr_eq(&before, &app.theme_manager.active_rc()),
        "opening must not re-resolve the live theme"
    );
    assert_eq!(
        app.theme_picker_rows()[state.selected].id,
        "ocean",
        "the picker must land on the theme that is actually active"
    );
}

#[test]
fn navigation_previews_valid_and_warning_themes_but_never_invalid_ones() {
    let mut app = app_with_files(&[
        ("nice", &user_theme("Nice")),
        ("odd", &warning_theme("Odd")),
        ("broken", &invalid_theme("Broken")),
    ]);
    app.open_theme_picker();

    // valid
    app.select_theme_row(app.theme_row_index("nice"));
    assert_eq!(app.theme_manager.active_id(), "nice");
    assert_eq!(app.theme_picker.as_ref().unwrap().preview_id, "nice");

    // warning: still previewable, and flagged
    let odd = app.theme_row_index("odd");
    app.select_theme_row(odd);
    assert_eq!(app.theme_manager.active_id(), "odd");
    assert_eq!(app.theme_picker_rows()[odd].status, ThemeRowStatus::Warning);

    // invalid: listed, selectable, but the runtime theme does not move
    let broken = app.theme_row_index("broken");
    let live = app.theme_manager.active_rc();
    app.select_theme_row(broken);
    assert_eq!(
        app.theme_picker_rows()[broken].status,
        ThemeRowStatus::Invalid
    );
    assert_eq!(app.theme_picker.as_ref().unwrap().selected, broken);
    assert_eq!(app.theme_manager.active_id(), "odd");
    assert!(
        Rc::ptr_eq(&live, &app.theme_manager.active_rc()),
        "an invalid row must never be activated"
    );
    assert_eq!(app.theme_picker.as_ref().unwrap().preview_id, "odd");
}

#[test]
fn reload_adopts_a_repaired_theme_file() {
    let mut app = app_with_files(&[("mine", &invalid_theme("Mine"))]);
    app.open_theme_picker();
    let mine = app.theme_row_index("mine");
    app.select_theme_row(mine);
    assert_eq!(app.theme_manager.active_id(), "default");

    fs::write(
        app.test_themes_dir().join("mine.toml"),
        user_theme("Repaired"),
    )
    .unwrap();
    app.reload_theme_picker();

    let mine = app.theme_row_index("mine");
    assert_eq!(app.theme_picker_rows()[mine].status, ThemeRowStatus::Valid);
    assert_eq!(app.theme_picker.as_ref().unwrap().selected, mine);
    assert_eq!(
        app.theme_manager.active_id(),
        "mine",
        "a repaired file must become the live preview immediately"
    );
}

#[test]
fn reload_after_deleting_the_preview_keeps_the_theme_and_leaves_a_tombstone() {
    let mut app = app_with_themes(["default", "ocean"]);
    app.open_theme_picker();
    let ocean = app.theme_row_index("ocean");
    app.select_theme_row(ocean);
    let previewed = app.theme_manager.active_rc();

    app.remove_user_theme_and_reload("ocean");

    let state = app.theme_picker.as_ref().unwrap();
    assert_eq!(
        state.tombstone.as_ref().map(|t| t.id.as_str()),
        Some("ocean"),
        "the removed entry must keep its slot"
    );
    assert_eq!(state.selected, ocean);
    let rows = app.theme_picker_rows();
    assert_eq!(rows[ocean].id, "ocean");
    assert_eq!(rows[ocean].status, ThemeRowStatus::Invalid);
    assert!(
        Rc::ptr_eq(&previewed, &app.theme_manager.active_rc()),
        "the last valid runtime theme stays active"
    );
}

#[test]
fn a_failed_reload_keeps_the_active_theme_and_explains_itself() {
    let mut app = app_with_themes(["default", "ocean"]);
    app.open_theme_picker();
    app.preview_theme("ocean");
    let live = app.theme_manager.active_rc();

    // Replace the directory with a regular file: `read_dir` now fails with
    // ENOTDIR, deterministically and even as root.
    let dir = app.test_themes_dir();
    fs::remove_dir_all(&dir).unwrap();
    fs::write(&dir, "not a directory").unwrap();
    app.reload_theme_picker();

    assert!(Rc::ptr_eq(&live, &app.theme_manager.active_rc()));
    assert_eq!(app.theme_manager.active_id(), "ocean");
    assert_eq!(app.mode, AppMode::ThemePicker);
    let error = app.theme_picker.as_ref().unwrap().error.as_deref();
    assert!(
        error.is_some_and(|e| e.contains("could not be read")),
        "a failed reload must explain itself: {error:?}"
    );
    assert!(
        app.theme_picker_rows().iter().any(|row| row.id == "ocean"),
        "a failed reload must not drop the list it could not replace"
    );
}

#[test]
fn reload_without_a_themes_directory_is_a_no_op() {
    // `new_with_deps` builds a manager that belongs to no directory at all.
    // A reload there must do nothing — and above all must never read the
    // working directory, which an empty `PathBuf` would have meant.
    let mut app = test_app(vec![]);
    assert_eq!(app.theme_manager.themes_dir(), None);
    app.open_theme_picker();
    let live = app.theme_manager.active_rc();

    app.reload_theme_picker();

    assert!(Rc::ptr_eq(&live, &app.theme_manager.active_rc()));
    assert_eq!(app.mode, AppMode::ThemePicker);
    assert_eq!(app.theme_picker.as_ref().unwrap().tombstone, None);
    assert_eq!(app.theme_picker_rows().len(), 5, "the built-ins remain");
}

#[test]
fn enter_on_an_invalid_theme_writes_nothing_and_keeps_the_picker_open() {
    let mut app = app_with_files(&[("broken", &invalid_theme("Broken"))]);
    app.open_theme_picker();
    app.select_theme_row(app.theme_row_index("broken"));

    let writes = Cell::new(0usize);
    app.commit_theme_picker_with(|_| {
        writes.set(writes.get() + 1);
        Ok(())
    });

    assert_eq!(writes.get(), 0, "an invalid theme must never be persisted");
    assert_eq!(app.mode, AppMode::ThemePicker);
    assert_eq!(app.theme_manager.active_id(), "default");
    assert_eq!(app.theme_manager.saved_id(), "default");
    assert!(app.theme_picker.as_ref().unwrap().error.is_some());
}

#[test]
fn a_successful_commit_writes_exactly_once_and_closes_the_picker() {
    let mut app = app_with_themes(["default", "ocean"]);
    app.open_theme_picker();
    app.preview_theme("ocean");

    let writes = Cell::new(0usize);
    let saved = RefCell::new(String::new());
    app.commit_theme_picker_with(|config| {
        writes.set(writes.get() + 1);
        saved.replace(config.appearance.active_theme.clone());
        Ok(())
    });

    assert_eq!(writes.get(), 1, "a commit must persist exactly once");
    assert_eq!(saved.into_inner(), "ocean");
    assert_eq!(app.config.appearance.active_theme, "ocean");
    assert_eq!(app.theme_manager.active_id(), "ocean");
    assert_eq!(app.theme_manager.saved_id(), "ocean");
    assert_eq!(app.mode, AppMode::Settings);
    assert!(app.theme_picker.is_none());
}

#[test]
fn open_navigation_reload_and_escape_never_write() {
    let mut app = app_with_files(&[
        ("ocean", &user_theme("Ocean")),
        ("broken", &invalid_theme("Broken")),
    ]);
    let dir = app.test_themes_dir();
    let root = dir.parent().unwrap().to_path_buf();
    let before = fingerprint(&root);

    app.open_theme_picker();
    app.select_theme_row(app.theme_row_index("ocean"));
    app.select_theme_row(app.theme_row_index("broken"));
    app.move_theme_selection(1);
    app.move_theme_selection(-1);
    app.reload_theme_picker();
    app.cancel_theme_picker();

    assert_eq!(
        before,
        fingerprint(&root),
        "opening, navigating, reloading and cancelling must not touch the disk"
    );
    assert!(!root.join("config.toml").exists());
    assert_eq!(app.theme_manager.saved_id(), "default");
    assert_eq!(app.mode, AppMode::Settings);
}

#[test]
fn a_user_file_squatting_a_reserved_id_is_listed_as_invalid() {
    // The exact file a confused user needs explained. `registry.get("aqua")`
    // answers with the built-in, so the picker must enumerate `records()`.
    let mut app = app_with_files(&[("aqua", &user_theme("Not Really Aqua"))]);
    app.open_theme_picker();

    let rows = app.theme_picker_rows();
    let squatter = rows
        .iter()
        .find(|row| row.id == "aqua" && !row.builtin)
        .expect("the squatting user file must be listed");
    assert_eq!(squatter.status, ThemeRowStatus::Invalid);
    assert!(
        squatter.path.is_some(),
        "a user row must show where the file lives"
    );
    assert!(
        !squatter.diagnostics.is_empty(),
        "the picker must be able to say why it is unusable"
    );
    assert!(
        rows.iter().any(|row| row.id == "aqua" && row.builtin),
        "the built-in must still be listed and usable"
    );
}

#[test]
fn built_ins_are_listed_first_in_their_frozen_order_then_user_themes() {
    let app = app_with_files(&[
        ("zulu", &user_theme("Alpha")),
        ("alpha", &user_theme("Zulu")),
    ]);
    let rows = app.theme_picker_rows();
    let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "default",
            "summer",
            "aqua",
            "fire",
            "high-contrast",
            // User themes sort by display name first, id second — `zulu.toml`
            // is named "Alpha", so it leads.
            "zulu",
            "alpha",
        ]
    );
    assert!(rows[..5].iter().all(|row| row.builtin));
    assert!(rows[5..].iter().all(|row| !row.builtin));
}

#[test]
fn arrow_keys_wrap_and_home_end_jump_to_the_edges() {
    let mut app = app_with_themes(["default"]);
    app.mode = AppMode::Settings;
    app.settings_selected = theme_setting_index();
    // Paging is sized from the terminal, so give the app a real one — and one
    // big enough that a page covers this short list, which is what lets the
    // PageDown/PageUp assertions below name the first and last row exactly.
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 120, 40);
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let last = app.theme_picker_rows().len() - 1;

    assert_eq!(app.theme_picker.as_ref().unwrap().selected, 0);
    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(app.theme_picker.as_ref().unwrap().selected, last);
    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(app.theme_picker.as_ref().unwrap().selected, 0);
    app.handle_key(key(KeyCode::End)).unwrap();
    assert_eq!(app.theme_picker.as_ref().unwrap().selected, last);
    app.handle_key(key(KeyCode::Home)).unwrap();
    assert_eq!(app.theme_picker.as_ref().unwrap().selected, 0);
    app.handle_key(key(KeyCode::PageDown)).unwrap();
    assert_eq!(app.theme_picker.as_ref().unwrap().selected, last);
    app.handle_key(key(KeyCode::PageUp)).unwrap();
    assert_eq!(app.theme_picker.as_ref().unwrap().selected, 0);
}

#[test]
fn escape_closes_the_picker_back_to_settings_and_restores_the_theme() {
    let mut app = app_with_themes(["default", "ocean"]);
    app.open_theme_picker();
    let original = app.theme_manager.active_rc();
    app.preview_theme("ocean");
    assert_eq!(app.theme_manager.active_id(), "ocean");

    app.handle_key(key(KeyCode::Esc)).unwrap();

    assert_eq!(app.mode, AppMode::Settings);
    assert!(app.theme_picker.is_none());
    assert_eq!(app.theme_manager.active_id(), "default");
    assert!(
        Rc::ptr_eq(&original, &app.theme_manager.active_rc()),
        "the theme that was live when the picker opened must be back"
    );
}

#[test]
fn the_reload_key_is_wired_to_the_picker() {
    let mut app = app_with_files(&[("mine", &invalid_theme("Mine"))]);
    app.open_theme_picker();
    app.select_theme_row(app.theme_row_index("mine"));
    fs::write(
        app.test_themes_dir().join("mine.toml"),
        user_theme("Repaired"),
    )
    .unwrap();

    app.handle_key(key_char('r')).unwrap();

    assert_eq!(app.theme_manager.active_id(), "mine");
}

// ── Activation invalidates every old-theme visual state ──────
//
// The frame pipeline's background painters select cells by their *current*
// colour (`CellSelection::Matching(Color::Reset)`). A snapshot or slide taken
// under the previous theme would therefore be matched against cells carrying
// that theme's colours, so activation has to drop all of them before the next
// frame runs.

/// An app carrying one of every buffer snapshot and in-flight slide.
fn app_with_populated_visual_state() -> App {
    use ratatui::layout::Rect;
    use std::time::{Duration, Instant};

    let mut app = app_with_themes(["default"]);
    let now = Instant::now();
    let buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 8, 4));

    *app.popup_snapshot.borrow_mut() = Some((Rect::new(0, 0, 8, 4), buffer.clone()));
    *app.popup_backdrop.borrow_mut() = Some(buffer.clone());
    *app.session_snapshot.borrow_mut() = Some(buffer.clone());
    *app.sftp_snapshot.borrow_mut() = Some(buffer);
    app.popup_closing_at = Some(now);
    app.session_enter_at = Some(now);
    app.session_exit_at = Some(now);
    app.session_tab_switch = Some(crate::app::SessionTabSwitch {
        dir: 1,
        from: 0,
        at: now,
    });
    app.sftp_anim = Some((crate::app::SftpAnim::PanesIn, now));
    app.tab_switch = Some(crate::app::TabSwitch {
        from: 0,
        to: 1,
        at: now,
    });
    app.zoom_anim = Some(crate::tui::tween::SlideAnim::new(
        Rect::new(0, 0, 4, 2),
        Rect::new(0, 0, 8, 4),
        Duration::from_millis(200),
    ));
    app.mode_entered_at = now;
    app
}

#[test]
fn theme_activation_clears_every_old_theme_snapshot_and_slide() {
    let mut app = app_with_populated_visual_state();

    assert!(app.activate_theme("fire"), "`fire` is a built-in");

    assert!(app.popup_snapshot.borrow().is_none());
    assert!(app.popup_backdrop.borrow().is_none());
    assert!(app.session_snapshot.borrow().is_none());
    assert!(app.sftp_snapshot.borrow().is_none());
    assert!(app.popup_closing_at.is_none());
    assert!(app.session_enter_at.is_none());
    assert!(app.session_exit_at.is_none());
    assert!(app.session_tab_switch.is_none());
    assert!(app.sftp_anim.is_none());
    assert!(app.tab_switch.is_none());
    assert!(app.zoom_anim.is_none());
}

#[test]
fn theme_activation_settles_the_popup_open_slide() {
    // A preview repaints while the picker is open. Clearing its backdrop would
    // otherwise let the next frame re-capture one and replay the drop-in for
    // every arrow key, so activation also ages the mode clock out of the slide.
    let mut app = app_with_populated_visual_state();
    app.mode = AppMode::ThemePicker;

    assert!(app.activate_theme("fire"));

    assert!(
        app.mode_entered_at.elapsed() >= crate::tui::POPUP_ANIM,
        "the open slide must read as finished"
    );
}

#[test]
fn every_path_that_changes_the_painted_theme_invalidates_it() {
    // `theme_manager` is private, so the painted theme can only move through
    // `activate_resolved_theme` (activation within one manager) or
    // `replace_theme_manager` (a new manager). This walks every caller of both
    // and asserts each one arrives at the invalidation — the guarantee the
    // frame pipeline's `Matching(Color::Reset)` painters depend on.
    let dirty = |app: &mut App| {
        *app.session_snapshot.borrow_mut() = Some(ratatui::buffer::Buffer::empty(
            ratatui::layout::Rect::new(0, 0, 8, 4),
        ))
    };

    // ── activation within one manager ────────────────────────
    let mut app = app_with_files(&[("ocean", &user_theme("Ocean"))]);
    app.open_theme_picker();

    dirty(&mut app);
    assert!(app.activate_theme("fire"), "direct activation");
    assert!(app.session_snapshot.borrow().is_none(), "activate_theme");

    dirty(&mut app);
    app.preview_theme("ocean");
    assert!(app.session_snapshot.borrow().is_none(), "preview");

    dirty(&mut app);
    app.reload_theme_picker();
    assert!(app.session_snapshot.borrow().is_none(), "reload");

    dirty(&mut app);
    app.commit_theme_picker_with(|_| Ok(()));
    assert!(app.session_snapshot.borrow().is_none(), "commit");

    // ── rollback ─────────────────────────────────────────────
    app.open_theme_picker();
    app.preview_theme("default");
    dirty(&mut app);
    app.cancel_theme_picker();
    assert!(app.session_snapshot.borrow().is_none(), "rollback");

    // ── replacing the manager wholesale ──────────────────────
    let root = themes_dir_with(&[("mine", &user_theme("Mine"))]);
    let mut app = app_wanting("mine");
    dirty(&mut app);
    app.load_themes_from(&themes_path(&root));
    assert_eq!(app.active_theme_id(), "mine");
    assert!(app.session_snapshot.borrow().is_none(), "load_themes_from");

    // Even the degraded path — an unreadable directory falling back to the
    // built-ins — is a manager replacement, and invalidates too.
    let broken = tempfile::tempdir().unwrap();
    let not_a_dir = broken.path().join("themes");
    fs::write(&not_a_dir, "not a directory").unwrap();
    dirty(&mut app);
    app.load_themes_from(&not_a_dir);
    assert!(
        app.session_snapshot.borrow().is_none(),
        "load_themes_from (degraded)"
    );
}

#[test]
fn the_theme_accessors_report_what_the_manager_holds() {
    // With the field private these read-only accessors are the whole outside
    // view of the theme state, so they have to keep agreeing with it.
    let root = themes_dir_with(&[("mine", &user_theme("Mine"))]);
    let mut app = app_wanting("mine");
    app.load_themes_from(&themes_path(&root));

    assert_eq!(app.active_theme_id(), "mine");
    assert_eq!(app.saved_theme_id(), "mine");
    assert_eq!(app.theme().id().as_str(), "mine");
    assert!(app.theme_registry().get("default").is_some());
    assert_eq!(app.themes_dir(), Some(themes_path(&root).as_path()));

    // A preview moves `active` and leaves `saved` where it was — the
    // distinction the picker is built on.
    app.open_theme_picker();
    app.preview_theme("fire");
    assert_eq!(app.active_theme_id(), "fire");
    assert_eq!(app.saved_theme_id(), "mine");
    assert_eq!(app.theme().id().as_str(), "fire");
}

// ── Reload identity, preview freshness, and the footer ──────────
//
// A reload swaps the whole registry. Two things that look like the same row —
// and two `Rc`s that spell the same id — are the traps that lets a reload
// silently change what the user is looking at.

/// A valid user theme whose accent is `accent`, so a repaired file is
/// distinguishable from the version it replaced by a value, not by an id.
fn theme_with_accent(name: &str, accent: &str) -> String {
    format!(
        "schema_version = 1\nname = \"{name}\"\nextends = \"default\"\n\n\
         [semantic]\naccent = \"{accent}\"\n"
    )
}

/// Index of the *user* row for `id` — the file, never the built-in of that
/// name. The whole point of these tests is that the two are different rows.
fn user_row_index(app: &App, id: &str) -> usize {
    app.theme_picker_rows()
        .iter()
        .position(|row| row.id == id && !row.builtin)
        .unwrap_or_else(|| panic!("no user row for `{id}`"))
}

#[test]
fn reload_keeps_the_selection_on_a_reserved_id_squatter() {
    // Two rows spell `aqua`: the canonical built-in and the user's file, which
    // is invalid precisely because it squats a reserved id. Matching rows by id
    // across a reload moved the selection onto the built-in, and the next Enter
    // would then have saved a theme the user never chose.
    let mut app = app_with_files(&[("aqua", &user_theme("Not Really Aqua"))]);
    app.open_theme_picker();
    let squatter = user_row_index(&app, "aqua");
    let builtin = app
        .theme_picker_rows()
        .iter()
        .position(|row| row.id == "aqua" && row.builtin)
        .expect("the built-in aqua is listed");
    assert_ne!(squatter, builtin, "the two aqua rows must be distinct");

    app.select_theme_row(squatter);
    let live = app.theme_manager.active_rc();
    assert_eq!(
        app.theme_manager.active_id(),
        "default",
        "an invalid row never previews"
    );

    app.reload_theme_picker();

    let state = app.theme_picker.as_ref().unwrap();
    assert_eq!(state.selected, squatter, "the selection jumped rows");
    assert_eq!(state.tombstone, None, "nothing disappeared");
    let selected = &app.theme_picker_rows()[state.selected];
    assert!(!selected.builtin, "the selection landed on the built-in");
    assert_eq!(selected.status, ThemeRowStatus::Invalid);
    // Value equality, not `Rc::ptr_eq`: a reload always re-adopts the preview
    // out of the new registry, so the pointer legitimately moves. What may not
    // move is the theme the user is looking at.
    assert_eq!(app.theme_manager.active_id(), "default");
    assert_eq!(
        *live,
        *app.theme_manager.active_rc(),
        "a reload of an unchanged directory must not change the live theme"
    );
}

#[test]
fn deleting_a_squatter_tombstones_the_user_slot_not_the_built_in() {
    let mut app = app_with_files(&[("aqua", &user_theme("Not Really Aqua"))]);
    app.open_theme_picker();
    let squatter = user_row_index(&app, "aqua");
    app.select_theme_row(squatter);

    app.remove_user_theme_and_reload("aqua");

    let state = app.theme_picker.as_ref().unwrap();
    let tombstone = state.tombstone.as_ref().expect("the file is gone");
    assert_eq!(tombstone.id, "aqua");
    assert!(!tombstone.builtin, "the built-in aqua did not disappear");
    assert_eq!(state.selected, tombstone.index);
    let rows = app.theme_picker_rows();
    assert!(
        !rows[state.selected].builtin,
        "the tombstone must sit in the user slot"
    );
    assert!(
        rows.iter()
            .any(|row| row.id == "aqua" && row.builtin && row.status == ThemeRowStatus::Valid),
        "the canonical built-in must still be listed and usable"
    );
}

#[test]
fn reload_readopts_the_preview_from_the_new_registry_while_an_invalid_row_is_selected() {
    // The freshness trap: `preview_id == active_id` is true both before and
    // after a repair, because an id says nothing about which registry the `Rc`
    // came from. Repairing the previewed file while the *selection* sits on an
    // unusable row therefore used to leave the old `Rc` painting.
    let mut app = app_with_files(&[
        ("ocean", &theme_with_accent("Ocean", "#010203")),
        ("broken", &invalid_theme("Broken")),
    ]);
    app.open_theme_picker();
    assert!(app.preview_theme("ocean"));
    let stale = app.theme_manager.active_rc();
    assert_eq!(stale.semantic().accent, ratatui::style::Color::Rgb(1, 2, 3));

    let broken = app.theme_row_index("broken");
    assert!(
        !app.select_theme_row(broken),
        "an invalid row must report that it did not activate"
    );
    assert_eq!(app.theme_picker.as_ref().unwrap().preview_id, "ocean");

    fs::write(
        app.test_themes_dir().join("ocean.toml"),
        theme_with_accent("Ocean", "#0a0b0c"),
    )
    .unwrap();
    app.reload_theme_picker();

    assert_eq!(app.theme_manager.active_id(), "ocean");
    assert_eq!(
        app.theme().semantic().accent,
        ratatui::style::Color::Rgb(0x0a, 0x0b, 0x0c),
        "the reload kept painting the theme from the discarded registry"
    );
    assert!(!Rc::ptr_eq(&stale, &app.theme_manager.active_rc()));
}

#[test]
fn a_reload_that_changes_nothing_still_leaves_a_usable_preview() {
    // The counterpart to the test above: re-adopting must not be able to drop
    // the preview when the file is untouched.
    let mut app = app_with_files(&[
        ("ocean", &theme_with_accent("Ocean", "#010203")),
        ("broken", &invalid_theme("Broken")),
    ]);
    app.open_theme_picker();
    assert!(app.preview_theme("ocean"));
    app.select_theme_row(app.theme_row_index("broken"));

    app.reload_theme_picker();

    assert_eq!(app.theme_manager.active_id(), "ocean");
    assert_eq!(
        app.theme().semantic().accent,
        ratatui::style::Color::Rgb(1, 2, 3)
    );
}

// ── The footer: description, diagnostics, scrolling ─────────────

#[test]
fn the_footer_shows_the_description_when_nothing_is_wrong() {
    // Spec, "Layout": the area under the list shows "Beschreibung oder
    // Validierungsfehler". A clean theme has no error, so it must show prose.
    let mut app = app_with_themes(["default"]);
    app.open_theme_picker();
    app.select_theme_row(app.theme_row_index("aqua"));

    let rows = app.theme_picker_rows();
    let description = rows[app.theme_row_index("aqua")]
        .description
        .clone()
        .expect("every built-in carries a description");
    assert!(description.contains("Deep-water"), "{description}");
    assert_eq!(app.theme_diagnostic_lines(), vec![description]);
}

#[test]
fn a_diagnostic_outranks_the_description() {
    let mut app = app_with_files(&[("warned", &warning_theme("Warned"))]);
    app.open_theme_picker();
    app.select_theme_row(app.theme_row_index("warned"));

    let lines = app.theme_diagnostic_lines();
    assert!(
        lines[0].contains("not_a_real_section_v2"),
        "the diagnostic must come first: {lines:?}"
    );
    let description = app.theme_picker_rows()[app.theme_row_index("warned")]
        .description
        .clone();
    assert_eq!(description, None, "this fixture carries no description");
    assert_eq!(lines.len(), 1);
}

#[test]
fn a_picker_error_outranks_both_the_diagnostics_and_the_description() {
    let mut app = app_with_files(&[("warned", &warning_theme("Warned"))]);
    app.open_theme_picker();
    app.select_theme_row(app.theme_row_index("warned"));
    // A save failure is the one picker error reachable without a filesystem
    // trick, and it leaves the preview alone.
    app.commit_theme_picker_with(|_| Err(anyhow::anyhow!("disk on fire")));

    let lines = app.theme_diagnostic_lines();
    assert_eq!(lines[0], "disk on fire");
    assert!(lines[1].contains("not_a_real_section_v2"), "{lines:?}");
}

/// A theme with four unknown component roles — more diagnostics than the
/// two-row footer can hold at once.
fn four_unknown_roles(name: &str) -> String {
    format!(
        "schema_version = 1\nname = \"{name}\"\nextends = \"default\"\n\n\
         [components.footer]\nglow = \"semantic.accent\"\nhalo = \"semantic.accent\"\n\
         shine = \"semantic.accent\"\nsparkle = \"semantic.accent\"\n"
    )
}

#[test]
fn the_diagnostics_footer_scrolls_to_every_ignored_role() {
    // Spec: a warning theme shows *all* ignored roles. The footer is two rows
    // tall, so four of them can only be reached by scrolling.
    let mut app = app_with_files(&[("noisy", &four_unknown_roles("Noisy"))]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
    app.open_theme_picker();
    app.select_theme_row(app.theme_row_index("noisy"));

    let lines = app.theme_diagnostic_lines();
    assert_eq!(lines.len(), 4, "{lines:?}");
    for role in ["glow", "halo", "shine", "sparkle"] {
        assert!(
            lines.iter().any(|line| line.contains(role)),
            "`{role}` is not among the diagnostics: {lines:?}"
        );
    }

    let visible = crate::tui::screens::theme_picker::diagnostic_rows(app.terminal_area);
    assert_eq!(visible, 2, "the footer holds two rows at 100x30");
    assert_eq!(app.theme_picker.as_ref().unwrap().diagnostics_scroll, 0);

    app.scroll_theme_diagnostics(1);
    assert_eq!(app.theme_picker.as_ref().unwrap().diagnostics_scroll, 1);
    app.scroll_theme_diagnostics(5);
    assert_eq!(
        app.theme_picker.as_ref().unwrap().diagnostics_scroll,
        lines.len() - visible,
        "scrolling must stop at the last line rather than run past it"
    );
    app.scroll_theme_diagnostics(-99);
    assert_eq!(app.theme_picker.as_ref().unwrap().diagnostics_scroll, 0);
}

#[test]
fn moving_the_selection_puts_the_diagnostics_footer_back_at_the_top() {
    let mut app = app_with_files(&[("noisy", &four_unknown_roles("Noisy"))]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
    app.open_theme_picker();
    app.select_theme_row(app.theme_row_index("noisy"));
    app.scroll_theme_diagnostics(2);
    assert_ne!(app.theme_picker.as_ref().unwrap().diagnostics_scroll, 0);

    app.select_theme_row(0);
    assert_eq!(
        app.theme_picker.as_ref().unwrap().diagnostics_scroll,
        0,
        "another row's diagnostics must start at their own first line"
    );
}

#[test]
fn shift_arrows_are_wired_to_the_diagnostics_scroll() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = app_with_files(&[("noisy", &four_unknown_roles("Noisy"))]);
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 30);
    app.open_theme_picker();
    let noisy = app.theme_row_index("noisy");
    app.select_theme_row(noisy);

    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT))
        .unwrap();
    let state = app.theme_picker.as_ref().unwrap();
    assert_eq!(state.diagnostics_scroll, 1);
    assert_eq!(state.selected, noisy, "Shift+Down must not move the list");

    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT))
        .unwrap();
    assert_eq!(app.theme_picker.as_ref().unwrap().diagnostics_scroll, 0);

    // The unshifted arrow still navigates, so the new binding cannot have
    // swallowed the old one.
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    assert_ne!(app.theme_picker.as_ref().unwrap().selected, noisy);
}
