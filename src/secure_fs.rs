//! Restrict permissions on directories and files that hold connection data
//! and credentials. On Unix this enforces `0700` for directories and `0600`
//! for files (owner-only). On other platforms these are no-ops — the calls
//! stay in place so behaviour is identical to "best effort".

use std::path::Path;

/// Tighten a directory to owner-only access (`0700`) on Unix. No-op elsewhere
/// and on any error (best effort — never fail the caller over a chmod).
pub fn restrict_dir(path: &Path) {
    set_mode(path, 0o700);
}

/// Tighten a file to owner-only access (`0600`) on Unix. No-op elsewhere and
/// on any error.
pub fn restrict_file(path: &Path) {
    set_mode(path, 0o600);
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        if perms.mode() & 0o777 != mode {
            perms.set_mode(mode);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn restrict_dir_and_file_set_owner_only_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("data");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("launcher.db");
        std::fs::write(&file, b"x").unwrap();
        // Loosen first so the assertion is meaningful.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

        restrict_dir(&dir);
        restrict_file(&file);

        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
