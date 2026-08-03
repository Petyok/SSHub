//! Isolated profiles — each profile owns its launcher/metadata databases,
//! `config.toml`, fallback credentials, session logs, and tunnel state.
//!
//! Layout (profile mode):
//!
//! ```text
//! ~/.local/share/sshub/
//! ├── state.toml              # known profiles + last used
//! └── profiles/<name>/        # one directory per profile
//!     ├── launcher.db
//!     ├── metadata.db
//!     ├── config.toml
//!     ├── credentials.json    # fallback when the OS keyring is unavailable
//!     ├── logs/
//!     └── tunnels/
//! ```
//!
//! When `SSHUB_DATA_DIR` / `SSHUB_CONFIG_DIR` (or their `SSH_LAUNCHER_*`
//! fallbacks) are set, profiles are bypassed entirely (compat mode): the
//! override directories are used verbatim, no `profiles/` nesting, no
//! `state.toml`. This keeps tests and scripted installs unchanged.

mod migrate;
pub mod picker;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_PROFILE_NAME: &str = "default";
pub const PROFILES_DIR: &str = "profiles";
pub const STATE_FILE: &str = "state.toml";

/// Upper bound for profile names — they double as directory names.
const MAX_NAME_LEN: usize = 64;

/// One profile as recorded in `state.toml`. The [`id`](Self::id) is stable
/// across renames and namespaces profile-owned keyring credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRecord {
    pub id: String,
    pub name: String,
}

/// Contents of the data-root `state.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileState {
    #[serde(default)]
    pub profiles: Vec<ProfileRecord>,
    /// Stable id of the profile launched last (drives the picker cursor).
    #[serde(default)]
    pub last_used: Option<String>,
}

impl ProfileState {
    pub fn state_path(data_root: &Path) -> PathBuf {
        data_root.join(STATE_FILE)
    }

    /// Load `state.toml`; `None` when the file does not exist yet.
    pub fn load(data_root: &Path) -> Result<Option<Self>> {
        let path = Self::state_path(data_root);
        if !path.exists() {
            return Ok(None);
        }
        let content =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let state: Self = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("invalid {}: {e}", path.display()))?;
        let mut names = HashSet::new();
        let mut ids = HashSet::new();
        for record in &state.profiles {
            validate_profile_name(&record.name)
                .with_context(|| format!("invalid profile name in {}", path.display()))?;
            anyhow::ensure!(
                names.insert(&record.name),
                "duplicate profile name in {}",
                path.display()
            );
            validate_profile_id(&record.id)
                .with_context(|| format!("invalid profile id in {}", path.display()))?;
            anyhow::ensure!(
                ids.insert(&record.id),
                "duplicate profile id in {}",
                path.display()
            );
        }
        Ok(Some(state))
    }

    /// Atomically persist to `<data_root>/state.toml` (tmp file + rename).
    pub fn save(&self, data_root: &Path) -> Result<()> {
        std::fs::create_dir_all(data_root)?;
        crate::secure_fs::restrict_dir(data_root);
        let path = Self::state_path(data_root);
        let content = toml::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("failed to serialize profile state: {e}"))?;
        let tmp = path.with_extension("toml.tmp");
        write_exclusive_private(&tmp, content.as_bytes())?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn by_name(&self, name: &str) -> Option<&ProfileRecord> {
        self.profiles.iter().find(|p| p.name == name)
    }

    pub fn by_id(&self, id: &str) -> Option<&ProfileRecord> {
        self.profiles.iter().find(|p| p.id == id)
    }

    pub fn last_used_record(&self) -> Option<&ProfileRecord> {
        self.last_used
            .as_deref()
            .and_then(|id| self.by_id(id))
            .or_else(|| self.profiles.first())
    }
}

fn write_exclusive_private(path: &Path, content: &[u8]) -> Result<()> {
    use std::io::Write;
    if path.exists() {
        anyhow::ensure!(
            !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
            "refusing symlink temporary file: {}",
            path.display()
        );
        std::fs::remove_file(path)?;
    }
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(content)?;
    file.sync_all()?;
    crate::secure_fs::restrict_file(path);
    Ok(())
}

