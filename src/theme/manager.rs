//! Runtime ownership of the active theme.
//!
//! [`ThemeManager`] is the one place that turns "what `config.toml` asked for"
//! into "what the runtime actually paints with". It deliberately does **no**
//! config I/O: the registry does not either, so the fallback policy — an
//! invalid or missing `active_theme` degrades to `default`, keeps the
//! configured id, and never rewrites `config.toml` — lives here alone.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::theme::model::{ResolvedTheme, ThemeDiagnostic, ThemeId, ThemeOrigin, ValidationMode};
use crate::theme::registry::ThemeRegistry;

/// The id every fallback lands on. Embedded in the binary, so it is reachable
/// without any filesystem access (spec, "Fehler- und Fallback-Verhalten").
pub const DEFAULT_THEME_ID: &str = "default";

/// The active [`ResolvedTheme`] plus the registry it came from.
///
/// Owned by `App`; there is no global mutable theme state. Handles are handed
/// out as `Rc<ResolvedTheme>` clones because a resolved theme is immutable.
pub struct ThemeManager {
    registry: ThemeRegistry,
    /// What `config.toml` asks for. Preserved verbatim even while the fallback
    /// is active, so repairing the theme file restores the user's choice
    /// without them having to re-pick it.
    saved_id: String,
    /// What is actually painting right now.
    active_id: String,
    active: Rc<ResolvedTheme>,
    themes_dir: PathBuf,
    startup_diagnostics: Vec<ThemeDiagnostic>,
}

impl ThemeManager {
    /// Built-ins only — no filesystem access at all.
    ///
    /// This is what `App::new_with_deps` hands tests, and the degraded state
    /// when there is no usable config directory.
    pub fn builtins(saved_id: impl Into<String>) -> Self {
        let registry = ThemeRegistry::builtins(ValidationMode::Compatible)
            // The embedded assets are a build invariant, proven by
            // `theme::builtins::tests` — `builtins()` performs no I/O and has
            // no failure mode left at runtime.
            .expect("built-in themes must always load");
        Self::from_registry(registry, PathBuf::new(), saved_id)
    }

    /// Activate `saved_id` against `registry`, falling back to `default`.
    ///
    /// Never fails: a theme that is unknown or did not resolve produces a
    /// non-fatal start-up diagnostic and `default` becomes active.
    pub fn from_registry(
        registry: ThemeRegistry,
        themes_dir: PathBuf,
        saved_id: impl Into<String>,
    ) -> Self {
        let saved_id = saved_id.into();
        let mut startup_diagnostics = Vec::new();

        // Both lists are surfaced on purpose. The directory list carries the
        // warnings that explain a theme *missing* from the picker (an unusable
        // file name, an unreadable path, the 256-file cut); filtering it to
        // errors would hide exactly the ones a user needs.
        startup_diagnostics.extend(registry.diagnostics().iter().cloned());

        let (active_id, active) = match Self::lookup(&registry, &saved_id) {
            Some(theme) => {
                // A compatible-mode warning (e.g. an unknown component role
                // from a newer SSHub) keeps the requested theme active; only
                // the notice channel reports it.
                if let Some(record) = registry.get(&saved_id) {
                    startup_diagnostics.extend(record.diagnostics.iter().cloned());
                }
                (saved_id.clone(), theme)
            }
            None => {
                startup_diagnostics.push(Self::fallback_diagnostic(&registry, &saved_id));
                // The record's own diagnostics say *why* it is unusable.
                if let Some(record) = registry.get(&saved_id) {
                    startup_diagnostics.extend(record.diagnostics.iter().cloned());
                }
                let theme = Self::lookup(&registry, DEFAULT_THEME_ID)
                    .expect("the built-in `default` theme must always resolve");
                (DEFAULT_THEME_ID.to_string(), theme)
            }
        };

        Self {
            registry,
            saved_id,
            active_id,
            active,
            themes_dir,
            startup_diagnostics,
        }
    }

    /// The theme every renderer paints with.
    pub fn theme(&self) -> &ResolvedTheme {
        &self.active
    }

    /// A cheap clone of the active theme, for code that must outlive the borrow
    /// (preview snapshots, rollback).
    pub fn active_rc(&self) -> Rc<ResolvedTheme> {
        Rc::clone(&self.active)
    }

    pub fn active_id(&self) -> &str {
        &self.active_id
    }

    /// What `config.toml` holds — not necessarily what is active.
    pub fn saved_id(&self) -> &str {
        &self.saved_id
    }

    pub fn registry(&self) -> &ThemeRegistry {
        &self.registry
    }

    /// Where user themes live. Empty for a built-ins-only manager.
    pub fn themes_dir(&self) -> &Path {
        &self.themes_dir
    }

    /// Activate `id` if it resolves. Returns `false` and changes **nothing**
    /// when it does not, so a failed preview or reload can never leave a
    /// partial theme on screen.
    pub fn activate(&mut self, id: &str) -> bool {
        match Self::lookup(&self.registry, id) {
            Some(theme) => {
                self.activate_resolved(id.to_string(), theme);
                true
            }
            None => false,
        }
    }

