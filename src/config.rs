use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TerminalKind {
    #[default]
    Kitty,
    Ghostty,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceConfig {
    #[serde(default = "default_true")]
    pub show_detail_panel: bool,
    #[serde(default = "default_date_format")]
    pub date_format: String,
    #[serde(default)]
    pub disable_animation: bool,
    /// Ask for confirmation before quitting (q / Ctrl+C). Default true.
    #[serde(default = "default_true")]
    pub confirm_quit: bool,
    /// Columns in the identities grid. 0 = auto (fit 1-2). Adjusted in-app
    /// with `[` / `]`.
    #[serde(default)]
    pub identity_columns: usize,
}

fn default_true() -> bool {
    true
}

fn default_date_format() -> String {
    "%Y-%m-%d %H:%M".to_string()
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            show_detail_panel: true,
            date_format: default_date_format(),
            disable_animation: false,
            confirm_quit: true,
            identity_columns: 0,
        }
    }
}

/// User-remappable keybindings. Each entry is a list of key specs so a user
/// can add their own binding without losing the defaults. Specs look like
/// `"F2"`, `"Ctrl+S"`, `"Alt+Enter"`, `"F10"` (parsed in `app`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindsConfig {
    #[serde(default = "default_save_keys")]
    pub save: Vec<String>,
    #[serde(default = "default_quit_keys")]
    pub quit: Vec<String>,
    #[serde(default = "default_help_keys")]
    pub help: Vec<String>,
    #[serde(default = "default_search_keys")]
    pub search: Vec<String>,
    #[serde(default = "default_add_host_keys")]
    pub add_host: Vec<String>,
    #[serde(default = "default_delete_keys")]
    pub delete: Vec<String>,
    #[serde(default = "default_duplicate_keys")]
    pub duplicate: Vec<String>,
    #[serde(default = "default_tag_filter_keys")]
    pub tag_filter: Vec<String>,
}

fn default_save_keys() -> Vec<String> {
    vec!["F2".to_string(), "Ctrl+S".to_string()]
}
fn default_quit_keys() -> Vec<String> {
    vec!["q".to_string()]
}
fn default_help_keys() -> Vec<String> {
    vec!["?".to_string(), "Shift+H".to_string()]
}
fn default_search_keys() -> Vec<String> {
    vec!["/".to_string()]
}
fn default_add_host_keys() -> Vec<String> {
    vec!["a".to_string()]
}
fn default_delete_keys() -> Vec<String> {
    vec!["d".to_string()]
}
fn default_duplicate_keys() -> Vec<String> {
    vec!["Shift+D".to_string()]
}
fn default_tag_filter_keys() -> Vec<String> {
    vec!["#".to_string()]
}

impl Default for KeybindsConfig {
    fn default() -> Self {
        Self {
            save: default_save_keys(),
            quit: default_quit_keys(),
            help: default_help_keys(),
            search: default_search_keys(),
            add_host: default_add_host_keys(),
            delete: default_delete_keys(),
            duplicate: default_duplicate_keys(),
            tag_filter: default_tag_filter_keys(),
        }
    }
}

impl KeybindsConfig {
    fn default_for(action: KeyAction) -> Vec<String> {
        match action {
            KeyAction::Save => default_save_keys(),
            KeyAction::Quit => default_quit_keys(),
            KeyAction::Help => default_help_keys(),
            KeyAction::Search => default_search_keys(),
            KeyAction::AddHost => default_add_host_keys(),
            KeyAction::Delete => default_delete_keys(),
            KeyAction::Duplicate => default_duplicate_keys(),
            KeyAction::TagFilter => default_tag_filter_keys(),
        }
    }

    /// Restore one action's bindings to its built-in default.
    pub fn reset_action(&mut self, action: KeyAction) {
        self.set(action, Self::default_for(action));
    }

    pub fn binds(&self, action: KeyAction) -> &[String] {
        match action {
            KeyAction::Save => &self.save,
            KeyAction::Quit => &self.quit,
            KeyAction::Help => &self.help,
            KeyAction::Search => &self.search,
            KeyAction::AddHost => &self.add_host,
            KeyAction::Delete => &self.delete,
            KeyAction::Duplicate => &self.duplicate,
            KeyAction::TagFilter => &self.tag_filter,
        }
    }