/// Validate a profile name as a safe directory component.
pub fn validate_profile_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    anyhow::ensure!(!trimmed.is_empty(), "profile name must not be empty");
    anyhow::ensure!(
        trimmed.len() <= MAX_NAME_LEN,
        "profile name must be at most {MAX_NAME_LEN} characters"
    );
    anyhow::ensure!(
        trimmed != "." && trimmed != ".." && !trimmed.starts_with('.'),
        "reserved profile name"
    );
    anyhow::ensure!(
        !trimmed.contains('/') && !trimmed.contains('\\') && !trimmed.contains('\0'),
        "profile name must not contain path separators"
    );
    anyhow::ensure!(
        !trimmed.chars().any(char::is_control),
        "profile name must not contain control characters"
    );
    Ok(())
}

pub(super) fn validate_profile_id(id: &str) -> Result<()> {
    anyhow::ensure!(!id.is_empty() && id.len() <= 128, "invalid profile id");
    anyhow::ensure!(
        id != "."
            && id != ".."
            && !id.starts_with('.')
            && !id.contains('/')
            && !id.contains('\\')
            && !id.contains('\0')
            && !id.chars().any(char::is_control),
        "invalid profile id"
    );
    Ok(())
}

/// Generate a stable profile id: timestamp + pid + counter, hex-encoded.
/// Uniqueness matters because keyring credentials are namespaced by it.
pub fn new_profile_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let pid = std::process::id() as u64;
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("p{:012x}{:04x}", nanos ^ (pid << 32), seq & 0xffff)
}

/// Resolved data/config roots for this installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootDirs {
    /// Where `state.toml`, `profiles/`, and the databases live.
    pub data_root: PathBuf,
    /// Legacy config directory (only used for `config.toml` in compat mode).
    pub config_root: PathBuf,
    /// True when `SSHUB_DATA_DIR` / `SSHUB_CONFIG_DIR` (or legacy fallbacks)
    /// forced compatibility mode.
    pub compat: bool,
}

/// Resolve installation roots. Any directory override env var switches the
/// whole installation into compat mode (no profile discovery or migration).
pub fn resolve_roots() -> Result<RootDirs> {
    let data_override = env_dir("SSHUB_DATA_DIR").or_else(|| env_dir("SSH_LAUNCHER_DATA_DIR"));
    let config_override =
        env_dir("SSHUB_CONFIG_DIR").or_else(|| env_dir("SSH_LAUNCHER_CONFIG_DIR"));
    let compat = data_override.is_some() || config_override.is_some();
    let data_root = match data_override {
        Some(dir) => dir,
        None => crate::config::data_dir()?,
    };
    let config_root = match config_override {
        Some(dir) => dir,
        None => crate::config::config_dir()?,
    };
    Ok(RootDirs {
        data_root,
        config_root,
        compat,
    })
}

fn env_dir(var: &str) -> Option<PathBuf> {
    match std::env::var(var) {
        Ok(dir) if !dir.trim().is_empty() => Some(dir.into()),
        _ => None,
    }
}

/// Every resolved path a profile-owned subsystem needs. Built once at startup
/// and passed down — subsystems must not rediscover paths from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfilePaths {
    /// Installation data root containing `state.toml` and `profiles/`.
    pub data_root: PathBuf,
    /// Stable profile id (namespaces keyring credentials).
    pub id: String,
    pub name: String,
    /// Profile directory; holds the databases, logs, tunnels, and (in profile
    /// mode) `config.toml`.
    pub root: PathBuf,
    /// `config.toml` location. In profile mode this is `root/config.toml`; in
    /// compat mode it stays in the (possibly overridden) config directory.
    pub config_file: PathBuf,
    /// Resolved SSH config source for this profile.
    pub ssh_config: PathBuf,
    /// True when directory overrides bypassed profile discovery.
    pub compat: bool,
}

impl ProfilePaths {
    pub fn launcher_db(&self) -> PathBuf {
        self.root.join("launcher.db")
    }

    pub fn metadata_db(&self) -> PathBuf {
        self.root.join("metadata.db")
    }

    pub fn credentials_file(&self) -> PathBuf {
        self.root.join("credentials.json")
    }

    /// Base directory for `session_log` (which appends `logs/` itself).
    pub fn session_log_base(&self) -> &Path {
        &self.root
    }

    /// Base directory for tunnel runtime state (`ensure_tunnel_pid_dir`
    /// appends `tunnels/` itself).
    pub fn tunnel_base(&self) -> &Path {
        &self.root
    }

    pub fn profiles_dir(data_root: &Path) -> PathBuf {
        data_root.join(PROFILES_DIR)
    }

