//! Writing pasted private-key material into `~/.ssh` as proper key files.

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Does `text` look like pasted private-key material (rather than a path)?
pub fn looks_like_private_key(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("-----BEGIN") && t.contains("PRIVATE KEY-----")
}

/// Write pasted key material to `~/.ssh/sshub_<name>` with `0600` permissions
/// and return the path.
///
/// If the destination already holds the same content it is reused; a name
/// collision with *different* content gets a numeric suffix so an existing
/// key is never overwritten.
pub fn write_key_material(name: &str, contents: &str) -> Result<PathBuf> {
    let home =
        std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME environment variable not set"))?;
    let ssh_dir = PathBuf::from(home).join(".ssh");
    std::fs::create_dir_all(&ssh_dir).context("create ~/.ssh")?;

    let safe_name: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe_name = if safe_name.is_empty() {
        "key".to_string()
    } else {
        safe_name
    };

    // Key files must end with a newline or ssh rejects them.
    let mut body = contents.trim_end().to_string();
    body.push('\n');

    for attempt in 0..100 {
        let file_name = if attempt == 0 {
            format!("sshub_{safe_name}")
        } else {
            format!("sshub_{safe_name}-{}", attempt + 1)
        };
        let dest = ssh_dir.join(file_name);

        if dest.exists() {
            match std::fs::read(&dest) {
                Ok(existing) if existing == body.as_bytes() => return Ok(dest),
                _ => continue, // different key under this name — try next suffix
            }
        }

        std::fs::write(&dest, &body)
            .with_context(|| format!("write key file {}", dest.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600))?;
        }
        return Ok(dest);
    }
    anyhow::bail!("too many conflicting sshub_{safe_name}* key files in ~/.ssh")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_key_material_vs_path() {
        assert!(looks_like_private_key(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----"
        ));
        assert!(looks_like_private_key(
            "  -----BEGIN RSA PRIVATE KEY-----\nabc"
        ));
        assert!(!looks_like_private_key("~/.ssh/id_ed25519"));
        assert!(!looks_like_private_key(
            "-----BEGIN CERTIFICATE-----\nabc"
        ));
    }

    #[test]
    fn writes_key_with_owner_only_perms_and_dedups() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());

        let blob = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";
        let p1 = write_key_material("work laptop", blob).unwrap();
        assert!(p1.ends_with("sshub_work_laptop"));
        let written = std::fs::read_to_string(&p1).unwrap();
        assert!(written.ends_with("-----END OPENSSH PRIVATE KEY-----\n"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p1).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // Same content → same file; different content → suffixed file.
        let p2 = write_key_material("work laptop", blob).unwrap();
        assert_eq!(p1, p2);
        let p3 = write_key_material("work laptop", "-----BEGIN OPENSSH PRIVATE KEY-----\nother\n-----END OPENSSH PRIVATE KEY-----").unwrap();
        assert_ne!(p1, p3);
        assert!(p3.to_string_lossy().contains("sshub_work_laptop-2"));

        std::env::remove_var("HOME");
    }
}