    pub fn set(&mut self, action: KeyAction, binds: Vec<String>) {
        match action {
            KeyAction::Save => self.save = binds,
            KeyAction::Quit => self.quit = binds,
            KeyAction::Help => self.help = binds,
            KeyAction::Search => self.search = binds,
            KeyAction::AddHost => self.add_host = binds,
            KeyAction::Delete => self.delete = binds,
            KeyAction::Duplicate => self.duplicate = binds,
            KeyAction::TagFilter => self.tag_filter = binds,
        }
    }

    /// Append `spec` to an action's bindings unless already present.
    pub fn add(&mut self, action: KeyAction, spec: String) {
        let mut binds = self.binds(action).to_vec();
        if !binds.iter().any(|b| b.eq_ignore_ascii_case(&spec)) {
            binds.push(spec);
            self.set(action, binds);
        }
    }
}

/// An action whose keybinding is user-configurable and editable in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Save,
    Quit,
    Help,
    Search,
    AddHost,
    Delete,
    Duplicate,
    TagFilter,
}

impl KeyAction {
    /// All editable actions, in display order.
    pub const ALL: [KeyAction; 8] = [
        KeyAction::Save,
        KeyAction::Quit,
        KeyAction::Help,
        KeyAction::Search,
        KeyAction::AddHost,
        KeyAction::Delete,
        KeyAction::Duplicate,
        KeyAction::TagFilter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            KeyAction::Save => "Save form",
            KeyAction::Quit => "Quit",
            KeyAction::Help => "Help",
            KeyAction::Search => "Search / palette",
            KeyAction::AddHost => "Add host",
            KeyAction::Delete => "Delete host",
            KeyAction::Duplicate => "Duplicate host",
            KeyAction::TagFilter => "Filter by tag",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub terminal: TerminalKind,
    pub launch_command: Option<String>,
    #[serde(default)]
    pub appearance: AppearanceConfig,
    #[serde(default)]
    pub keybinds: KeybindsConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            terminal: TerminalKind::Kitty,
            launch_command: None,
            appearance: AppearanceConfig::default(),
            keybinds: KeybindsConfig::default(),
        }
    }
}

/// Path to `config.toml` inside [`config_dir`].
pub fn config_file_path() -> anyhow::Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// Parse TOML config text (for unit tests and internal use).
pub fn parse_config_str(s: &str) -> anyhow::Result<AppConfig> {
    toml::from_str(s).map_err(|e| anyhow::anyhow!("invalid config.toml: {e}"))
}

fn default_config_toml() -> anyhow::Result<String> {
    toml::to_string_pretty(&AppConfig::default())
        .map_err(|e| anyhow::anyhow!("failed to serialize default config: {e}"))
}

/// Load config from XDG path, creating the directory and default file if missing.
pub fn load_config() -> anyhow::Result<AppConfig> {
    let path = config_file_path()?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        crate::secure_fs::restrict_dir(parent);
    }

    if !path.exists() {
        fs::write(&path, default_config_toml()?)?;
    }

    let content = fs::read_to_string(&path)?;
    parse_config_str(&content)
}

/// Serialize and atomically write `config` back to `config.toml`.
pub fn save_config(config: &AppConfig) -> anyhow::Result<()> {
    let path = config_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        crate::secure_fs::restrict_dir(parent);
    }
    let toml = toml::to_string_pretty(config)
        .map_err(|e| anyhow::anyhow!("failed to serialize config: {e}"))?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, toml)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Config directory (`~/.config/sshub` or `SSHUB_CONFIG_DIR`).
/// Falls back to `SSH_LAUNCHER_CONFIG_DIR` for backward compatibility.
/// Migrates data from `~/.config/ssh-launcher` if the new path doesn't exist yet.
pub fn config_dir() -> anyhow::Result<std::path::PathBuf> {
    if let Some(dir) = env_dir("SSHUB_CONFIG_DIR").or_else(|| env_dir("SSH_LAUNCHER_CONFIG_DIR")) {
        return Ok(dir);
    }
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME not set"))?;
    let new_dir = std::path::PathBuf::from(&home).join(".config/sshub");
    let legacy_dir = std::path::PathBuf::from(&home).join(".config/ssh-launcher");
    migrate_legacy_dir(&new_dir, &legacy_dir);
    Ok(new_dir)
}

