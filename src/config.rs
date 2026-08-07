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
    /// Id of the runtime theme to activate at start-up — a built-in or the file
    /// stem of `themes/<id>.toml`. A missing or broken id falls back to
    /// `default` with a non-fatal hint and never rewrites this file, so the
    /// user's choice survives a temporarily unreadable theme.
    #[serde(default = "default_active_theme")]
    pub active_theme: String,
}

fn default_true() -> bool {
    true
}

fn default_active_theme() -> String {
    "default".to_string()
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

/// Clipboard behaviour. Currently only governs what the *remote* side may do:
/// SSHub's own copy shortcuts are unaffected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardConfig {
    /// Relay OSC 52 clipboard writes emitted by applications inside a session
    /// PTY to the host clipboard. On by default — that is what makes copying
    /// inside a nested multiplexer or `clipboard=osc52` editor work at all.
    /// Set to `false` to drop those writes: the remote then cannot touch the
    /// local clipboard.
    #[serde(default = "default_true")]
    pub relay_from_pty: bool,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            relay_from_pty: true,
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
            active_theme: default_active_theme(),
        }
    }
}

/// User-remappable keybindings. See [`crate::keybinds`].
pub use crate::keybinds::{KeyAction, KeybindsConfig};

/// SSH source selection. `config_path` overrides the imported ssh_config
/// location for this profile (`~` expanded); environment overrides
/// (`SSHUB_SSH_CONFIG` / `SSH_LAUNCHER_SSH_CONFIG`) still win.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SshSourceConfig {
    #[serde(default)]
    pub config_path: Option<String>,
}

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
    #[serde(default)]
    pub clipboard: ClipboardConfig,
    #[serde(default)]
    pub ssh: SshSourceConfig,
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
    load_config_at(&config_file_path()?)
}

/// Load config from an explicit `config.toml` path (profile-aware entry point).
pub fn load_config_at(path: &Path) -> anyhow::Result<AppConfig> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        crate::secure_fs::restrict_dir(parent);
    }

    if !path.exists() {
        write_private_file(path, default_config_toml()?.as_bytes())?;
    }

    let content = fs::read_to_string(path)?;
    let mut config = parse_config_str(&content)?;
    // One-shot keybind migrations for upgrading installs (see
    // KeybindsConfig::migrate_pre_sftp_tabs / migrate_help_frees_known_hosts).
    // Persist so each runs exactly once — otherwise a user who deliberately
    // keeps a legacy bind would have it silently rewritten on every launch.
    let mut migrated = false;
    if config.keybinds.migrate_pre_sftp_tabs(&content) {
        migrated = true;
    }
    if config.keybinds.migrate_help_frees_known_hosts(&content) {
        migrated = true;
    }
    if migrated {
        // Persist via save_config_at so the migration runs once — it merges
        // through toml_edit (preserving comments + keys we don't model) and
        // writes atomically, unlike a raw serialize+overwrite.
        let _ = save_config_at(path, &config);
    }
    Ok(config)
}

/// Serialize and atomically write `config` back to `config.toml`.
pub fn save_config(config: &AppConfig) -> anyhow::Result<()> {
    save_config_at(&config_file_path()?, config)
}

