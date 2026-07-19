use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceConfig {
    #[serde(default = "default_true")]
    pub show_detail_panel: bool,
    #[serde(default = "default_date_format")]
    pub date_format: String,
    /// Reduced-motion toggle. When true, skip all UI motion (the startup
    /// splash and every panel/toast slide + morph); surfaces jump straight to
    /// their final state. Default off. Also flipped in Settings (`Ctrl+H`).
    #[serde(default)]
    pub disable_animation: bool,
    /// Ask for confirmation before quitting (q / Ctrl+C). Default true.
    #[serde(default = "default_true")]
    pub confirm_quit: bool,
    /// Columns in the identities grid. 0 = auto (fit 1-2). Adjusted in-app
    /// with `[` / `]`.
    #[serde(default)]
    pub identity_columns: usize,
    /// Show the detected OS logo in the host detail view. Default true.
    #[serde(default = "default_true")]
    pub os_logo: bool,
    /// Paint a solid background behind the whole UI instead of leaving cells
    /// transparent. Fixes unreadable text on transparent terminals. Default off.
    #[serde(default)]
    pub opaque_background: bool,
}

fn default_true() -> bool {
    true
}

fn default_session_log_max_bytes() -> u64 {
    10 * 1024 * 1024
}

fn default_session_log_retention() -> usize {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLoggingConfig {
    /// When true, embedded SSH sessions write PTY output to log files unless a
    /// per-host override disables it.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_session_log_max_bytes")]
    pub max_file_bytes: u64,
    #[serde(default = "default_session_log_retention")]
    pub retention_files: usize,
}

impl Default for SessionLoggingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_file_bytes: default_session_log_max_bytes(),
            retention_files: default_session_log_retention(),
        }
    }
}

fn default_tunnel_reconnect_max_attempts() -> u32 {
    12
}

fn default_tunnel_reconnect_initial_ms() -> u64 {
    1000
}

fn default_tunnel_reconnect_max_ms() -> u64 {
    60_000
}

fn default_tunnel_reconnect_jitter() -> f64 {
    0.25
}

fn default_tunnel_stable_secs() -> u64 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelReconnectConfig {
    /// Maximum consecutive reconnect attempts after an unexpected exit (`0` = unlimited).
    #[serde(default = "default_tunnel_reconnect_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_tunnel_reconnect_initial_ms")]
    pub initial_delay_ms: u64,
    #[serde(default = "default_tunnel_reconnect_max_ms")]
    pub max_delay_ms: u64,
    /// Jitter factor applied around the backoff delay (`0.25` → ±25%).
    #[serde(default = "default_tunnel_reconnect_jitter")]
    pub jitter_ratio: f64,
    /// Child must stay alive this long before a reconnect counts as successful.
    #[serde(default = "default_tunnel_stable_secs")]
    pub stable_secs: u64,
}

impl Default for TunnelReconnectConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_tunnel_reconnect_max_attempts(),
            initial_delay_ms: default_tunnel_reconnect_initial_ms(),
            max_delay_ms: default_tunnel_reconnect_max_ms(),
            jitter_ratio: default_tunnel_reconnect_jitter(),
            stable_secs: default_tunnel_stable_secs(),
        }
    }
}

/// Attempt counter after a tunnel child exits. Short uptimes (flapping spawn)
/// advance the series; a run longer than `stable_secs` resets the budget.
pub fn tunnel_failure_attempt(current: u32, uptime_secs: u64, stable_secs: u64) -> u32 {
    if uptime_secs >= stable_secs {
        0
    } else {
        current.saturating_add(1)
    }
}

impl TunnelReconnectConfig {
    /// Human-readable value for settings row `row` (0..4).
    pub fn display_field(&self, row: usize) -> String {
        match row {
            0 => {
                if self.max_attempts == 0 {
                    "unlimited".into()
                } else {
                    self.max_attempts.to_string()
                }
            }
            1 => format!("{} s", self.initial_delay_ms / 1000),
            2 => format!("{} s", self.max_delay_ms / 1000),
            3 => format!("{} s", self.stable_secs),
            4 => format!("{:.0}%", self.jitter_ratio * 100.0),
            _ => String::new(),
        }
    }