    /// Activate an already-resolved theme.
    ///
    /// The single mutation point every preview / reload / rollback / commit
    /// runs through, so callers that must invalidate buffer snapshots have one
    /// place to hook.
    pub fn activate_resolved(&mut self, id: String, theme: Rc<ResolvedTheme>) {
        self.active_id = id;
        self.active = theme;
    }

    /// Put back a snapshot taken before a preview. Same mechanics as
    /// [`Self::activate_resolved`]; named separately so the call sites read as
    /// what they are.
    pub fn restore_snapshot(&mut self, id: String, theme: Rc<ResolvedTheme>) {
        self.activate_resolved(id, theme);
    }

    /// Swap in a freshly loaded registry (a reload). The active theme is left
    /// alone — re-activating is the caller's decision, and doing it here would
    /// silently drop a live preview.
    pub fn replace_registry(&mut self, registry: ThemeRegistry) {
        self.registry = registry;
    }

    /// Record that `config.toml` now holds `id`.
    pub fn mark_saved(&mut self, id: String) {
        self.saved_id = id;
    }

    /// Registry-level and active-record diagnostics collected at construction,
    /// for the non-fatal start-up notice. Never a reason to abort start-up.
    pub fn startup_diagnostics(&self) -> &[ThemeDiagnostic] {
        &self.startup_diagnostics
    }

    fn lookup(registry: &ThemeRegistry, id: &str) -> Option<Rc<ResolvedTheme>> {
        let id = ThemeId::parse(id).ok()?;
        registry.resolved(&id)
    }

