//! PTY session transcript logging to plain-text files under the data directory.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
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

fn timestamp_filename(serial: u32) -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if serial == 0 {
        format!("{secs}.log")
    } else {
        format!("{secs}-{serial}.log")
    }
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
}

impl SessionLogWriter {
    pub fn open(
        base_dir: impl AsRef<Path>,
        host_name: &str,
        max_file_bytes: u64,
        retention_files: usize,
    ) -> Result<Self> {
        let base_dir = base_dir.as_ref().to_path_buf();
        let logs_root = base_dir.join("logs");
        fs::create_dir_all(&logs_root)
            .with_context(|| format!("create session logs dir {}", logs_root.display()))?;
        secure_fs::restrict_dir(&logs_root);

        let host_dir = logs_root.join(sanitize_host_dir(host_name));
        fs::create_dir_all(&host_dir)
            .with_context(|| format!("create host log dir {}", host_dir.display()))?;
        secure_fs::restrict_dir(&host_dir);

        let current_path = host_dir.join(timestamp_filename(0));
        let file = File::create(&current_path)
            .with_context(|| format!("create session log {}", current_path.display()))?;
        secure_fs::restrict_file(&current_path);

        Ok(Self {
            host_dir,
            current_path,
            writer: BufWriter::new(file),
            bytes_written: 0,
            max_file_bytes,
            retention_files,
            file_serial: 0,
        })
    }

    pub fn path(&self) -> &Path {
        &self.current_path
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
        let new_path = self.host_dir.join(timestamp_filename(self.file_serial));
        let file = File::create(&new_path)
            .with_context(|| format!("rotate session log {}", new_path.display()))?;
        secure_fs::restrict_file(&new_path);
        self.current_path = new_path;
        self.writer = BufWriter::new(file);
        self.bytes_written = 0;
        Ok(())
    }

    pub fn close(mut self) -> Result<()> {
        self.writer.flush()?;
        drop(self.writer);
        prune_retention(&self.host_dir, self.retention_files)?;
        Ok(())
    }
}

/// Delete oldest `.log` files beyond `retention_files` in `host_dir`.
pub fn prune_retention(host_dir: &Path, retention_files: usize) -> Result<()> {
    if retention_files == 0 {
        return Ok(());
    }
    let mut logs: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(host_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "log")
        })
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
        let mut w = SessionLogWriter::open(tmp.path(), "web", 1024, 2)?;
        w.append(b"hello")?;
        let first = w.path().to_path_buf();
        w.close()?;

        // Seed two stale files so retention keeps only the newest two.
        let host_dir = tmp.path().join("logs").join("web");
        fs::write(host_dir.join("old1.log"), b"x")?;
        fs::write(host_dir.join("old2.log"), b"x")?;
        std::thread::sleep(std::time::Duration::from_millis(5));

        let mut w2 = SessionLogWriter::open(tmp.path(), "web", 1024, 2)?;
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
        let mut w = SessionLogWriter::open(tmp.path(), "db", 8, 10)?;
        let first = w.path().to_path_buf();
        w.append(b"12345678")?;
        w.append(b"X")?;
        let second = w.path().to_path_buf();
        assert_ne!(first, second);
        w.close()?;
        Ok(())
    }
}
