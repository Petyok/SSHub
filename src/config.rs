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

/// User-remappable keybindings. See [`crate::keybinds`].
pub use crate::keybinds::{KeyAction, KeybindsConfig};

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
    let mut config = parse_config_str(&content)?;
    // Migrate keybinds written before the SFTP tab was inserted as tab #2, so
    // upgrading users don't get misrouted digit navigation (see
    // KeybindsConfig::migrate_pre_sftp_tabs). Persist the migrated config so it
    // runs exactly once — otherwise a user who deliberately keeps a pre-SFTP
    // tab digit would have it silently rewritten on every launch.
    if config.keybinds.migrate_pre_sftp_tabs(&content) {
        // Persist via save_config so the migration runs once — it merges through
        // toml_edit (preserving comments + keys we don't model) and writes
        // atomically, unlike a raw serialize+overwrite.
        let _ = save_config(&config);
    }
    Ok(config)
}

/// Serialize and atomically write `config` back to `config.toml`.
pub fn save_config(config: &AppConfig) -> anyhow::Result<()> {
    let path = config_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        crate::secure_fs::restrict_dir(parent);
    }
    // Merge our fields into the existing document rather than replacing it, so
    // user comments and any keys we don't model survive a save (which fires on
    // trivial UI actions like zoom).
    let generated = toml::to_string_pretty(config)
        .map_err(|e| anyhow::anyhow!("failed to serialize config: {e}"))?;
    let new_doc: toml_edit::DocumentMut = generated
        .parse()
        .map_err(|e| anyhow::anyhow!("failed to parse serialized config: {e}"))?;
    let mut doc: toml_edit::DocumentMut = fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();
    merge_toml_table(doc.as_table_mut(), new_doc.as_table());

    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, doc.to_string())?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Deep-merge every key of `src` into `dst`, recursing into sub-tables so
/// unrelated keys (and their comments) in `dst` are left untouched.
fn merge_toml_table(dst: &mut toml_edit::Table, src: &toml_edit::Table) {
    for (key, src_item) in src.iter() {
        match (dst.get_mut(key), src_item) {
            (Some(toml_edit::Item::Table(dst_sub)), toml_edit::Item::Table(src_sub)) => {
                merge_toml_table(dst_sub, src_sub);
            }
            // Existing key: overwrite only the value, leaving the key's leading
            // comment/whitespace decor intact.
            (Some(existing), _) => {
                *existing = src_item.clone();
            }
            (None, _) => {
                dst.insert(key, src_item.clone());
            }
        }
    }
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
    fn save_config_preserves_comments_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SSHUB_CONFIG_DIR", dir.path());
        let path = config_file_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "# my hand-written note\nterminal = \"kitty\"\nfuture_option = true  # keep me\n",
        )
        .unwrap();

        let config = AppConfig {
            terminal: TerminalKind::Ghostty,
            ..AppConfig::default()
        };
        save_config(&config).unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("# my hand-written note"),
            "comment lost: {after}"
        );
        assert!(
            after.contains("future_option = true"),
            "unknown key lost: {after}"
        );
        assert!(after.contains("ghostty"), "our change not written: {after}");
        std::env::remove_var("SSHUB_CONFIG_DIR");
    }

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
