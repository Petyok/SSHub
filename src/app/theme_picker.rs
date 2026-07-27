//! The theme picker state machine.
//!
//! The picker juggles three ids on purpose (spec, "Theme-Picker"):
//!
//! - `saved_id` — what `config.toml` holds; owned by [`ThemeManager`].
//! - `original_id` — the theme that was genuinely active when the picker opened.
//! - `preview_id` — what is temporarily painting the whole UI right now.
//!
//! Everything except a successful `Enter` is a pure in-memory transition:
//! opening, navigating, reloading and cancelling must never write `config.toml`
//! or any theme file. That is why persistence is injected into
//! [`App::commit_theme_picker_with`] rather than reached for directly — the one
//! code path that writes is the one path a test can count.

use std::path::PathBuf;
use std::rc::Rc;

use crate::app::{App, AppMode};
use crate::config::AppConfig;
use crate::theme::model::{ResolvedTheme, ThemeDiagnostic, ThemeId, ValidationMode};
use crate::theme::registry::{ThemeRegistry, ThemeSource, ThemeStatus};

/// Whether a row can be previewed and saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeRowStatus {
    /// Resolved cleanly.
    Valid,
    /// Resolved, but carries diagnostics — in Compatible mode an unknown
    /// component role from a newer SSHub. Previewable and savable.
    Warning,
    /// Listed so the reason is visible, but never activated.
    Invalid,
}

impl ThemeRowStatus {
    /// The word the list column shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Warning => "warning",
            Self::Invalid => "invalid",
        }
    }

    /// Whether a row with this status may become the live preview.
    pub fn is_activatable(self) -> bool {
        !matches!(self, Self::Invalid)
    }
}

/// One line of the picker list.
///
/// Rebuilt from the registry on demand rather than cached: the registry is the
/// single source of truth, and a cached list is exactly how a reload ends up
/// showing a theme that no longer exists.
#[derive(Clone, Debug)]
pub struct ThemeRow {
    pub id: String,
    /// Display name from the file; empty when the file could not be parsed.
    pub name: String,
    pub builtin: bool,
    pub status: ThemeRowStatus,
    /// Where the file lives; `None` for a built-in.
    pub path: Option<PathBuf>,
    pub diagnostics: Vec<ThemeDiagnostic>,
}

impl ThemeRow {
    /// What the list shows as the theme's name, falling back to the id for a
    /// file that never parsed far enough to have one.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}

/// A row that vanished from the registry during a reload.
///
/// Kept so the selection can stay in the slot the user left it in and still
/// explain what happened, instead of silently teleporting somewhere else
/// (spec: "die Auswahl springt auf den … entfernten Eintragsplatz").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeRecordSummary {
    pub id: String,
    pub name: String,
    pub builtin: bool,
    pub path: Option<PathBuf>,
    /// Where the row used to sit, so it can be put back in place.
    pub index: usize,
}

/// Everything the picker needs while it is open.
pub struct ThemePickerState {
    pub selected: usize,
    /// The theme that was active when the picker opened; what `Esc` returns to.
    pub original_id: String,
    /// The resolved theme captured at open time. `Esc` falls back to this `Rc`
    /// when `original_id` no longer resolves — the user's file may have been
    /// deleted while the picker was open, and dropping them onto `default`
    /// would be a change they never asked for.
    pub original_theme: Rc<ResolvedTheme>,
    /// The id currently painting the UI. Only ever an activatable theme.
    pub preview_id: String,
    /// A failed save or a failed reload. Rendered on the picker's own surface,
    /// never through `host_notice` (which the next keypress would clear).
    pub error: Option<String>,
    pub tombstone: Option<ThemeRecordSummary>,
}

impl App {
    /// Open the picker over the Settings overlay.
    ///
    /// Captures the active theme and nothing else: no activation, no I/O, no
    /// registry reload. Whatever is painting keeps painting.
    pub(crate) fn open_theme_picker(&mut self) {
        let original_id = self.theme_manager.active_id().to_string();
        let original_theme = self.theme_manager.active_rc();
        let rows = picker_rows(self.theme_manager.registry(), None);
        let selected = rows
            .iter()
            .position(|row| row.id == original_id && row.status.is_activatable())
            .unwrap_or(0);
        self.theme_picker = Some(ThemePickerState {
            selected,
            preview_id: original_id.clone(),
            original_id,
            original_theme,
            error: None,
            tombstone: None,
        });
        self.mode = AppMode::ThemePicker;
    }