    /// Nudge field `row` by `delta` sign (`+1` / `-1`). Clamps to sane bounds.
    pub fn adjust_field(&mut self, row: usize, delta: i32) {
        match row {
            0 => {
                let next = self.max_attempts as i64 + i64::from(delta);
                self.max_attempts = next.clamp(0, 999) as u32;
            }
            1 => {
                let step = 1_000_i64;
                let next =
                    (self.initial_delay_ms as i64 + i64::from(delta) * step).clamp(1_000, 300_000);
                self.initial_delay_ms = next as u64;
                if self.initial_delay_ms > self.max_delay_ms {
                    self.max_delay_ms = self.initial_delay_ms;
                }
            }
            2 => {
                let step = 5_000_i64;
                let next =
                    (self.max_delay_ms as i64 + i64::from(delta) * step).clamp(5_000, 600_000);
                self.max_delay_ms = next as u64;
                if self.max_delay_ms < self.initial_delay_ms {
                    self.initial_delay_ms = self.max_delay_ms;
                }
            }
            3 => {
                let next = self.stable_secs as i64 + i64::from(delta);
                self.stable_secs = next.clamp(1, 120) as u64;
            }
            4 => {
                let next = self.jitter_ratio + f64::from(delta) * 0.05;
                self.jitter_ratio = next.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }

    /// Restore one settings row to its built-in default.
    pub fn reset_field(&mut self, row: usize) {
        let d = Self::default();
        match row {
            0 => self.max_attempts = d.max_attempts,
            1 => self.initial_delay_ms = d.initial_delay_ms,
            2 => self.max_delay_ms = d.max_delay_ms,
            3 => self.stable_secs = d.stable_secs,
            4 => self.jitter_ratio = d.jitter_ratio,
            _ => {}
        }
    }
}

/// Exponential backoff with deterministic jitter for a tunnel reconnect attempt.
pub fn tunnel_backoff_delay(attempt: u32, tunnel_id: i64, cfg: &TunnelReconnectConfig) -> Duration {
    use std::time::Duration;
    let attempt = attempt.max(1);
    let exp = attempt.saturating_sub(1).min(20);
    let base = cfg
        .initial_delay_ms
        .saturating_mul(1u64 << exp)
        .min(cfg.max_delay_ms);
    let jitter = jitter_factor(tunnel_id, attempt, cfg.jitter_ratio);
    Duration::from_millis(((base as f64) * jitter).max(1.0) as u64)
}

fn jitter_factor(tunnel_id: i64, attempt: u32, jitter_ratio: f64) -> f64 {
    let hash = (tunnel_id as u64)
        .wrapping_mul(31)
        .wrapping_add(attempt as u64)
        .wrapping_mul(1_103_515_245);
    let frac = (hash % 2000) as f64 / 1000.0;
    1.0 + jitter_ratio * (frac - 1.0)
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
            os_logo: true,
            opaque_background: false,
        }
    }
}

