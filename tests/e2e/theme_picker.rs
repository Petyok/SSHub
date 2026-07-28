//! The theme picker as a user drives it: Settings → picker → preview →
//! rollback or save.
//!
//! Everything here goes through `App::handle_key` and the public API, and every
//! theme file lives in a `tempfile::tempdir()`. Nothing touches the real HOME,
//! the real SSHub config, a database or the keyring — the one write a commit
//! would do is captured through `commit_theme_picker_with` instead of being
//! sent to a config directory.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

use sshub::app::{App, AppDeps, AppMode, ThemeRowStatus};
use sshub::config::AppConfig;
use sshub::metadata::MetadataDb;
use sshub::ssh::{HostResolver, SshHost};
use sshub::store::LauncherStore;

/// A valid user theme with a recognisable accent, so "which theme is live" can
/// be answered by a value rather than by a name.
fn theme_with_accent(name: &str, accent: &str) -> String {
    format!(
        "schema_version = 1\nname = \"{name}\"\nextends = \"default\"\n\n\
         [semantic]\naccent = \"{accent}\"\n"
    )
}

/// Valid, but with an unknown component role: `warning` at runtime — savable,
/// and its ignored role has to be visible.
fn warning_theme(name: &str) -> String {
    format!(
        "schema_version = 1\nname = \"{name}\"\nextends = \"default\"\n\n\
         [components.footer]\nglow = \"semantic.accent\"\n"
    )
}

/// Fatally invalid in both validation modes.
fn invalid_theme(name: &str) -> String {
    format!("schema_version = 99\nname = \"{name}\"\n")
}

struct NoHosts;

impl HostResolver for NoHosts {
    fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }
    fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
        Ok(SshHost::new(name))
    }
}

/// A tempdir-backed SSHub with a `themes/` directory, driven by keys only.
struct ThemeEnv {
    root: tempfile::TempDir,
    app: App,
}

impl ThemeEnv {
    /// An app whose themes directory holds exactly `files`, already loaded.
    fn new(files: &[(&str, String)]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let themes = root.path().join("themes");
        std::fs::create_dir(&themes).unwrap();
        for (id, body) in files {
            std::fs::write(themes.join(format!("{id}.toml")), body).unwrap();
        }
        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(NoHosts),
                metadata: Arc::new(MetadataDb::default()),
                store: Arc::new(LauncherStore::open_in_memory().unwrap()),
                password_store: Box::new(sshub::credentials::NoopPasswordStore),
            },
        );
        app.terminal_area = Rect::new(0, 0, 100, 30);
        app.load_themes_from(&themes);
        Self { root, app }
    }

    fn themes_dir(&self) -> PathBuf {
        self.root.path().join("themes")
    }

    fn theme_path(&self, id: &str) -> PathBuf {
        self.themes_dir().join(format!("{id}.toml"))
    }

    fn press(&mut self, code: KeyCode) {
        self.app
            .handle_key(KeyEvent::new(code, KeyModifiers::NONE))
            .unwrap();
    }

    fn press_char(&mut self, c: char) {
        self.press(KeyCode::Char(c));
    }

    /// Ctrl+H → the `Theme…` action row → Enter, exactly as a user reaches it.
    fn open_picker(&mut self) {
        self.app
            .handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(self.app.mode, AppMode::Settings, "Ctrl+H opens Settings");
        // The theme action is the first Settings row (spec: above the toggles).
        self.app.settings_selected = 0;
        self.press(KeyCode::Enter);
        assert_eq!(self.app.mode, AppMode::ThemePicker);
    }

    /// Walk the list to `id` with the arrow keys, so navigation is what moves
    /// the selection rather than a direct index poke. Walks the short way, so
    /// the rows it previews on the way are only the ones in between.
    fn select(&mut self, id: &str) {
        let rows = self.app.theme_picker_rows();
        let target = rows
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("`{id}` is not listed"));
        let mut guard = 0;
        while self.selected_index() != target {
            let key = if self.selected_index() > target {
                KeyCode::Up
            } else {
                KeyCode::Down
            };
            self.press(key);
            guard += 1;
            assert!(guard <= rows.len() * 2, "could not reach `{id}` by walking");
        }
    }

    fn selected_index(&self) -> usize {
        self.app
            .theme_picker
            .as_ref()
            .expect("the picker is open")
            .selected
    }

    fn selected_id(&self) -> String {
        self.app.theme_picker_rows()[self.selected_index()]
            .id
            .clone()
    }

    fn status_of(&self, id: &str) -> ThemeRowStatus {
        self.app
            .theme_picker_rows()
            .iter()
            .find(|row| row.id == id)
            .unwrap_or_else(|| panic!("`{id}` is not listed"))
            .status
    }

    fn accent(&self) -> ratatui::style::Color {
        self.app
            .theme()
            .semantic()
            .slot(sshub::theme::catalog::SemanticSlot::Accent)
    }

    /// `Enter` with the write captured instead of sent to a config directory.
    /// Returns every config a commit tried to persist.
    fn commit_capturing(&mut self) -> Vec<AppConfig> {
        let written = RefCell::new(Vec::new());
        self.app.commit_theme_picker_with(|config| {
            written.borrow_mut().push(config.clone());
            Ok(())
        });
        written.into_inner()
    }

    fn draw(&self, width: u16, height: u16) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| sshub::tui::render(frame, &self.app))
            .unwrap();
    }
}