    /// The list the picker renders and navigates: built-ins in their frozen
    /// order, then user themes, plus the tombstone of a row that disappeared.
    pub(crate) fn theme_picker_rows(&self) -> Vec<ThemeRow> {
        let tombstone = self
            .theme_picker
            .as_ref()
            .and_then(|state| state.tombstone.as_ref());
        picker_rows(self.theme_manager.registry(), tombstone)
    }

    /// Move the selection by `delta`, wrapping at both ends (`Up` / `Down`).
    pub(crate) fn move_theme_selection(&mut self, delta: isize) {
        let len = self.theme_picker_rows().len();
        if len == 0 {
            return;
        }
        let Some(state) = self.theme_picker.as_ref() else {
            return;
        };
        let next = (state.selected as isize + delta).rem_euclid(len as isize) as usize;
        self.select_theme_row(next);
    }

    /// Move the selection by `delta` without wrapping (`PageUp` / `PageDown`).
    ///
    /// A page that runs off the end stops there rather than reappearing at the
    /// other end: the spec makes only the arrow keys circular, and a wrapping
    /// page on a short list is indistinguishable from having done nothing.
    pub(crate) fn page_theme_selection(&mut self, delta: isize) {
        let len = self.theme_picker_rows().len();
        if len == 0 {
            return;
        }
        let Some(state) = self.theme_picker.as_ref() else {
            return;
        };
        let next = (state.selected as isize + delta).clamp(0, len as isize - 1) as usize;
        self.select_theme_row(next);
    }

    /// Select row `index` and preview it if it is activatable.
    ///
    /// An invalid row is still selectable — its diagnostics are the whole
    /// reason it is listed — but the live theme does not move.
    pub(crate) fn select_theme_row(&mut self, index: usize) {
        let rows = self.theme_picker_rows();
        let Some(row) = rows.get(index) else {
            return;
        };
        let Some(state) = self.theme_picker.as_mut() else {
            return;
        };
        state.selected = index;
        // A new navigation supersedes a stale save/reload error.
        state.error = None;
        if !row.status.is_activatable() {
            return;
        }
        let Ok(id) = ThemeId::parse(&row.id) else {
            return;
        };
        let Some(theme) = self.theme_manager.registry().resolved(&id) else {
            return;
        };
        self.activate_resolved_theme(theme);
        if let Some(state) = self.theme_picker.as_mut() {
            state.preview_id = row.id.clone();
        }
    }

    /// Select the row for `id` and preview it. Returns whether the row exists.
    ///
    /// Test-facing: the running picker only ever knows row *positions* (the
    /// keys move an index), so addressing a row by id exists purely so a test
    /// can say what it means instead of hard-coding a list offset.
    #[cfg(test)]
    pub(crate) fn preview_theme(&mut self, id: &str) -> bool {
        let Some(index) = self
            .theme_picker_rows()
            .iter()
            .position(|row| row.id == id && row.status.is_activatable())
        else {
            return false;
        };
        self.select_theme_row(index);
        true
    }

    /// Activate `id` outside the picker (start-up, tests). Returns whether it
    /// resolved; a failure changes nothing.
    pub(crate) fn activate_theme(&mut self, id: &str) -> bool {
        let Ok(parsed) = ThemeId::parse(id) else {
            return false;
        };
        let Some(theme) = self.theme_manager.registry().resolved(&parsed) else {
            return false;
        };
        self.activate_resolved_theme(theme);
        true
    }

    /// The one place the runtime theme changes.
    ///
    /// Preview, reload, rollback and commit all funnel through here so Task 12
    /// has a single seam for invalidating buffer snapshots and ending blit
    /// transitions. A second path would be a latent bug, not a shortcut.
    fn activate_resolved_theme(&mut self, theme: Rc<ResolvedTheme>) {
        self.theme_manager.activate_resolved(theme);
    }

