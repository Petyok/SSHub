use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    CertAuthority,
    Revoked,
}

impl std::fmt::Display for Marker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Marker::CertAuthority => write!(f, "@cert-authority"),
            Marker::Revoked => write!(f, "@revoked"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownHostEntry {
    pub marker: Option<Marker>,
    pub hosts: String,
    pub key_type: String,
    pub fingerprint: Option<String>,
}

impl KnownHostEntry {
    pub fn is_hashed(&self) -> bool {
        self.hosts.starts_with("|1|")
    }

    pub fn is_deletable(&self) -> bool {
        !self.is_hashed()
            && self.marker.is_none()
            && !self.hosts.contains('*')
            && !self.hosts.contains('?')
    }

    pub fn deletion_block_reason(&self) -> Option<&'static str> {
        if self.is_hashed() {
            Some("Cannot delete hashed entry \u{2014} run ssh-keygen -R <host> manually, or set HashKnownHosts no")
        } else if self.marker.is_some() {
            Some("Cannot delete @cert-authority / @revoked entries \u{2014} edit ~/.ssh/known_hosts manually")
        } else if self.hosts.contains('*') || self.hosts.contains('?') {
            Some("Cannot delete wildcard entries \u{2014} edit ~/.ssh/known_hosts manually")
        } else {
            None
        }
    }

    pub fn display_host(&self) -> &str {
        if self.is_hashed() {
            "(hashed)"
        } else {
            &self.hosts
        }
    }

    pub fn display_type(&self) -> String {
        normalize_key_type(&self.key_type)
    }
}

pub fn known_hosts_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".ssh").join("known_hosts")
}

pub fn parse_known_hosts(content: &str) -> Vec<KnownHostEntry> {
    let mut entries = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let first = match fields.next() {
            Some(f) => f,
            None => continue,
        };

        let (marker, hosts) = match first {
            "@cert-authority" => (Some(Marker::CertAuthority), fields.next()),
            "@revoked" => (Some(Marker::Revoked), fields.next()),
            _ => (None, Some(first)),
        };

        let hosts = match hosts {
            Some(h) => h,
            None => continue,
        };
        let key_type = match fields.next() {
            Some(t) => t,
            None => continue,
        };
        if fields.next().is_none() {
            continue;
        }

        entries.push(KnownHostEntry {
            marker,
            hosts: hosts.to_string(),
            key_type: key_type.to_string(),
            fingerprint: None,
        });
    }
    entries
}

fn normalize_key_type(raw: &str) -> String {
    let upper = raw.to_uppercase();
    if let Some(rest) = upper.strip_prefix("SSH-") {
        rest.to_string()
    } else if upper.starts_with("ECDSA-SHA2-") {
        "ECDSA".to_string()
    } else if let Some(rest) = upper.strip_prefix("SK-SSH-") {
        rest.split('@').next().unwrap_or(rest).to_string()
    } else if upper.starts_with("SK-ECDSA-SHA2-") {
        "ECDSA".to_string()
    } else {
        upper
    }
}

fn fingerprints(path: &Path) -> HashMap<(String, String), Vec<String>> {
    let mut map: HashMap<(String, String), Vec<String>> = HashMap::new();
    let Ok(output) = Command::new("ssh-keygen")
        .args(["-l", "-f"])
        .arg(path)
        .output()
    else {
        return map;
    };
    if !output.status.success() {
        return map;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let _bits = parts.next();
        let fp = match parts.next() {
            Some(fp) if fp.starts_with("SHA256:") => fp,
            _ => continue,
        };
        let name = match parts.next() {
            Some(n) => n,
            None => continue,
        };
        let type_raw = match parts.next() {
            Some(t) => t.trim_start_matches('(').trim_end_matches(')'),
            None => continue,
        };
        map.entry((name.to_string(), type_raw.to_string()))
            .or_default()
            .push(fp.to_string());
    }
    map
}

