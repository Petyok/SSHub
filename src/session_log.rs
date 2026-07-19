//! PTY session transcript logging to plain-text files under the data directory.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::secure_fs;

/// Per-host override for session logging (tri-state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionLoggingOverride {
    #[default]
    Inherit,
    On,
    Off,
}

impl SessionLoggingOverride {
    pub fn label(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::On => "on",
            Self::Off => "off",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Inherit => Self::On,
            Self::On => Self::Off,
            Self::Off => Self::Inherit,
        }
    }

    pub fn from_db(value: Option<i64>) -> Self {
        match value {
            None => Self::Inherit,
            Some(0) => Self::Off,
            Some(1) => Self::On,
            _ => Self::Inherit,
        }
    }

    pub fn to_db(self) -> Option<i64> {
        match self {
            Self::Inherit => None,
            Self::On => Some(1),
            Self::Off => Some(0),
        }
    }
}

/// Resolve whether logging is active for a connect attempt.
pub fn effective_enabled(global: bool, host_override: SessionLoggingOverride) -> bool {
    match host_override {
        SessionLoggingOverride::Inherit => global,
        SessionLoggingOverride::On => true,
        SessionLoggingOverride::Off => false,
    }
}

/// Build the per-host log directory segment under `logs/`.
///
/// Managed hosts include a stable id so colliding sanitized names (e.g.
/// `web/prod` and `web_prod` both → `web_prod`) stay in separate dirs.
pub fn host_log_dir_name(host_name: &str, host_id: Option<i64>) -> String {
    let sanitized = sanitize_host_dir(host_name);
    match host_id {
        Some(id) => format!("{sanitized}-{id}"),
        None => sanitized,
    }
}

/// Sanitize a host name for use as a single path segment.
pub fn sanitize_host_dir(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "_unknown".to_string();
    }
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out == "." || out == ".." {
        "_unknown".to_string()
    } else {
        out
    }
}

fn timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

static OPEN_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_log_filename(serial: u32) -> String {
    let secs = timestamp_secs();
    let pid = std::process::id();
    let open_id = OPEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    if serial == 0 {
        format!("{secs}-{pid}-{open_id}.log")
    } else {
        format!("{secs}-{pid}-{open_id}-{serial}.log")
    }
}

fn create_unique_log_file(host_dir: &Path, serial: u32) -> Result<(PathBuf, File)> {
    let base_name = unique_log_filename(serial);
    for attempt in 0..100u32 {
        let name = if attempt == 0 {
            base_name.clone()
        } else {
            format!("{}-{}.log", base_name.trim_end_matches(".log"), attempt)
        };
        let path = host_dir.join(&name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                secure_fs::restrict_file(&path);
                return Ok((path, file));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(e).with_context(|| format!("create session log {}", path.display()));
            }
        }
    }
    anyhow::bail!(
        "could not allocate unique session log filename in {}",
        host_dir.display()
    )
}

/// Append-only session log writer with size-based rotation and retention pruning.
pub struct SessionLogWriter {
    host_dir: PathBuf,
    current_path: PathBuf,
    writer: BufWriter<File>,
    bytes_written: u64,
    max_file_bytes: u64,
    retention_files: usize,
    file_serial: u32,
    closed: bool,
}

impl SessionLogWriter {
    pub fn open(
        base_dir: impl AsRef<Path>,
        host_name: &str,
        host_id: Option<i64>,
        max_file_bytes: u64,
        retention_files: usize,
    ) -> Result<Self> {
        let base_dir = base_dir.as_ref().to_path_buf();
        let logs_root = base_dir.join("logs");
        fs::create_dir_all(&logs_root)
            .with_context(|| format!("create session logs dir {}", logs_root.display()))?;
        secure_fs::restrict_dir(&logs_root);

        let host_dir = logs_root.join(host_log_dir_name(host_name, host_id));
        fs::create_dir_all(&host_dir)
            .with_context(|| format!("create host log dir {}", host_dir.display()))?;
        secure_fs::restrict_dir(&host_dir);

        let (current_path, file) = create_unique_log_file(&host_dir, 0)?;

        Ok(Self {
            host_dir,
            current_path,
            writer: BufWriter::new(file),
            bytes_written: 0,
            max_file_bytes,
            retention_files,
            file_serial: 0,
            closed: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.current_path
    }

    /// Directory holding this host's rotated log segments.
    pub fn host_dir(&self) -> &Path {
        &self.host_dir
    }

    pub fn append(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.writer.write_all(bytes)?;
        self.bytes_written += bytes.len() as u64;
        if self.bytes_written >= self.max_file_bytes {
            self.rotate()?;
        }
        Ok(())
    }

    fn rotate(&mut self) -> Result<()> {
        self.writer.flush()?;

        self.file_serial += 1;
        let (new_path, file) = create_unique_log_file(&self.host_dir, self.file_serial)?;
        self.current_path = new_path;
        self.writer = BufWriter::new(file);
        self.bytes_written = 0;
        prune_retention(&self.host_dir, self.retention_files)?;
        Ok(())
    }

    pub fn close(mut self) -> Result<()> {
        self.writer.flush()?;
        prune_retention(&self.host_dir, self.retention_files)?;
        self.closed = true;
        Ok(())
    }
}

impl Drop for SessionLogWriter {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let _ = self.writer.flush();
        let _ = prune_retention(&self.host_dir, self.retention_files);
    }
}

/// Allocate a fresh log file path for external session capture (e.g. `script(1)`).
pub fn allocate_log_path(
    base_dir: impl AsRef<Path>,
    host_name: &str,
    host_id: Option<i64>,
) -> Result<PathBuf> {
    let base_dir = base_dir.as_ref().to_path_buf();
    let logs_root = base_dir.join("logs");
    fs::create_dir_all(&logs_root)
        .with_context(|| format!("create session logs dir {}", logs_root.display()))?;
    secure_fs::restrict_dir(&logs_root);

    let host_dir = logs_root.join(host_log_dir_name(host_name, host_id));
    fs::create_dir_all(&host_dir)
        .with_context(|| format!("create host log dir {}", host_dir.display()))?;
    secure_fs::restrict_dir(&host_dir);

    let (path, _file) = create_unique_log_file(&host_dir, 0)?;
    Ok(path)
}

/// Wrap `inner_argv` in a platform `script(1)` invocation for session logging.
///
/// Returns `None` when `script` is unavailable or the OS has no supported wrapper.
pub fn wrap_script_command(log_path: &Path, inner_argv: &[String]) -> Option<Vec<String>> {
    if inner_argv.is_empty() {
        return None;
    }
    script_binary()?;

    let log = log_path.to_string_lossy().into_owned();

    #[cfg(target_os = "linux")]
    {
        let inner = shell_join(inner_argv);
        Some(vec![
            "script".into(),
            "-q".into(),
            "-a".into(),
            log,
            "-c".into(),
            inner,
        ])
    }

    #[cfg(target_os = "macos")]
    {
        let mut argv = vec!["script".into(), "-q".into(), log];
        argv.extend(inner_argv.iter().cloned());
        return Some(argv);
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = log;
        None
    }
}

fn script_binary() -> Option<&'static str> {
    if std::process::Command::new("which")
        .arg("script")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        Some("script")
    } else {
        None
    }
}

fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/')
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn prune_retention(host_dir: &Path, retention_files: usize) -> Result<()> {
    if retention_files == 0 {
        return Ok(());
    }
    let mut logs: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(host_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "log"))
        .filter_map(|e| {
            let mtime = e.metadata().ok()?.modified().ok()?;
            Some((e.path(), mtime))
        })
        .collect();
    if logs.len() <= retention_files {
        return Ok(());
    }
    logs.sort_by_key(|(_, mtime)| *mtime);
    let excess = logs.len().saturating_sub(retention_files);
    for (path, _) in logs.into_iter().take(excess) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_log_dir_name_distinct_for_colliding_sanitized_names() {
        assert_eq!(host_log_dir_name("web/prod", Some(1)), "web_prod-1");
        assert_eq!(host_log_dir_name("web_prod", Some(2)), "web_prod-2");
        assert_ne!(
            host_log_dir_name("web/prod", Some(1)),
            host_log_dir_name("web_prod", Some(2))
        );
        assert_eq!(host_log_dir_name("legacy", None), "legacy");
    }

    #[test]
    fn sanitize_host_dir_replaces_unsafe_chars() {
        assert_eq!(sanitize_host_dir("web/prod"), "web_prod");
        assert_eq!(sanitize_host_dir("  my-host  "), "my-host");
        assert_eq!(sanitize_host_dir(".."), "_unknown");
    }

    #[test]
    fn effective_enabled_respects_override() {
        assert!(effective_enabled(true, SessionLoggingOverride::Inherit));
        assert!(!effective_enabled(false, SessionLoggingOverride::Inherit));
        assert!(effective_enabled(false, SessionLoggingOverride::On));
        assert!(!effective_enabled(true, SessionLoggingOverride::Off));
    }

    #[test]
    fn override_db_roundtrip() {
        assert_eq!(
            SessionLoggingOverride::from_db(SessionLoggingOverride::On.to_db()),
            SessionLoggingOverride::On
        );
        assert_eq!(
            SessionLoggingOverride::from_db(SessionLoggingOverride::Inherit.to_db()),
            SessionLoggingOverride::Inherit
        );
    }

    #[test]
    fn writer_appends_and_prunes_old_logs() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let mut w = SessionLogWriter::open(tmp.path(), "web", None, 1024, 2)?;
        w.append(b"hello")?;
        let first = w.path().to_path_buf();
        w.close()?;

        // Seed two stale files so retention keeps only the newest two.
        let host_dir = tmp.path().join("logs").join("web");
        fs::write(host_dir.join("old1.log"), b"x")?;
        fs::write(host_dir.join("old2.log"), b"x")?;
        std::thread::sleep(std::time::Duration::from_millis(5));

        let mut w2 = SessionLogWriter::open(tmp.path(), "web", None, 1024, 2)?;
        w2.append(b"world")?;
        w2.close()?;

        let count = fs::read_dir(&host_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "log"))
            .count();
        assert!(count <= 2, "retention should cap at 2 files");
        assert!(first.exists() || count >= 1);
        Ok(())
    }

    #[test]
    fn rotate_on_size_limit() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let mut w = SessionLogWriter::open(tmp.path(), "db", None, 8, 10)?;
        let first = w.path().to_path_buf();
        w.append(b"12345678")?;
        w.append(b"X")?;
        let second = w.path().to_path_buf();
        assert_ne!(first, second);
        w.close()?;
        Ok(())
    }

    #[test]
    fn concurrent_opens_get_distinct_files_same_second() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let w1 = SessionLogWriter::open(tmp.path(), "web", None, 1024, 10)?;
        let w2 = SessionLogWriter::open(tmp.path(), "web", None, 1024, 10)?;
        assert_ne!(w1.path(), w2.path());
        drop(w1);
        drop(w2);
        Ok(())
    }

    #[test]
    fn managed_hosts_with_colliding_names_use_distinct_dirs() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let w1 = SessionLogWriter::open(tmp.path(), "web/prod", Some(1), 1024, 10)?;
        let w2 = SessionLogWriter::open(tmp.path(), "web_prod", Some(2), 1024, 10)?;
        assert_ne!(w1.host_dir(), w2.host_dir());
        drop(w1);
        drop(w2);
        Ok(())
    }
}