/// Save config to an explicit `config.toml` path (profile-aware entry point).
pub fn save_config_at(path: &Path, config: &AppConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        crate::secure_fs::restrict_dir(parent);
    }
    let existing = fs::read_to_string(path).unwrap_or_default();
    let merged = merge_config_document(&existing, config)?;

    let tmp = path.with_extension("toml.tmp");
    write_private_file(&tmp, merged.as_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Merge our fields into `existing` rather than replacing it, so user comments
/// and any keys we don't model survive a save (which fires on trivial UI
/// actions like zoom). An `existing` that does not parse is treated as empty —
/// a corrupt file must not block writing a valid one.
fn merge_config_document(existing: &str, config: &AppConfig) -> anyhow::Result<String> {
    let generated = toml::to_string_pretty(config)
        .map_err(|e| anyhow::anyhow!("failed to serialize config: {e}"))?;
    let new_doc: toml_edit::DocumentMut = generated
        .parse()
        .map_err(|e| anyhow::anyhow!("failed to parse serialized config: {e}"))?;
    let mut doc: toml_edit::DocumentMut = existing.parse().unwrap_or_default();
    merge_toml_table(doc.as_table_mut(), new_doc.as_table());
    Ok(doc.to_string())
}

fn write_private_file(path: &Path, content: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;

    if path.exists() {
        anyhow::ensure!(
            !fs::symlink_metadata(path)?.file_type().is_symlink(),
            "refusing symlink temporary file: {}",
            path.display()
        );
        fs::remove_file(path)?;
    }
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(content)?;
    file.sync_all()?;
    crate::secure_fs::restrict_file(path);
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
    let dir = config_dir_path()?;
    // Only the default HOME location has a legacy predecessor to inherit from;
    // an explicit override names the directory the user wants, verbatim.
    if env_dir("SSHUB_CONFIG_DIR")
        .or_else(|| env_dir("SSH_LAUNCHER_CONFIG_DIR"))
        .is_none()
    {
        let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME not set"))?;
        migrate_legacy_dir(
            &dir,
            &std::path::PathBuf::from(&home).join(".config/ssh-launcher"),
        );
    }
    Ok(dir)
}

/// Where the config directory *is*, with no side effects at all.
///
/// Same override order as [`config_dir`] — `SSHUB_CONFIG_DIR`, then
/// `SSH_LAUNCHER_CONFIG_DIR`, then `$HOME/.config/sshub` — but it neither
/// migrates a legacy tree nor creates anything. Read-only callers such as
/// `sshub theme list` and `sshub theme show` use this: merely asking which
/// themes are installed must not write to the user's home directory.
pub fn config_dir_path() -> anyhow::Result<std::path::PathBuf> {
    if let Some(dir) = env_dir("SSHUB_CONFIG_DIR").or_else(|| env_dir("SSH_LAUNCHER_CONFIG_DIR")) {
        return Ok(dir);
    }
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME not set"))?;
    Ok(std::path::PathBuf::from(&home).join(".config/sshub"))
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

/// Run `f` with `SSHUB_CONFIG_DIR` pointed at `dir`, holding a process-wide lock
/// so parallel tests cannot race on that env var (macOS CI surfaces this often).
#[cfg(test)]
pub(crate) fn with_test_config_dir<R>(dir: &std::path::Path, f: impl FnOnce() -> R) -> R {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var("SSHUB_CONFIG_DIR", dir);
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    std::env::remove_var("SSHUB_CONFIG_DIR");
    match out {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// The one lock every environment-mutating test helper here takes, so no two
/// of them can be mid-mutation at the same time. Never take it twice on one
/// thread — these helpers do not nest.
#[cfg(test)]
fn env_lock() -> &'static std::sync::Mutex<()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Set one env var for a scope and put the previous value back on drop.
///
/// Only valid while [`env_lock`] is held — that is what makes it safe under
/// `cargo test`'s thread pool.
#[cfg(test)]
pub(crate) struct EnvVar {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

#[cfg(test)]
impl EnvVar {
    pub(crate) fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }

    pub(crate) fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, previous }
    }
}

#[cfg(test)]
impl Drop for EnvVar {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

/// Run `f` with `HOME` pointed at `home` and both config-dir overrides cleared,
/// so the HOME branch of [`config_dir_path`] is the one under test.
///
/// Two locks, because the two variables have two different sets of contenders:
/// `crate::test_env::lock_home()` is the crate-wide `$HOME` lock that
/// `ssh::keyfile`, `ssh::resolver` and `app::tests::misc` already take, and
/// [`env_lock`] covers `SSHUB_CONFIG_DIR` / `SSH_LAUNCHER_CONFIG_DIR` against
/// [`with_test_config_dir`]. HOME first, config env second — every caller takes
/// them in that order, so the pair cannot deadlock.
#[cfg(test)]
pub(crate) fn with_test_home<R>(home: &std::path::Path, f: impl FnOnce() -> R) -> R {
    let _home_guard = crate::test_env::lock_home();
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _home = EnvVar::set("HOME", home);
    let _primary = EnvVar::unset("SSHUB_CONFIG_DIR");
    let _fallback = EnvVar::unset("SSH_LAUNCHER_CONFIG_DIR");
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
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
///
/// Symlinks are refused outright — the root as well as every entry. The
/// migration runs unattended on upgrade with the user's own privileges, so a
/// link planted in the legacy directory would otherwise have its *target*
/// copied into the new config directory (`fs::copy` follows links) or send the
/// walk out of the tree entirely. Anything that is neither a regular file nor a
/// real directory is rejected the same way, as [`ErrorKind::InvalidData`].
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    let rejected = |path: &Path, what: &str| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("refusing to copy {what}: {}", path.display()),
        )
    };

    if fs::symlink_metadata(src)?.file_type().is_symlink() {
        return Err(rejected(src, "a symlinked directory"));
    }

    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        // `symlink_metadata` rather than `entry.file_type()`: both report the
        // link itself, but using the same call for the root and for every
        // entry makes the do-not-follow semantics explicit and uniform.
        let file_type = fs::symlink_metadata(&path)?.file_type();
        let dest_path = dst.join(entry.file_name());
        if file_type.is_symlink() {
            return Err(rejected(&path, "a symlink"));
        } else if file_type.is_dir() {
            // The directory is still path-based after this check. Closing that
            // replacement race requires an fd-based openat traversal.
            copy_dir_recursive(&path, &dest_path)?;
        } else if file_type.is_file() {
            copy_regular_file(&path, &dest_path)?;
        } else {
            return Err(rejected(&path, "a special file"));
        }
    }
    Ok(())
}