/// User-remappable keybindings. See [`crate::keybinds`].
pub use crate::keybinds::{KeyAction, KeybindsConfig};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub appearance: AppearanceConfig,
    #[serde(default)]
    pub session_logging: SessionLoggingConfig,
    #[serde(default)]
    pub tunnel_reconnect: TunnelReconnectConfig,
    #[serde(default)]
    pub keybinds: KeybindsConfig,
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
            "# my hand-written note\nfuture_option = true  # keep me\n\n[appearance]\ndate_format = \"%Y-%m-%d %H:%M\"\n",
        )
        .unwrap();

        let config = AppConfig {
            appearance: AppearanceConfig {
                date_format: "%d/%m/%Y".to_string(),
                ..AppearanceConfig::default()
            },
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
        assert!(
            after.contains("%d/%m/%Y"),
            "our change not written: {after}"
        );
        std::env::remove_var("SSHUB_CONFIG_DIR");
    }

    #[test]
    fn parse_config_uses_defaults_for_empty_toml() {
        let config = parse_config_str("").unwrap();
        assert!(config.appearance.show_detail_panel);
        assert_eq!(config.appearance.date_format, "%Y-%m-%d %H:%M");
    }

    #[test]
    fn parse_config_session_logging_defaults() {
        let config = parse_config_str("").unwrap();
        assert!(!config.session_logging.enabled);
        assert_eq!(config.session_logging.max_file_bytes, 10 * 1024 * 1024);
        assert_eq!(config.session_logging.retention_files, 50);
    }

    #[test]
    fn parse_config_tunnel_reconnect_defaults() {
        let config = parse_config_str("").unwrap();
        assert_eq!(config.tunnel_reconnect.max_attempts, 12);
        assert_eq!(config.tunnel_reconnect.initial_delay_ms, 1000);
        assert_eq!(config.tunnel_reconnect.max_delay_ms, 60_000);
        assert_eq!(config.tunnel_reconnect.stable_secs, 5);
        assert!((config.tunnel_reconnect.jitter_ratio - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn tunnel_failure_attempt_counts_flaps_and_resets_after_stable() {
        assert_eq!(tunnel_failure_attempt(0, 0, 5), 1);
        assert_eq!(tunnel_failure_attempt(1, 2, 5), 2);
        assert_eq!(tunnel_failure_attempt(2, 5, 5), 0);
        assert_eq!(tunnel_failure_attempt(4, 10, 5), 0);
    }

    #[test]
    fn tunnel_backoff_grows_and_caps() {
        let cfg = TunnelReconnectConfig::default();
        let d1 = tunnel_backoff_delay(1, 1, &cfg);
        let d2 = tunnel_backoff_delay(2, 1, &cfg);
        let d10 = tunnel_backoff_delay(10, 1, &cfg);
        assert!(d2 >= d1);
        assert!(d10 <= Duration::from_millis((cfg.max_delay_ms as f64 * 1.26) as u64));
    }

    #[test]
    fn tunnel_backoff_jitter_bounded() {
        let cfg = TunnelReconnectConfig {
            jitter_ratio: 0.25,
            ..Default::default()
        };
        for attempt in 1..=5 {
            let d = tunnel_backoff_delay(attempt, 42, &cfg);
            let base = cfg
                .initial_delay_ms
                .saturating_mul(1u64 << (attempt - 1).min(20));
            let capped = base.min(cfg.max_delay_ms);
            let min = (capped as f64 * 0.75) as u64;
            let max = (capped as f64 * 1.25) as u64;
            assert!(
                d.as_millis() as u64 >= min.saturating_sub(1),
                "attempt {attempt}: {d:?} below {min}"
            );
            assert!(
                d.as_millis() as u64 <= max + 1,
                "attempt {attempt}: {d:?} above {max}"
            );
        }
    }

    #[test]
    fn tunnel_reconnect_display_uses_seconds_for_delays() {
        let cfg = TunnelReconnectConfig::default();
        assert_eq!(cfg.display_field(1), "1 s");
        assert_eq!(cfg.display_field(2), "60 s");
    }

    #[test]
    fn tunnel_reconnect_adjust_keeps_delay_order() {
        let mut cfg = TunnelReconnectConfig::default();
        for _ in 0..200 {
            cfg.adjust_field(1, 1);
        }
        assert!(cfg.initial_delay_ms <= cfg.max_delay_ms);
        for _ in 0..200 {
            cfg.adjust_field(2, -1);
        }
        assert!(cfg.initial_delay_ms <= cfg.max_delay_ms);
        assert_eq!(cfg.display_field(0), "12");
        cfg.adjust_field(0, -20);
        assert_eq!(cfg.max_attempts, 0);
        assert_eq!(cfg.display_field(0), "unlimited");
    }

    #[test]
    fn parse_config_applies_overrides() {
        // Old configs may still carry the removed `terminal` / `launch_command`
        // keys; they must be silently ignored (no deny_unknown_fields) so the
        // rest of the config still loads.
        let toml = r#"
terminal = "ghostty"
launch_command = "foot ssh {host}"

[appearance]
show_detail_panel = false
date_format = "%d/%m/%Y"
"#;
        let config = parse_config_str(toml).unwrap();
        assert!(!config.appearance.show_detail_panel);
        assert_eq!(config.appearance.date_format, "%d/%m/%Y");
    }

    #[test]
    fn parse_config_fixture_toml() {
        let fixture = include_str!("../tests/fixtures/config.toml");
        let config = parse_config_str(fixture).unwrap();
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
        assert!(config.appearance.show_detail_panel);
        assert_eq!(config.appearance.date_format, "%Y-%m-%d %H:%M");
    }
}