    pub fn profile_dir(data_root: &Path, name: &str) -> PathBuf {
        Self::profiles_dir(data_root).join(name)
    }

    /// Prefix applied to every credential key this profile stores. Empty in
    /// compat mode so existing installs keep their current keys.
    pub fn credential_prefix(&self) -> String {
        if self.compat {
            String::new()
        } else {
            format!("profile:{}:", self.id)
        }
    }
}

/// Paths for the synthetic compat-mode profile (no `profiles/` nesting).
pub fn compat_paths(roots: &RootDirs, ssh_config: PathBuf) -> ProfilePaths {
    ProfilePaths {
        data_root: roots.data_root.clone(),
        id: DEFAULT_PROFILE_NAME.to_string(),
        name: DEFAULT_PROFILE_NAME.to_string(),
        root: roots.data_root.clone(),
        config_file: roots.config_root.join("config.toml"),
        ssh_config,
        compat: true,
    }
}

/// Paths for a profile recorded in `state.toml`.
pub fn profile_paths(
    roots: &RootDirs,
    record: &ProfileRecord,
    ssh_config: PathBuf,
) -> ProfilePaths {
    let root = ProfilePaths::profile_dir(&roots.data_root, &record.name);
    ProfilePaths {
        data_root: roots.data_root.clone(),
        id: record.id.clone(),
        name: record.name.clone(),
        config_file: root.join("config.toml"),
        root,
        ssh_config,
        compat: false,
    }
}

pub(crate) fn require_profile_dir(roots: &RootDirs, record: &ProfileRecord) -> Result<()> {
    let path = ProfilePaths::profile_dir(&roots.data_root, &record.name);
    let metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("profile directory missing: {}", path.display()))?;
    anyhow::ensure!(
        metadata.file_type().is_dir(),
        "profile path is not a directory: {}",
        path.display()
    );
    Ok(())
}

/// Profile selection request parsed from global CLI flags.
#[derive(Debug, Clone, Default)]
pub struct StartupOptions {
    /// `--profile NAME` — bypass the picker.
    pub profile: Option<String>,
    /// `--manage-profiles` — open the picker even with a single profile.
    pub manage_profiles: bool,
}

/// Extract global profile flags (`--profile NAME` / `--profile=NAME` /
/// `--manage-profiles`) from argv, returning the parsed options and the
/// remaining arguments in order. Runs before subcommand dispatch so the flags
/// work for both the TUI and headless commands.
pub fn extract_startup_flags(args: Vec<String>) -> Result<(StartupOptions, Vec<String>)> {
    let mut opts = StartupOptions::default();
    let mut rest = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--profile" {
            let value = args
                .get(i + 1)
                .ok_or_else(|| anyhow::anyhow!("--profile requires a value"))?;
            anyhow::ensure!(opts.profile.is_none(), "--profile given more than once");
            opts.profile = Some(value.clone());
            i += 2;
        } else if let Some(value) = arg.strip_prefix("--profile=") {
            anyhow::ensure!(opts.profile.is_none(), "--profile given more than once");
            anyhow::ensure!(!value.trim().is_empty(), "--profile requires a value");
            opts.profile = Some(value.trim().to_string());
            i += 1;
        } else if arg == "--manage-profiles" {
            opts.manage_profiles = true;
            i += 1;
        } else {
            rest.push(arg.clone());
            i += 1;
        }
    }
    anyhow::ensure!(
        !(opts.profile.is_some() && opts.manage_profiles),
        "--profile cannot be combined with --manage-profiles"
    );
    Ok((opts, rest))
}

/// Outcome of startup resolution.
#[derive(Debug)]
pub enum Startup {
    /// Profile resolved without user interaction.
    Silent(ProfilePaths),
    /// More than one profile (or `--manage-profiles`): show the picker.
    Picker {
        roots: RootDirs,
        state: ProfileState,
    },
}

/// Resolve which profile to launch, or whether the picker must run first.
///
/// `interactive` is false when there is no terminal (headless CI smoke): the
/// picker cannot render, so the last-used profile is selected silently.
pub fn resolve_startup(opts: &StartupOptions, interactive: bool) -> Result<Startup> {
    let roots = resolve_roots()?;
    resolve_startup_at(opts, interactive, roots)
}