fn rgb(r: u8, g: u8, b: u8) -> ratatui::style::Color {
    ratatui::style::Color::Rgb(r, g, b)
}

/// A fingerprint of every file under `root`, so "nothing was written" can be
/// asserted rather than assumed.
fn fingerprint(root: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root).unwrap().flatten() {
        let meta = entry.metadata().unwrap();
        out.push((entry.file_name().to_string_lossy().into_owned(), meta.len()));
        if meta.is_dir() {
            out.extend(fingerprint(&entry.path()));
        }
    }
    out.sort();
    out
}

/// The headline workflow: preview, roll back with `Esc`, come back and save.
#[test]
fn theme_picker_previews_rolls_back_and_persists() {
    let mut env = ThemeEnv::new(&[("custom", theme_with_accent("Custom", "#ff00ff"))]);
    assert_eq!(env.app.active_theme_id(), "default");
    let default_accent = env.accent();

    env.open_picker();
    env.select("custom");
    assert_eq!(env.app.active_theme_id(), "custom");
    assert_eq!(env.accent(), rgb(0xff, 0x00, 0xff), "the preview is live");
    assert_eq!(
        env.app.saved_theme_id(),
        "default",
        "a preview must not move the saved theme"
    );

    env.press(KeyCode::Esc);
    assert_eq!(env.app.mode, AppMode::Settings);
    assert_eq!(env.app.active_theme_id(), "default");
    assert_eq!(env.accent(), default_accent, "Esc must roll the theme back");

    env.open_picker();
    env.select("custom");
    let written = env.commit_capturing();
    assert_eq!(written.len(), 1, "a commit writes exactly once");
    assert_eq!(written[0].appearance.active_theme, "custom");
    assert_eq!(env.app.saved_theme_id(), "custom");
    assert_eq!(env.app.mode, AppMode::Settings, "a save closes the picker");
    assert!(env.app.theme_picker.is_none());
}

/// A theme with an unknown component role is usable: it previews, it saves, and
/// the role it ignored is named in the picker's diagnostics.
#[test]
fn a_warning_theme_previews_saves_and_names_the_ignored_role() {
    let mut env = ThemeEnv::new(&[("warned", warning_theme("Warned"))]);
    env.open_picker();
    env.select("warned");

    assert_eq!(env.status_of("warned"), ThemeRowStatus::Warning);
    assert_eq!(
        env.app.active_theme_id(),
        "warned",
        "a warning still previews"
    );
    let lines = env.app.theme_diagnostic_lines();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("components.footer.glow")),
        "the ignored role must be named: {lines:?}"
    );

    let written = env.commit_capturing();
    assert_eq!(written.len(), 1, "a warning theme is savable");
    assert_eq!(written[0].appearance.active_theme, "warned");
    assert_eq!(env.app.saved_theme_id(), "warned");
}

/// An invalid theme is listed with its reason, never previewed, and `Enter`
/// refuses it without writing anything.
#[test]
fn an_invalid_theme_is_listed_but_never_previewed_or_saved() {
    let mut env = ThemeEnv::new(&[
        ("broken", invalid_theme("Broken")),
        ("good", theme_with_accent("Good", "#00ff00")),
    ]);
    env.open_picker();
    env.select("good");
    assert_eq!(env.app.active_theme_id(), "good");

    // `broken` sorts directly above `good`, so this is a single `Up`: the walk
    // previews nothing in between and the live theme can only have been changed
    // by the invalid row itself.
    env.select("broken");
    assert_eq!(env.status_of("broken"), ThemeRowStatus::Invalid);
    assert_eq!(
        env.selected_id(),
        "broken",
        "an invalid row is still selectable — its reason is why it is listed"
    );
    assert_eq!(
        env.app.active_theme_id(),
        "good",
        "an invalid row must never become the live theme"
    );
    assert!(
        !env.app.theme_diagnostic_lines().is_empty(),
        "the picker must be able to say why it is unusable"
    );

    let written = env.commit_capturing();
    assert!(
        written.is_empty(),
        "an invalid theme must never be persisted"
    );
    assert_eq!(env.app.mode, AppMode::ThemePicker, "the picker stays open");
    assert_eq!(env.app.saved_theme_id(), "default");
    assert!(
        env.app
            .theme_picker
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .is_some_and(|e| e.contains("broken")),
        "the refusal has to say which theme it refused"
    );
}