/// Copy one regular file without resolving a replacement symlink on Unix.
fn copy_regular_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    let mut source_options = fs::OpenOptions::new();
    source_options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        source_options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut source = source_options.open(src)?;
    let source_metadata = source.metadata()?;
    if !source_metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("refusing to copy a special file: {}", src.display()),
        ));
    }

    let mut destination_options = fs::OpenOptions::new();
    destination_options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        destination_options.custom_flags(libc::O_NOFOLLOW);
        destination_options.mode(source_metadata.permissions().mode());
    }
    let mut destination = destination_options.open(dst)?;
    std::io::copy(&mut source, &mut destination)?;
    #[cfg(unix)]
    destination.set_permissions(source_metadata.permissions())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_relay_defaults_to_on_for_configs_without_the_section() {
        // Every config written before the section existed must keep behaving
        // exactly as it did — the relay is opt-out, not opt-in.
        let config = parse_config_str("").unwrap();
        assert!(config.clipboard.relay_from_pty);
        assert!(AppConfig::default().clipboard.relay_from_pty);
    }

    #[test]
    fn clipboard_relay_can_be_switched_off() {
        let config = parse_config_str("[clipboard]\nrelay_from_pty = false\n").unwrap();
        assert!(!config.clipboard.relay_from_pty);
    }

    #[test]
    fn an_empty_clipboard_section_keeps_the_default() {
        let config = parse_config_str("[clipboard]\n").unwrap();
        assert!(config.clipboard.relay_from_pty);
    }

    #[test]
    fn save_config_preserves_comments_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        with_test_config_dir(dir.path(), || {
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
        });
    }

    /// Resolving the config path is a *query*: `theme list` and `theme show`
    /// both go through it while only reading, so it must not create the new
    /// directory nor drag a legacy tree along behind the user's back.
    #[test]
    fn config_dir_path_resolves_without_touching_the_filesystem() {
        let home = tempfile::tempdir().unwrap();
        let legacy = home.path().join(".config/ssh-launcher");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("config.toml"), "# legacy\n").unwrap();

        let resolved = with_test_home(home.path(), config_dir_path);

        let expected = home.path().join(".config/sshub");
        assert_eq!(resolved.unwrap(), expected);
        assert!(
            !expected.exists(),
            "resolving the path created {}",
            expected.display()
        );
        assert!(
            !expected.with_extension("migrating").exists(),
            "resolving the path left a staging directory behind"
        );
        assert_eq!(
            std::fs::read_to_string(legacy.join("config.toml")).unwrap(),
            "# legacy\n",
            "the legacy tree was migrated by a pure path query"
        );
    }

    /// Both env overrides keep working, and neither is affected by HOME.
    #[test]
    fn config_dir_path_honours_both_env_overrides_in_order() {
        let home = tempfile::tempdir().unwrap();
        let primary = tempfile::tempdir().unwrap();
        let fallback = tempfile::tempdir().unwrap();

        with_test_home(home.path(), || {
            let _guard = EnvVar::set("SSH_LAUNCHER_CONFIG_DIR", fallback.path());
            assert_eq!(config_dir_path().unwrap(), fallback.path());
            let _primary = EnvVar::set("SSHUB_CONFIG_DIR", primary.path());
            assert_eq!(config_dir_path().unwrap(), primary.path());
        });
    }

    /// A legacy entry that is a *symlink* must never be followed: the migration
    /// runs on upgrade with the user's own privileges, and following a planted
    /// link would copy a file from outside the legacy tree into the new config
    /// directory (or, for a directory link, walk out of the tree entirely).
    #[cfg(unix)]
    #[test]
    fn a_symlinked_legacy_entry_aborts_the_migration_untouched() {
        for kind in ["file", "dir"] {
            let root = tempfile::tempdir().unwrap();
            let legacy = root.path().join("legacy");
            std::fs::create_dir_all(&legacy).unwrap();
            std::fs::write(legacy.join("config.toml"), "# real\n").unwrap();

            // The link's target lives *outside* the legacy tree — exactly the
            // file an attacker would be trying to have copied into the new dir.
            let outside = root.path().join("outside");
            let link = legacy.join("planted");
            match kind {
                "file" => {
                    std::fs::write(&outside, "secret\n").unwrap();
                    std::os::unix::fs::symlink(&outside, &link).unwrap();
                }
                _ => {
                    std::fs::create_dir(&outside).unwrap();
                    std::fs::write(outside.join("secret.txt"), "secret\n").unwrap();
                    std::os::unix::fs::symlink(&outside, &link).unwrap();
                }
            }

            let new_dir = root.path().join("sshub");
            migrate_legacy_dir(&new_dir, &legacy);

            assert!(
                !new_dir.exists(),
                "{kind} symlink: the migration completed into {}",
                new_dir.display()
            );
            assert!(
                !new_dir.with_extension("migrating").exists(),
                "{kind} symlink: the staging directory was left behind"
            );
            assert!(
                outside.exists(),
                "{kind} symlink: the link target was removed"
            );
            assert!(
                link.symlink_metadata().unwrap().file_type().is_symlink(),
                "{kind} symlink: the legacy entry was replaced"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn copy_regular_file_rejects_a_replaced_symlink_source() {
        let root = tempfile::tempdir().unwrap();
        let source_dir = root.path().join("legacy");
        std::fs::create_dir(&source_dir).unwrap();
        let secret = root.path().join("outside-secret");
        std::fs::write(&secret, "PRIVATE KEY MATERIAL\n").unwrap();
        let replaced_source = source_dir.join("credentials.json");
        std::os::unix::fs::symlink(&secret, &replaced_source).unwrap();
        let destination = root.path().join("sshub/credentials.json");
        std::fs::create_dir(destination.parent().unwrap()).unwrap();

        let error = copy_regular_file(&replaced_source, &destination).unwrap_err();

        assert_eq!(error.raw_os_error(), Some(libc::ELOOP));
        assert!(
            !destination.exists(),
            "the symlink target was copied into {}",
            destination.display()
        );
    }

    #[cfg(unix)]
    #[test]
    fn copy_regular_file_rejects_a_symlink_destination_without_touching_its_target() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("legacy/credentials.json");
        std::fs::create_dir(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "MIGRATED SECRET\n").unwrap();
        let victim = root.path().join("outside-victim");
        std::fs::write(&victim, "KEEP ME\n").unwrap();
        let destination = root.path().join("sshub/credentials.json");
        std::fs::create_dir(destination.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&victim, &destination).unwrap();

        assert!(copy_regular_file(&source, &destination).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "KEEP ME\n");
    }

    #[cfg(unix)]
    #[test]
    fn copy_regular_file_rejects_an_existing_destination_without_changing_it() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("legacy/credentials.json");
        std::fs::create_dir(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "MIGRATED SECRET\n").unwrap();
        let destination = root.path().join("sshub/credentials.json");
        std::fs::create_dir(destination.parent().unwrap()).unwrap();
        std::fs::write(&destination, "KEEP ME\n").unwrap();
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o640)).unwrap();

        assert!(copy_regular_file(&source, &destination).is_err());
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "KEEP ME\n");
        assert_eq!(
            destination.metadata().unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    /// The legacy root itself being a symlink is the same refusal.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_legacy_root_aborts_the_migration() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("elsewhere");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("config.toml"), "# real\n").unwrap();
        let legacy = root.path().join("legacy");
        std::os::unix::fs::symlink(&real, &legacy).unwrap();

        let new_dir = root.path().join("sshub");
        migrate_legacy_dir(&new_dir, &legacy);

        assert!(!new_dir.exists(), "a symlinked legacy root was followed");
        assert!(!new_dir.with_extension("migrating").exists());
        assert_eq!(
            std::fs::read_to_string(real.join("config.toml")).unwrap(),
            "# real\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn migration_preserves_restrictive_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("legacy");
        std::fs::create_dir(&legacy).unwrap();
        let source = legacy.join("credentials.json");
        std::fs::write(&source, "secret\n").unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();

        let new_dir = root.path().join("sshub");
        migrate_legacy_dir(&new_dir, &legacy);

        let destination = new_dir.join("credentials.json");
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "secret\n");
        assert_eq!(
            destination.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn migration_copies_nested_regular_files() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("legacy");
        let nested = legacy.join("themes/custom");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(legacy.join("config.toml"), "# legacy\n").unwrap();
        std::fs::write(nested.join("theme.toml"), "name = \"custom\"\n").unwrap();

        let new_dir = root.path().join("sshub");
        migrate_legacy_dir(&new_dir, &legacy);

        assert_eq!(
            std::fs::read_to_string(new_dir.join("config.toml")).unwrap(),
            "# legacy\n"
        );
        assert_eq!(
            std::fs::read_to_string(new_dir.join("themes/custom/theme.toml")).unwrap(),
            "name = \"custom\"\n"
        );
    }

    /// The pure half of [`save_config`]: what would be written for `config`
    /// given `original` as the file on disk. Keeps the round-trip tests off the
    /// filesystem (and off the process-wide `SSHUB_CONFIG_DIR`).
    fn merge_config_for_test(original: &str, config: &AppConfig) -> anyhow::Result<String> {
        merge_config_document(original, config)
    }

    #[test]
    fn parse_config_theme_defaults_to_default() {
        let config = parse_config_str("").unwrap();
        assert_eq!(config.appearance.active_theme, "default");
    }

    #[test]
    fn active_theme_roundtrips_without_removing_unknown_config() {
        let original = "# keep\n[appearance]\nactive_theme = \"aqua\"\nfuture = 7\n";
        let mut config = parse_config_str(original).unwrap();
        config.appearance.active_theme = "fire".into();
        let saved = merge_config_for_test(original, &config).unwrap();
        assert!(saved.contains("# keep"), "comment lost: {saved}");
        assert!(saved.contains("future = 7"), "unknown key lost: {saved}");
        assert!(
            saved.contains("active_theme = \"fire\""),
            "our change not written: {saved}"
        );
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
        // The fixture predates `active_theme`, which is exactly the spec's
        // backwards-compatibility case: an older config.toml must still load
        // and simply get `default`.
        assert_eq!(config.appearance.active_theme, "default");
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
        assert!(
            toml.contains("active_theme = \"default\""),
            "active_theme missing from the generated default config: {toml}"
        );
        let config = parse_config_str(&toml).unwrap();
        assert!(config.appearance.show_detail_panel);
        assert_eq!(config.appearance.date_format, "%Y-%m-%d %H:%M");
        assert_eq!(config.appearance.active_theme, "default");
    }
}