fn resolve_startup_at(
    opts: &StartupOptions,
    interactive: bool,
    roots: RootDirs,
) -> Result<Startup> {
    if roots.compat {
        anyhow::ensure!(
            opts.profile.is_none(),
            "--profile is unavailable when SSHUB_DATA_DIR/SSHUB_CONFIG_DIR overrides are set"
        );
        let ssh_config = crate::ssh::ssh_config_path()?;
        return Ok(Startup::Silent(compat_paths(&roots, ssh_config)));
    }

    let state = ensure_layout(&roots)?;

    if let Some(requested) = &opts.profile {
        let record = state.by_name(requested).ok_or_else(|| {
            let available: Vec<&str> = state.profiles.iter().map(|p| p.name.as_str()).collect();
            anyhow::anyhow!(
                "unknown profile '{requested}' (available: {})",
                if available.is_empty() {
                    "none".to_string()
                } else {
                    available.join(", ")
                }
            )
        })?;
        require_profile_dir(&roots, record)?;
        let ssh_config = ssh_config_path_for_profile(&roots, record)?;
        let paths = profile_paths(&roots, record, ssh_config);
        return Ok(Startup::Silent(paths));
    }

    match state.profiles.len() {
        0 => {
            // ensure_layout always creates at least one profile; guard anyway.
            anyhow::bail!("no profiles found in {}", roots.data_root.display());
        }
        1 if !opts.manage_profiles => {
            let record = state
                .profiles
                .first()
                .expect("len == 1 guarantees a first profile");
            require_profile_dir(&roots, record)?;
            let ssh_config = ssh_config_path_for_profile(&roots, record)?;
            Ok(Startup::Silent(profile_paths(&roots, record, ssh_config)))
        }
        _ => {
            if interactive {
                Ok(Startup::Picker { roots, state })
            } else {
                let record = state
                    .last_used_record()
                    .expect("non-empty profile list guarantees a record");
                require_profile_dir(&roots, record)?;
                let ssh_config = ssh_config_path_for_profile(&roots, record)?;
                Ok(Startup::Silent(profile_paths(&roots, record, ssh_config)))
            }
        }
    }
}