    /// Re-read the themes directory (`r`).
    ///
    /// Deliberately explicit about what happens afterwards: `replace_registry`
    /// does not re-activate, so a repaired preview file is adopted here and a
    /// deleted one leaves the last valid runtime theme painting.
    pub(crate) fn reload_theme_picker(&mut self) {
        let Some(state) = self.theme_picker.as_ref() else {
            return;
        };
        let rows = self.theme_picker_rows();
        let previous = rows.get(state.selected).cloned();
        let previous_index = state.selected;

        // `None` is a manager that belongs to no directory at all (tests, or no
        // config directory). There is nothing to re-read — and above all the
        // working directory is not a fallback.
        let Some(dir) = self.theme_manager.themes_dir().map(|dir| dir.to_path_buf()) else {
            // Say so rather than doing nothing at all: a silent `r` is
            // indistinguishable from a broken key.
            self.set_theme_picker_error("no themes directory to reload");
            return;
        };

        let registry = match ThemeRegistry::load_installed(&dir, ValidationMode::Compatible) {
            Ok(registry) => registry,
            Err(e) => {
                // Never a partial state: the old registry, the old list and the
                // live theme all survive a reload that failed.
                self.set_theme_picker_error(format!("{} could not be read ({e})", dir.display()));
                return;
            }
        };
        self.theme_manager.replace_registry(registry);

        let fresh = picker_rows(self.theme_manager.registry(), None);
        let (selected, tombstone) = match previous {
            Some(previous) => match fresh.iter().position(|row| row.id == previous.id) {
                Some(index) => (index, None),
                None => {
                    let index = previous_index.min(fresh.len());
                    (
                        index,
                        Some(ThemeRecordSummary {
                            id: previous.id,
                            name: previous.name,
                            builtin: previous.builtin,
                            path: previous.path,
                            index,
                        }),
                    )
                }
            },
            None => (0, None),
        };

        if let Some(state) = self.theme_picker.as_mut() {
            state.selected = selected;
            state.tombstone = tombstone;
            state.error = None;
        }

        // The file the user was looking at is the one they were editing, so a
        // repaired selection becomes the live preview immediately.
        self.select_theme_row(selected);

        // A selection that is still unusable leaves the preview where it was —
        // but that file may have changed too, so re-resolve it. A preview that
        // is gone or broken keeps the last valid runtime theme painting.
        let preview_id = self
            .theme_picker
            .as_ref()
            .map(|state| state.preview_id.clone())
            .unwrap_or_default();
        if preview_id != self.theme_manager.active_id() {
            if let Ok(parsed) = ThemeId::parse(&preview_id) {
                if let Some(theme) = self.theme_manager.registry().resolved(&parsed) {
                    self.activate_resolved_theme(theme);
                }
            }
        }
    }

    /// `Esc`: put the original theme back and return to Settings.
    pub(crate) fn cancel_theme_picker(&mut self) {
        let Some(state) = self.theme_picker.take() else {
            self.mode = AppMode::Settings;
            return;
        };
        // Prefer the id — a reload may have picked up a newer version of the
        // very same theme — and fall back to the `Rc` captured at open time
        // when the file is gone.
        let restored = ThemeId::parse(&state.original_id)
            .ok()
            .and_then(|id| self.theme_manager.registry().resolved(&id))
            .unwrap_or(state.original_theme);
        self.activate_resolved_theme(restored);
        self.mode = AppMode::Settings;
    }

    /// `Enter`: persist the previewed theme through the real config writer.
    pub(crate) fn commit_theme_picker(&mut self) {
        self.commit_theme_picker_with(crate::config::save_config);
    }