/// `r` after repairing the file on disk: the row turns valid and becomes the
/// live preview immediately, and nothing was written on the way there.
#[test]
fn reload_after_repair_adopts_the_fixed_file() {
    let mut env = ThemeEnv::new(&[("mine", invalid_theme("Mine"))]);
    env.open_picker();
    env.select("mine");
    assert_eq!(env.status_of("mine"), ThemeRowStatus::Invalid);
    // Navigating past the built-ins previews each of them, so what matters is
    // that the invalid row itself never became live.
    let live_before = env.app.active_theme_id().to_string();
    assert_ne!(live_before, "mine", "an invalid row must never preview");

    let before = fingerprint(env.root.path());
    std::fs::write(
        env.theme_path("mine"),
        theme_with_accent("Repaired", "#0a0b0c"),
    )
    .unwrap();
    env.press_char('r');

    assert_eq!(env.status_of("mine"), ThemeRowStatus::Valid);
    assert_eq!(env.selected_id(), "mine", "the selection stays on the file");
    assert_eq!(env.app.active_theme_id(), "mine");
    assert_eq!(
        env.accent(),
        rgb(0x0a, 0x0b, 0x0c),
        "the repaired file has to be the one painting"
    );
    assert_eq!(
        env.app.saved_theme_id(),
        "default",
        "a reload saves nothing"
    );

    // The only file that changed is the one the test rewrote itself.
    let after = fingerprint(env.root.path());
    assert_eq!(before.len(), after.len(), "a reload must not create files");
}

/// Deleting the previewed file and reloading leaves the last valid theme
/// painting and keeps the selection in the slot the file used to have.
#[test]
fn reload_after_deleting_the_preview_keeps_the_theme() {
    let mut env = ThemeEnv::new(&[("gone", theme_with_accent("Gone", "#123456"))]);
    env.open_picker();
    env.select("gone");
    assert_eq!(env.accent(), rgb(0x12, 0x34, 0x56));
    let slot = env.selected_index();

    std::fs::remove_file(env.theme_path("gone")).unwrap();
    env.press_char('r');

    assert_eq!(
        env.accent(),
        rgb(0x12, 0x34, 0x56),
        "the last valid theme keeps painting"
    );
    assert_eq!(env.selected_index(), slot, "the selection keeps its slot");
    assert_eq!(env.status_of("gone"), ThemeRowStatus::Invalid);
    assert!(
        env.app
            .theme_diagnostic_lines()
            .iter()
            .any(|line| line.contains("no longer installed")),
        "the vanished row has to explain itself"
    );
}

/// The picker has to survive terminals far below its own minimum, with the
/// list still usable, and it must not write outside the frame.
#[test]
fn the_picker_survives_a_tiny_terminal() {
    let mut env = ThemeEnv::new(&[("custom", theme_with_accent("Custom", "#ff00ff"))]);
    env.open_picker();
    env.select("custom");

    for (w, h) in [(1, 1), (20, 5), (40, 10), (80, 24)] {
        env.app.terminal_area = Rect::new(0, 0, w, h);
        env.draw(w, h);
        // Navigation must keep working at every size: this is the "die Liste
        // bleibt bedienbar" rule, driven through the real key handler.
        env.press(KeyCode::Down);
        env.press(KeyCode::Up);
        assert_eq!(env.selected_id(), "custom", "{w}x{h}: navigation broke");
        assert_eq!(env.app.mode, AppMode::ThemePicker, "{w}x{h}");
    }
}

/// Opening, navigating, reloading and cancelling write nothing at all.
#[test]
fn browsing_the_picker_never_touches_the_disk() {
    let mut env = ThemeEnv::new(&[
        ("one", theme_with_accent("One", "#111111")),
        ("two", theme_with_accent("Two", "#222222")),
    ]);
    let before = fingerprint(env.root.path());

    env.open_picker();
    env.select("two");
    env.press_char('r');
    env.select("one");
    env.press(KeyCode::Esc);

    assert_eq!(
        before,
        fingerprint(env.root.path()),
        "opening, navigating, reloading and cancelling must not touch the disk"
    );
    assert!(!env.root.path().join("config.toml").exists());
    assert_eq!(env.app.saved_theme_id(), "default");
    assert_eq!(env.app.mode, AppMode::Settings);
}