/// Resolve the SSH config source for a profile: env override, then the
/// profile's own `config.toml` (`[ssh] config_path`), then `~/.ssh/config`.
pub fn ssh_config_path_for_profile(roots: &RootDirs, record: &ProfileRecord) -> Result<PathBuf> {
    if let Ok(path) = std::env::var("SSHUB_SSH_CONFIG") {
        if !path.trim().is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    if let Ok(path) = std::env::var("SSH_LAUNCHER_SSH_CONFIG") {
        if !path.trim().is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    let config_file = ProfilePaths::profile_dir(&roots.data_root, &record.name).join("config.toml");
    if let Ok(content) = std::fs::read_to_string(&config_file) {
        if let Ok(config) = toml::from_str::<crate::config::AppConfig>(&content) {
            if let Some(path) = config.ssh.config_path {
                if !path.trim().is_empty() {
                    return Ok(crate::ssh::expand_tilde(path.trim()));
                }
            }
        }
    }
    Ok(crate::ssh::expand_tilde("~/.ssh/config"))
}

/// Read selected profile's motion preference before the startup picker runs.
/// Picker itself remains after splash; this preserves startup ordering while
/// honoring reduced-motion preference for last-used profile.
pub fn picker_animation_enabled(roots: &RootDirs, state: &ProfileState) -> bool {
    let Some(record) = state.last_used_record() else {
        return true;
    };
    let path = ProfilePaths::profile_dir(&roots.data_root, &record.name).join("config.toml");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| toml::from_str::<crate::config::AppConfig>(&content).ok())
        .map(|config| !config.appearance.disable_animation)
        .unwrap_or(true)
}

/// Ensure the profile layout exists under `roots`: migrate a legacy install,
/// adopt stray profile directories, or create the first `default` profile.
/// Returns the loaded (or freshly written) state.
pub fn ensure_layout(roots: &RootDirs) -> Result<ProfileState> {
    std::fs::create_dir_all(&roots.data_root)
        .with_context(|| format!("create {}", roots.data_root.display()))?;
    crate::secure_fs::restrict_dir(&roots.data_root);
    let profiles_dir = ProfilePaths::profiles_dir(&roots.data_root);
    if profiles_dir.exists() {
        let metadata = std::fs::symlink_metadata(&profiles_dir)?;
        anyhow::ensure!(
            metadata.file_type().is_dir(),
            "profiles path is not a directory: {}",
            profiles_dir.display()
        );
    } else {
        std::fs::create_dir(&profiles_dir)?;
        crate::secure_fs::restrict_dir(&profiles_dir);
    }

    if let Some(state) = ProfileState::load(&roots.data_root)? {
        sweep_deleting_profiles(&profiles_dir, Some(&state));
        return Ok(state);
    }

    // Retry legacy migration before adopting profile directories. This keeps a
    // crash-created staging directory from becoming a visible profile.
    if migrate::legacy_installation_present(roots) {
        migrate::run_legacy_migration(roots)?;
        if let Some(state) = ProfileState::load(&roots.data_root)? {
            sweep_deleting_profiles(&profiles_dir, Some(&state));
            return Ok(state);
        }
    }

    // No state.toml yet. A `profiles/` directory may already hold profiles
    // (e.g. a crash after final rename but before state write) — adopt them.
    if profiles_dir.exists() {
        sweep_deleting_profiles(&profiles_dir, None);
        let adopted = adopt_existing_profiles(roots)?;
        if !adopted.profiles.is_empty() {
            adopted.save(&roots.data_root)?;
            return Ok(adopted);
        }
    }

    // Fresh install: create the default profile.
    let mut state = ProfileState::default();
    let record = create_profile_dirs(roots, &mut state, DEFAULT_PROFILE_NAME)?;
    state.last_used = Some(record.id.clone());
    state.save(&roots.data_root)?;
    Ok(state)
}

fn sweep_deleting_profiles(profiles_dir: &Path, state: Option<&ProfileState>) {
    let Ok(entries) = std::fs::read_dir(profiles_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_prefix(".deleting-") else {
            continue;
        };
        if let Some(state) = state {
            if let Some(record) = state.by_id(id) {
                let target = profiles_dir.join(&record.name);
                if !target.exists() {
                    let _ = std::fs::rename(entry.path(), target);
                } else {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
                continue;
            }
        }
        let _ = std::fs::remove_dir_all(entry.path());
    }
}

/// Build state from profile directories that exist without a `state.toml`.
fn adopt_existing_profiles(roots: &RootDirs) -> Result<ProfileState> {
    let profiles_dir = ProfilePaths::profiles_dir(&roots.data_root);
    let mut state = ProfileState::default();
    let mut entries: Vec<_> = std::fs::read_dir(&profiles_dir)
        .with_context(|| format!("read {}", profiles_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || validate_profile_name(&name).is_err() {
            continue;
        }
        let id = migrate::read_profile_id(&entry.path()).unwrap_or_else(new_profile_id);
        state.profiles.push(ProfileRecord { id, name });
    }
    if let Some(first) = state.profiles.first() {
        state.last_used = Some(first.id.clone());
    }
    Ok(state)
}

/// Create a new profile: directory, default `config.toml`, state update.
/// Returns the updated state and the new record.
pub fn create_profile(
    roots: &RootDirs,
    state: &mut ProfileState,
    name: &str,
) -> Result<ProfileRecord> {
    let name = name.trim().to_string();
    validate_profile_name(&name)?;
    anyhow::ensure!(
        state.by_name(&name).is_none(),
        "profile '{name}' already exists"
    );
    let original_state = state.clone();
    let record = create_profile_dirs(roots, state, &name)?;
    if let Err(save_err) = state.save(&roots.data_root) {
        *state = original_state.clone();
        let dir = ProfilePaths::profile_dir(&roots.data_root, &name);
        let _ = std::fs::remove_dir_all(&dir);
        return Err(save_err).context("save profile state after creation");
    }
    Ok(record)
}

/// Directory + default config for a new profile, plus the state entry
/// (without persisting — callers decide when `state.save` happens).
fn create_profile_dirs(
    roots: &RootDirs,
    state: &mut ProfileState,
    name: &str,
) -> Result<ProfileRecord> {
    let dir = ProfilePaths::profile_dir(&roots.data_root, name);
    anyhow::ensure!(
        !dir.exists(),
        "profile directory already exists: {}",
        dir.display()
    );
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create profile directory {}", dir.display()))?;
    crate::secure_fs::restrict_dir(&dir);
    let record = ProfileRecord {
        id: new_profile_id(),
        name: name.to_string(),
    };
    if let Err(err) = write_profile_setup(&dir, &record.id) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(err);
    }
    state.profiles.push(record.clone());
    Ok(record)
}

fn write_profile_setup(dir: &Path, id: &str) -> Result<()> {
    migrate::write_profile_id(dir, id)?;
    write_default_profile_config(dir)
}

/// Seed a fresh profile directory with the default `config.toml`.
fn write_default_profile_config(dir: &Path) -> Result<()> {
    let path = dir.join("config.toml");
    if path.exists() {
        return Ok(());
    }
    let content = toml::to_string_pretty(&crate::config::AppConfig::default())
        .map_err(|e| anyhow::anyhow!("failed to serialize default config: {e}"))?;
    std::fs::write(&path, content)?;
    crate::secure_fs::restrict_file(&path);
    Ok(())
}

/// Rename a profile: move the directory, update state atomically, preserve
/// the stable id. Fails without side effects if the destination exists.
pub fn rename_profile(
    roots: &RootDirs,
    state: &mut ProfileState,
    id: &str,
    new_name: &str,
) -> Result<()> {
    let new_name = new_name.trim().to_string();
    validate_profile_name(&new_name)?;
    anyhow::ensure!(
        state.by_name(&new_name).is_none(),
        "profile '{new_name}' already exists"
    );
    let idx = state
        .profiles
        .iter()
        .position(|p| p.id == id)
        .with_context(|| format!("profile id '{id}' not found"))?;
    let old_name = state.profiles[idx].name.clone();
    let src = ProfilePaths::profile_dir(&roots.data_root, &old_name);
    let dst = ProfilePaths::profile_dir(&roots.data_root, &new_name);
    let src_meta = std::fs::symlink_metadata(&src)
        .with_context(|| format!("profile directory missing: {}", src.display()))?;
    anyhow::ensure!(
        src_meta.file_type().is_dir(),
        "profile path is not a directory: {}",
        src.display()
    );
    anyhow::ensure!(
        !dst.exists(),
        "profile directory already exists: {}",
        dst.display()
    );
    std::fs::rename(&src, &dst)
        .with_context(|| format!("rename {} -> {}", src.display(), dst.display()))?;
    let original_state = state.clone();
    state.profiles[idx].name = new_name;
    if let Err(save_err) = state.save(&roots.data_root) {
        *state = original_state;
        if let Err(rollback_err) = std::fs::rename(&dst, &src) {
            anyhow::bail!(
                "save renamed profile state failed: {save_err}; rollback failed: {rollback_err}"
            );
        }
        return Err(save_err).context("save profile state after rename");
    }
    Ok(())
}

/// Delete a profile: directory, state entry, and profile-owned keyring
/// credentials. Refuses to delete the final profile. Never touches external
/// SSH config.
pub fn delete_profile(roots: &RootDirs, state: &mut ProfileState, id: &str) -> Result<()> {
    let store = crate::credentials::OsKeyring;
    delete_profile_with_store(roots, state, id, &store)
}

pub fn delete_profile_with_store(
    roots: &RootDirs,
    state: &mut ProfileState,
    id: &str,
    credential_store: &dyn crate::credentials::PasswordStore,
) -> Result<()> {
    anyhow::ensure!(
        state.profiles.len() > 1,
        "cannot delete the last remaining profile"
    );
    let idx = state
        .profiles
        .iter()
        .position(|p| p.id == id)
        .with_context(|| format!("profile id '{id}' not found"))?;
    let record = state.profiles[idx].clone();
    let dir = ProfilePaths::profile_dir(&roots.data_root, &record.name);
    if dir.exists() {
        let metadata = std::fs::symlink_metadata(&dir)?;
        anyhow::ensure!(
            metadata.file_type().is_dir(),
            "profile path is not a directory: {}",
            dir.display()
        );
    }

    stop_profile_tunnels(&dir)?;

    let trash =
        ProfilePaths::profiles_dir(&roots.data_root).join(format!(".deleting-{}", record.id));
    anyhow::ensure!(!trash.exists(), "profile deletion already in progress");
    if dir.exists() {
        std::fs::rename(&dir, &trash)
            .with_context(|| format!("stage profile directory {}", dir.display()))?;
    }
    let original_state = state.clone();
    state.profiles.remove(idx);
    if state.last_used.as_deref() == Some(id) {
        state.last_used = state.profiles.first().map(|p| p.id.clone());
    }
    if let Err(save_err) = state.save(&roots.data_root) {
        *state = original_state.clone();
        if let Err(rollback_err) = std::fs::rename(&trash, &dir) {
            anyhow::bail!(
                "save profile deletion state failed: {save_err}; rollback failed: {rollback_err}"
            );
        }
        return Err(save_err).context("save profile state after deletion");
    }

    // Capture profile-owned key names while the database is still available.
    // Delete keyring entries only after filesystem deletion commits, so a
    // failed delete can restore the complete profile.
    let credential_keys = profile_credential_keys(&trash, &record.id);
    if trash.exists() {
        if let Err(remove_err) = std::fs::remove_dir_all(&trash) {
            *state = original_state.clone();
            if let Err(restore_state_err) = state.save(&roots.data_root) {
                anyhow::bail!(
                    "remove staged profile directory failed: {remove_err}; restore state failed: {restore_state_err}"
                );
            }
            if let Err(restore_dir_err) = std::fs::rename(&trash, &dir) {
                anyhow::bail!(
                    "remove staged profile directory failed: {remove_err}; restore directory failed: {restore_dir_err}"
                );
            }
            return Err(remove_err).context("remove staged profile directory");
        }
    }
    purge_profile_credential_keys(&credential_keys, credential_store);
    Ok(())
}

fn stop_profile_tunnels(dir: &Path) -> Result<()> {
    let db_path = dir.join("launcher.db");
    if !db_path.exists() {
        return Ok(());
    }
    let store = crate::store::LauncherStore::open(&db_path)?;
    for tunnel in store.list_tunnels()? {
        crate::tunnel::stop_detached_tunnel(dir, tunnel.id)?;
    }
    Ok(())
}

/// Best-effort removal of keyring entries namespaced to a deleted profile.
fn profile_credential_keys(dir: &Path, profile_id: &str) -> Vec<String> {
    let db_path = dir.join("launcher.db");
    if !db_path.exists() {
        return Vec::new();
    }
    let conn = match rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(conn) => conn,
        Err(_) => return Vec::new(),
    };
    let prefix = format!("profile:{profile_id}:");
    let mut keys = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id FROM hosts") {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, i64>(0)) {
            for id in rows.flatten() {
                keys.push(format!("{prefix}host:{id}"));
            }
        }
    }
    if let Ok(mut stmt) = conn.prepare("SELECT id FROM identities") {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, i64>(0)) {
            for id in rows.flatten() {
                keys.push(format!("{prefix}identity:{id}"));
            }
        }
    }
    keys
}