/// Data directory for SQLite (`~/.local/share/sshub` or `SSHUB_DATA_DIR`).
/// Falls back to `SSH_LAUNCHER_DATA_DIR` for backward compatibility.
/// Migrates data from `~/.local/share/ssh-launcher` if the new path doesn't exist yet.
pub fn data_dir() -> anyhow::Result<std::path::PathBuf> {
    if let Some(dir) = env_dir("SSHUB_DATA_DIR").or_else(|| env_dir("SSH_LAUNCHER_DATA_DIR")) {
        return Ok(dir);
    }
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME not set"))?;
    let new_dir = std::path::PathBuf::from(&home).join(".local/share/sshub");
    let legacy_dir = std::path::PathBuf::from(&home).join(".local/share/ssh-launcher");
    migrate_legacy_dir(&new_dir, &legacy_dir);
    Ok(new_dir)
}

/// Env-var directory override; empty values are ignored so e.g.
/// `SSHUB_CONFIG_DIR=""` doesn't silently resolve to the CWD.
fn env_dir(var: &str) -> Option<std::path::PathBuf> {
    match std::env::var(var) {
        Ok(dir) if !dir.trim().is_empty() => Some(dir.into()),
        _ => None,
    }
}

/// If `new_dir` does not exist but `legacy_dir` does, copy the legacy directory
/// to the new location so user data is preserved on upgrade.
///
/// The copy is staged into a `<new_dir>.migrating` sibling and renamed into
/// place only when complete: a crash or I/O error mid-copy must not leave a
/// half-populated `new_dir`, because `new_dir.exists()` would then prevent the
/// migration from ever being retried (frozen partial copy, "lost" hosts).
fn migrate_legacy_dir(new_dir: &Path, legacy_dir: &Path) {
    if new_dir.exists() || !legacy_dir.exists() {
        return;
    }
    let staging = new_dir.with_extension("migrating");
    let _ = fs::remove_dir_all(&staging);
    let result =
        copy_dir_recursive(legacy_dir, &staging).and_then(|()| fs::rename(&staging, new_dir));
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&staging);
        eprintln!(
            "Warning: failed to migrate data from {}: {e}",
            legacy_dir.display()
        );
    } else {
        crate::secure_fs::restrict_dir(new_dir);
    }
}

/// Recursively copy a directory tree from `src` to `dst`.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_config_uses_defaults_for_empty_toml() {
        let config = parse_config_str("").unwrap();
        assert_eq!(config.terminal, TerminalKind::Kitty);
        assert!(config.launch_command.is_none());
        assert!(config.appearance.show_detail_panel);
        assert_eq!(config.appearance.date_format, "%Y-%m-%d %H:%M");
    }

    #[test]
    fn parse_config_applies_overrides() {
        let toml = r#"
terminal = "ghostty"
launch_command = "foot ssh {host}"

[appearance]
show_detail_panel = false
date_format = "%d/%m/%Y"
"#;
        let config = parse_config_str(toml).unwrap();
        assert_eq!(config.terminal, TerminalKind::Ghostty);
        assert_eq!(config.launch_command.as_deref(), Some("foot ssh {host}"));
        assert!(!config.appearance.show_detail_panel);
        assert_eq!(config.appearance.date_format, "%d/%m/%Y");
    }

    #[test]
    fn parse_config_fixture_toml() {
        let fixture = include_str!("../tests/fixtures/config.toml");
        let config = parse_config_str(fixture).unwrap();
        assert_eq!(config.terminal, TerminalKind::Kitty);
        assert!(config.launch_command.is_none());
        assert!(config.appearance.show_detail_panel);
        assert_eq!(config.appearance.date_format, "%Y-%m-%d %H:%M");
    }

    #[test]
    fn parse_config_rejects_invalid_toml() {
        let err = parse_config_str("terminal = [[[").unwrap_err();
        assert!(
            err.to_string().contains("invalid config.toml"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn default_config_toml_roundtrips() {
        let toml = default_config_toml().unwrap();
        let config = parse_config_str(&toml).unwrap();
        assert_eq!(config.terminal, TerminalKind::Kitty);
        assert!(config.launch_command.is_none());
        assert!(config.appearance.show_detail_panel);
        assert_eq!(config.appearance.date_format, "%Y-%m-%d %H:%M");
    }
}