pub fn load_known_hosts(path: &Path) -> Result<Vec<KnownHostEntry>> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut entries = parse_known_hosts(&content);
    let mut fps = fingerprints(path);
    for entry in &mut entries {
        let norm = normalize_key_type(&entry.key_type);
        let key = (entry.hosts.clone(), norm);
        if let Some(list) = fps.get_mut(&key) {
            if !list.is_empty() {
                entry.fingerprint = Some(list.remove(0));
            }
        }
    }
    Ok(entries)
}

pub fn remove_host(name: &str, path: &Path) -> Result<()> {
    let mut ran = false;
    for host in name.split(',') {
        let host = host.trim();
        if host.is_empty() || host.starts_with('-') {
            continue;
        }
        let output = Command::new("ssh-keygen")
            .args(["-R", host, "-f"])
            .arg(path)
            .output()
            .context("run ssh-keygen -R")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("ssh-keygen -R failed: {}", stderr.trim());
        }
        ran = true;
    }
    if !ran {
        anyhow::bail!("no valid host names to remove");
    }
    Ok(())
}

pub fn host_key_fingerprint_from_log(debug_log: &str) -> Option<String> {
    for line in debug_log.lines() {
        let line = line.trim();
        if !line.starts_with("debug1: Server host key:") {
            continue;
        }
        let rest = &line["debug1: Server host key:".len()..];
        let mut parts = rest.split_whitespace();
        let Some(key_type) = parts.next() else {
            continue;
        };
        if !key_type.starts_with("ssh-")
            && !key_type.starts_with("ecdsa-")
            && !key_type.starts_with("sk-")
        {
            continue;
        }
        if let Some(fp) = parts.next() {
            let Some(b64) = fp.strip_prefix("SHA256:") else {
                continue;
            };
            if b64.len() == 43
                && b64
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
            {
                return Some(fp.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    const FIXTURE: &str = "\
# This is a comment
host-a.example.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBbSwmRXm0WEQzC3oHnJkV0tBk3kCQh8mFjWz3nLx9oK user@host-a

[host-b.example.com]:2222 ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQC7vbqajD user@host-b
|1|abc123def456=|ghi789jkl012= ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHnXmK4oXsQmBpDPn8l0V3aFk7R2sYw9cT5uN1eMx6Qb
@cert-authority *.example.com ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQDKm4VBc3oXk ca@example.com
@revoked bad-host.example.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMvXpKR3sQmBpDPn8l0V3aFk7R2sYw9cT5uN1eMx6Qc
";

    #[test]
    fn parse_plain_entry() {
        let entries = parse_known_hosts(FIXTURE);
        let plain = &entries[0];
        assert_eq!(plain.hosts, "host-a.example.com");
        assert_eq!(plain.key_type, "ssh-ed25519");
        assert_eq!(plain.marker, None);
        assert_eq!(plain.display_host(), "host-a.example.com");
        assert_eq!(plain.display_type(), "ED25519");
        assert!(!plain.is_hashed());
    }

    #[test]
    fn parse_port_qualified_entry() {
        let entries = parse_known_hosts(FIXTURE);
        let port = &entries[1];
        assert_eq!(port.hosts, "[host-b.example.com]:2222");
        assert_eq!(port.key_type, "ssh-rsa");
        assert_eq!(port.display_type(), "RSA");
    }

    #[test]
    fn parse_hashed_entry() {
        let entries = parse_known_hosts(FIXTURE);
        let hashed = &entries[2];
        assert!(hashed.is_hashed());
        assert_eq!(hashed.display_host(), "(hashed)");
    }

    #[test]
    fn parse_cert_authority_entry() {
        let entries = parse_known_hosts(FIXTURE);
        let ca = &entries[3];
        assert_eq!(ca.marker, Some(Marker::CertAuthority));
        assert_eq!(ca.hosts, "*.example.com");
    }

    #[test]
    fn parse_revoked_entry() {
        let entries = parse_known_hosts(FIXTURE);
        let revoked = &entries[4];
        assert_eq!(revoked.marker, Some(Marker::Revoked));
        assert_eq!(revoked.hosts, "bad-host.example.com");
    }

    #[test]
    fn parse_skips_comments_and_blanks() {
        let entries = parse_known_hosts(FIXTURE);
        assert_eq!(entries.len(), 5);
    }

    #[test]
    fn parse_skips_malformed_lines() {
        let content = "host-only\nhost ssh-ed25519\nvalid ssh-ed25519 AAAA comment\n";
        let entries = parse_known_hosts(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hosts, "valid");
    }

    #[test]
    fn normalize_key_type_strips_prefixes() {
        assert_eq!(normalize_key_type("ssh-ed25519"), "ED25519");
        assert_eq!(normalize_key_type("ssh-rsa"), "RSA");
        assert_eq!(normalize_key_type("ecdsa-sha2-nistp256"), "ECDSA");
        assert_eq!(normalize_key_type("sk-ssh-ed25519@openssh.com"), "ED25519");
        assert_eq!(
            normalize_key_type("sk-ecdsa-sha2-nistp256@openssh.com"),
            "ECDSA"
        );
    }

    #[test]
    fn delete_removes_target_and_keeps_others() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "host-a.example.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBbSwmRXm0WEQzC3oHnJkV0tBk3kCQh8mFjWz3nLx9oK").unwrap();
        writeln!(file, "# a comment").unwrap();
        writeln!(
            file,
            "host-b.example.com ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQC7vbqajD"
        )
        .unwrap();
        file.flush().unwrap();

        remove_host("host-a.example.com", file.path()).unwrap();

        let after = std::fs::read_to_string(file.path()).unwrap();
        assert!(!after.contains("host-a.example.com"));
        assert!(after.contains("host-b.example.com"));
        assert!(after.contains("# a comment"));

        let backup = file.path().with_extension("");
        let old_path = {
            let mut p = file.path().as_os_str().to_os_string();
            p.push(".old");
            PathBuf::from(p)
        };
        let _ = backup;
        assert!(old_path.exists(), ".old backup should exist");
    }

    #[test]
    fn delete_hashed_host_is_refused_by_caller() {
        let entry = KnownHostEntry {
            marker: None,
            hosts: "|1|abc|def".to_string(),
            key_type: "ssh-ed25519".to_string(),
            fingerprint: None,
        };
        assert!(entry.is_hashed());
    }

    #[test]
    fn fingerprint_from_log_extracts_sha256() {
        let log = "\
debug1: Server host key: ssh-ed25519 SHA256:wTZYfLI5nCdGqxsM2v45Z90mFjK3kCQh8mFjWz3nLx9
debug1: Host 'example.com' is known and matches the ED25519 host key.
";
        let fp = host_key_fingerprint_from_log(log);
        assert_eq!(
            fp,
            Some("SHA256:wTZYfLI5nCdGqxsM2v45Z90mFjK3kCQh8mFjWz3nLx9".to_string())
        );
    }

    #[test]
    fn fingerprint_from_log_rejects_spoofed_ident_line() {
        let log = "\
debug1: Remote protocol version 2.0, remote software version x debug1: Server host key: ssh-ed25519 SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
debug1: Server host key: ssh-ed25519 SHA256:wTZYfLI5nCdGqxsM2v45Z90mFjK3kCQh8mFjWz3nLx9
";
        let fp = host_key_fingerprint_from_log(log);
        assert_eq!(
            fp,
            Some("SHA256:wTZYfLI5nCdGqxsM2v45Z90mFjK3kCQh8mFjWz3nLx9".to_string())
        );
    }

    #[test]
    fn fingerprint_from_log_returns_none_when_absent() {
        assert_eq!(host_key_fingerprint_from_log("debug1: Connecting..."), None);
    }
}