fn purge_profile_credential_keys(keys: &[String], store: &dyn crate::credentials::PasswordStore) {
    for key in keys {
        let _ = store.delete(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(dir: &Path) -> RootDirs {
        RootDirs {
            data_root: dir.to_path_buf(),
            config_root: dir.join("config"),
            compat: false,
        }
    }

    #[test]
    fn profile_names_reject_path_components() {
        for name in ["", " ", ".", "..", ".hidden", "a/b", r"a\b", "a\0b"] {
            assert!(validate_profile_name(name).is_err(), "accepted {name:?}");
        }
        assert!(validate_profile_name("work-laptop").is_ok());
    }

    #[test]
    fn state_roundtrip_preserves_last_used_id() {
        let dir = tempfile::tempdir().unwrap();
        let state = ProfileState {
            profiles: vec![ProfileRecord {
                id: "stable-id".into(),
                name: "work".into(),
            }],
            last_used: Some("stable-id".into()),
        };
        state.save(dir.path()).unwrap();
        assert_eq!(ProfileState::load(dir.path()).unwrap(), Some(state));
    }

    #[test]
    fn profile_paths_isolate_owned_resources_and_preserve_id_across_rename() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let mut state = ProfileState::default();
        let record = create_profile(&roots, &mut state, "work").unwrap();
        let old_paths = profile_paths(&roots, &record, PathBuf::from("/ssh/work"));
        let old_id = record.id.clone();

        rename_profile(&roots, &mut state, &record.id, "personal").unwrap();
        let renamed = state.by_name("personal").unwrap();
        let new_paths = profile_paths(&roots, renamed, PathBuf::from("/ssh/personal"));

        assert_eq!(renamed.id, old_id);
        assert_ne!(old_paths.launcher_db(), new_paths.launcher_db());
        assert_ne!(old_paths.credentials_file(), new_paths.credentials_file());
        assert_eq!(old_paths.credential_prefix(), new_paths.credential_prefix());
        assert!(new_paths.root.join("config.toml").exists());
    }

    #[test]
    fn fresh_layout_creates_default_profile() {
        let dir = tempfile::tempdir().unwrap();
        let state = ensure_layout(&roots(dir.path())).unwrap();
        let record = state.by_name(DEFAULT_PROFILE_NAME).unwrap();
        assert_eq!(state.last_used.as_deref(), Some(record.id.as_str()));
        assert!(ProfilePaths::profile_dir(dir.path(), DEFAULT_PROFILE_NAME)
            .join("config.toml")
            .exists());
    }

    #[test]
    fn picker_animation_reads_last_used_profile_preference() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let state = ensure_layout(&roots).unwrap();
        let record = state.last_used_record().unwrap().clone();
        std::fs::write(
            ProfilePaths::profile_dir(dir.path(), &record.name).join("config.toml"),
            "[appearance]\ndisable_animation = true\n",
        )
        .unwrap();
        assert!(!picker_animation_enabled(&roots, &state));
    }

    #[test]
    fn state_load_rejects_traversal_profile_names() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(STATE_FILE),
            "[[profiles]]\nid = \"id\"\nname = \"../escape\"\n",
        )
        .unwrap();
        assert!(ProfileState::load(dir.path()).is_err());

        std::fs::write(
            dir.path().join(STATE_FILE),
            "[[profiles]]\nid = \"../escape\"\nname = \"safe\"\n",
        )
        .unwrap();
        assert!(ProfileState::load(dir.path()).is_err());
    }

    #[test]
    fn startup_flags_support_separate_and_equals_forms() {
        let (opts, rest) = extract_startup_flags(vec![
            "--profile".into(),
            "work".into(),
            "host".into(),
            "list".into(),
        ])
        .unwrap();
        assert_eq!(opts.profile.as_deref(), Some("work"));
        assert!(!opts.manage_profiles);
        assert_eq!(rest, ["host", "list"]);

        let (opts, rest) =
            extract_startup_flags(vec!["--profile=personal".into(), "list".into()]).unwrap();
        assert_eq!(opts.profile.as_deref(), Some("personal"));
        assert_eq!(rest, ["list"]);
        assert!(extract_startup_flags(vec![
            "--profile".into(),
            "work".into(),
            "--manage-profiles".into(),
        ])
        .is_err());
    }

    #[test]
    fn profile_selection_resolves_before_database_open() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let mut state = ensure_layout(&roots).unwrap();
        let record = create_profile(&roots, &mut state, "work").unwrap();
        let opts = StartupOptions {
            profile: Some("work".into()),
            manage_profiles: false,
        };

        let startup = resolve_startup_at(&opts, false, roots.clone()).unwrap();
        let Startup::Silent(paths) = startup else {
            panic!("explicit profile must resolve silently");
        };
        assert_eq!(paths.data_root, roots.data_root);
        assert_eq!(paths.id, record.id);
        assert_eq!(
            paths.launcher_db(),
            roots.data_root.join("profiles/work/launcher.db")
        );
        assert!(!paths.launcher_db().exists());

        let unknown = StartupOptions {
            profile: Some("missing".into()),
            manage_profiles: false,
        };
        assert!(resolve_startup_at(&unknown, false, roots.clone()).is_err());
        assert!(!roots
            .data_root
            .join("profiles/missing/launcher.db")
            .exists());
    }

    #[test]
    fn compatibility_mode_rejects_profile_selection() {
        let dir = tempfile::tempdir().unwrap();
        let opts = StartupOptions {
            profile: Some("work".into()),
            manage_profiles: false,
        };
        let roots = RootDirs {
            data_root: dir.path().join("data"),
            config_root: dir.path().join("config"),
            compat: true,
        };
        assert!(resolve_startup_at(&opts, false, roots).is_err());
    }

    #[test]
    fn legacy_layout_migrates_without_removing_source_files() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        std::fs::write(dir.path().join("launcher.db"), b"legacy database").unwrap();
        std::fs::create_dir_all(&roots.config_root).unwrap();
        std::fs::write(roots.config_root.join("config.toml"), "[appearance]\n").unwrap();
        std::fs::create_dir_all(dir.path().join("profiles/.staging-default")).unwrap();

        let state = ensure_layout(&roots).unwrap();
        assert_eq!(state.profiles.len(), 1);
        assert!(ProfilePaths::profile_dir(dir.path(), DEFAULT_PROFILE_NAME)
            .join("launcher.db")
            .exists());
        assert!(dir.path().join("launcher.db").exists());
        assert!(ProfilePaths::profile_dir(dir.path(), DEFAULT_PROFILE_NAME)
            .join("config.toml")
            .exists());
    }
}