    fn fallback_diagnostic(registry: &ThemeRegistry, saved_id: &str) -> ThemeDiagnostic {
        let known = registry.get(saved_id).is_some();
        let message = if known {
            format!("theme `{saved_id}` could not be used, so `{DEFAULT_THEME_ID}` is active")
        } else {
            format!("theme `{saved_id}` was not found, so `{DEFAULT_THEME_ID}` is active")
        };
        ThemeDiagnostic::error(ThemeOrigin::BuiltIn, None, message).with_help(format!(
            "`appearance.active_theme` still reads `{saved_id}`; config.toml was not changed, so \
             fixing the theme is enough to get it back"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// A themes directory holding exactly the given `<id>.toml` files.
    fn themes_dir_with(files: &[(&str, &str)]) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (id, body) in files {
            fs::write(dir.path().join(format!("{id}.toml")), body).unwrap();
        }
        dir
    }

    /// A syntactically valid user theme that extends `default`.
    fn valid_theme(name: &str) -> String {
        format!(
            "schema_version = 1\nname = \"{name}\"\nextends = \"default\"\n\n\
             [semantic]\naccent = \"#ff00ff\"\n"
        )
    }

    fn installed(dir: &Path) -> ThemeRegistry {
        ThemeRegistry::load_installed(dir, ValidationMode::Compatible).unwrap()
    }

    /// A registry in which `id` exists as a user file but is unusable.
    fn registry_with_invalid(id: &str) -> (TempDir, ThemeRegistry) {
        // An unknown schema version is fatal in both modes, so this is invalid
        // regardless of the validation mode.
        let dir = themes_dir_with(&[(id, "schema_version = 99\nname = \"Broken\"\n")]);
        let registry = installed(dir.path());
        (dir, registry)
    }

    fn manager_with_user_themes() -> (TempDir, ThemeManager) {
        let dir = themes_dir_with(&[("mine", &valid_theme("Mine"))]);
        let registry = installed(dir.path());
        let manager = ThemeManager::from_registry(registry, dir.path().to_path_buf(), "mine");
        (dir, manager)
    }

    #[test]
    fn invalid_saved_theme_falls_back_without_rewriting_config() {
        let (dir, registry) = registry_with_invalid("broken");
        let manager = ThemeManager::from_registry(registry, dir.path().to_path_buf(), "broken");
        assert_eq!(manager.active_id(), "default");
        assert_eq!(manager.saved_id(), "broken");
        assert!(manager
            .startup_diagnostics()
            .iter()
            .any(ThemeDiagnostic::is_error));
        // The registry does no config I/O and neither do we: nothing outside
        // the themes directory may be touched, and the broken file survives.
        assert!(dir.path().join("broken.toml").exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_activation_keeps_the_previous_rc() {
        let (_dir, mut manager) = manager_with_user_themes();
        let before = manager.active_rc();
        assert!(!manager.activate("invalid"));
        assert!(Rc::ptr_eq(&before, &manager.active_rc()));
        assert_eq!(manager.active_id(), "mine");
    }

    #[test]
    fn a_missing_theme_falls_back_and_keeps_the_configured_id() {
        let manager = ThemeManager::builtins("never-installed");
        assert_eq!(manager.active_id(), "default");
        assert_eq!(manager.saved_id(), "never-installed");
        let hint = manager
            .startup_diagnostics()
            .iter()
            .find(|d| d.is_error())
            .expect("a missing theme must be reported");
        assert!(
            hint.message.contains("never-installed"),
            "the hint must name the configured id: {}",
            hint.message
        );
        assert!(
            hint.help
                .as_deref()
                .is_some_and(|help| help.contains("config.toml was not changed")),
            "the hint must say config.toml is untouched: {hint:?}"
        );
    }

    #[test]
    fn a_valid_saved_theme_stays_active() {
        let (_dir, manager) = manager_with_user_themes();
        assert_eq!(manager.active_id(), "mine");
        assert_eq!(manager.saved_id(), "mine");
        assert!(
            !manager
                .startup_diagnostics()
                .iter()
                .any(ThemeDiagnostic::is_error),
            "a clean user theme must not produce a start-up error"
        );
    }

    #[test]
    fn directory_warnings_are_surfaced_not_filtered_to_errors() {
        // A `*.toml` path that is a directory is a *warning*, and it is exactly
        // what explains a theme missing from the picker.
        let dir = themes_dir_with(&[("usable", &valid_theme("Usable"))]);
        fs::create_dir(dir.path().join("bundle.toml")).unwrap();
        let registry = installed(dir.path());
        let manager = ThemeManager::from_registry(registry, dir.path().to_path_buf(), "usable");

        assert_eq!(manager.active_id(), "usable");
        assert!(
            manager
                .startup_diagnostics()
                .iter()
                .any(|d| d.is_warning() && d.message.contains("not a readable theme file")),
            "directory-level warnings must reach the notice: {:?}",
            manager.startup_diagnostics()
        );
    }

    #[test]
    fn an_active_theme_with_warnings_stays_active_and_reports_them() {
        // An unknown component role is downgraded to a warning in Compatible
        // mode, so the theme must still be the one painting.
        let body = format!(
            "{}\n[components.not_a_real_section_v2]\nborder = \"semantic.accent\"\n",
            valid_theme("Futuristic")
        );
        let dir = themes_dir_with(&[("futuristic", &body)]);
        let registry = installed(dir.path());
        let manager = ThemeManager::from_registry(registry, dir.path().to_path_buf(), "futuristic");

        assert_eq!(
            manager.active_id(),
            "futuristic",
            "a compatible-mode warning must not deactivate the theme: {:?}",
            manager.startup_diagnostics()
        );
        assert!(
            manager
                .startup_diagnostics()
                .iter()
                .any(ThemeDiagnostic::is_warning),
            "the compatibility warning must be surfaced: {:?}",
            manager.startup_diagnostics()
        );
    }

    #[test]
    fn activate_switches_the_theme_and_leaves_the_saved_id_alone() {
        let (_dir, mut manager) = manager_with_user_themes();
        assert!(manager.activate("aqua"));
        assert_eq!(manager.active_id(), "aqua");
        assert_eq!(
            manager.saved_id(),
            "mine",
            "activating must not pretend config.toml changed"
        );
        manager.mark_saved("aqua".to_string());
        assert_eq!(manager.saved_id(), "aqua");
    }

    #[test]
    fn restore_snapshot_puts_back_the_exact_rc() {
        let (_dir, mut manager) = manager_with_user_themes();
        let before_id = manager.active_id().to_string();
        let before = manager.active_rc();
        assert!(manager.activate("fire"));
        assert!(!Rc::ptr_eq(&before, &manager.active_rc()));
        manager.restore_snapshot(before_id.clone(), Rc::clone(&before));
        assert_eq!(manager.active_id(), before_id);
        assert!(Rc::ptr_eq(&before, &manager.active_rc()));
    }

    #[test]
    fn replace_registry_keeps_the_active_theme() {
        let (_dir, mut manager) = manager_with_user_themes();
        let before = manager.active_rc();
        manager.replace_registry(ThemeRegistry::builtins(ValidationMode::Compatible).unwrap());
        assert!(
            Rc::ptr_eq(&before, &manager.active_rc()),
            "a reload must not silently drop the live theme"
        );
        // The user theme is gone from the new registry, so re-activating it now
        // fails — and still leaves the live theme untouched.
        assert!(!manager.activate("mine"));
        assert!(Rc::ptr_eq(&before, &manager.active_rc()));
    }

    #[test]
    fn a_theme_id_that_is_not_even_a_valid_id_falls_back() {
        // `../../etc/passwd` cannot be a ThemeId at all; the manager must treat
        // it as any other missing theme rather than panicking or path-joining.
        let manager = ThemeManager::builtins("../../etc/passwd");
        assert_eq!(manager.active_id(), "default");
        assert_eq!(manager.saved_id(), "../../etc/passwd");
        assert!(manager
            .startup_diagnostics()
            .iter()
            .any(ThemeDiagnostic::is_error));
    }

    #[test]
    fn builtins_only_manager_touches_no_directory() {
        let manager = ThemeManager::builtins("default");
        assert_eq!(manager.active_id(), "default");
        assert_eq!(manager.themes_dir(), Path::new(""));
        assert!(manager.startup_diagnostics().is_empty());
        // All five embedded themes are reachable without any filesystem access.
        for id in ["default", "summer", "aqua", "fire", "high-contrast"] {
            assert!(
                manager.registry().get(id).is_some(),
                "built-in `{id}` missing"
            );
        }
    }
}