    /// `Enter` with an injectable writer.
    ///
    /// The only path in the whole picker that persists anything, so a test can
    /// count writes instead of inspecting call sites. Order matters and is
    /// enforced by the signatures: activate, then write, then `mark_saved()` —
    /// which adopts `active_id` and therefore can only ever record a theme that
    /// was genuinely active.
    pub(crate) fn commit_theme_picker_with(
        &mut self,
        persist: impl FnOnce(&AppConfig) -> anyhow::Result<()>,
    ) {
        let Some(state) = self.theme_picker.as_ref() else {
            return;
        };
        let rows = self.theme_picker_rows();
        let Some(row) = rows.get(state.selected).cloned() else {
            return;
        };
        if !row.status.is_activatable() {
            self.set_theme_picker_error(format!("`{}` cannot be used", row.id));
            return;
        }
        // Activate before staging, so what is saved is by construction what the
        // user is looking at: `active_id` is derived from the theme itself.
        if !self.activate_theme(&row.id) {
            self.set_theme_picker_error(format!("`{}` could not be resolved", row.id));
            return;
        }
        if let Some(state) = self.theme_picker.as_mut() {
            state.preview_id = row.id.clone();
        }

        let staged = self.config_with_preview_theme();
        match persist(&staged) {
            Ok(()) => self.finish_theme_commit(staged),
            Err(error) => self.set_theme_picker_error(error),
        }
    }

    /// The config that a commit would write: today's config with
    /// `appearance.active_theme` set to the theme that is actually active.
    fn config_with_preview_theme(&self) -> AppConfig {
        let mut config = self.config.clone();
        config.appearance.active_theme = self.theme_manager.active_id().to_string();
        config
    }

    fn finish_theme_commit(&mut self, staged: AppConfig) {
        self.config = staged;
        self.theme_manager.mark_saved();
        self.theme_picker = None;
        self.mode = AppMode::Settings;
    }

    /// Keep the picker open with the preview intact and the reason on screen.
    fn set_theme_picker_error(&mut self, error: impl std::fmt::Display) {
        let error = error.to_string();
        if let Some(state) = self.theme_picker.as_mut() {
            state.error = Some(error);
        }
    }
}

/// Build the picker list from a registry.
///
/// Enumerates `records()`, never `get()`: `get()` answers with the *canonical*
/// record, so a user file squatting a reserved built-in id — precisely the file
/// its author needs explained — would be invisible. Built-ins come first in
/// their frozen registration order; user themes follow, sorted by display name
/// and then id (spec, "Layout").
pub(crate) fn picker_rows(
    registry: &ThemeRegistry,
    tombstone: Option<&ThemeRecordSummary>,
) -> Vec<ThemeRow> {
    let mut builtins = Vec::new();
    let mut user = Vec::new();
    for record in registry.records() {
        let builtin = matches!(record.source, ThemeSource::BuiltIn);
        let status = match record.status {
            ThemeStatus::Invalid => ThemeRowStatus::Invalid,
            ThemeStatus::Valid if record.diagnostics.is_empty() => ThemeRowStatus::Valid,
            ThemeStatus::Valid => ThemeRowStatus::Warning,
        };
        let row = ThemeRow {
            id: record.id.as_str().to_string(),
            name: record.name.clone(),
            builtin,
            status,
            path: match &record.source {
                ThemeSource::BuiltIn => None,
                ThemeSource::User(path) => Some(path.clone()),
            },
            diagnostics: record.diagnostics.clone(),
        };
        if builtin {
            builtins.push(row);
        } else {
            user.push(row);
        }
    }
    user.sort_by(|a, b| {
        a.display_name()
            .to_lowercase()
            .cmp(&b.display_name().to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    builtins.append(&mut user);

    if let Some(tombstone) = tombstone {
        let index = tombstone.index.min(builtins.len());
        builtins.insert(
            index,
            ThemeRow {
                id: tombstone.id.clone(),
                name: tombstone.name.clone(),
                builtin: tombstone.builtin,
                status: ThemeRowStatus::Invalid,
                path: tombstone.path.clone(),
                diagnostics: vec![crate::theme::model::ThemeDiagnostic::error(
                    tombstone
                        .path
                        .clone()
                        .map(crate::theme::model::ThemeOrigin::User)
                        .unwrap_or(crate::theme::model::ThemeOrigin::BuiltIn),
                    None,
                    format!("`{}` is no longer installed", tombstone.id),
                )],
            },
        );
    }
    builtins
}
