//! Importer for the manual Termius CSV export produced by `termius-exporter.js`.
//!
//! The export is a directory containing:
//! - `L00t.csv` — host list (`Label,Host,Port,Username,Password,SSH_Key,OS`)
//! - `ssh_keys/` — `*-<fp>.pem` private keys and optional `*-<fp>.passphrase`
//! - `snippets.csv` — snippets (currently ignored; the app has no snippet store)
//!
//! See `termius_export_readme_file.md` for the full format description.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::credentials::PasswordStore;
use crate::store::{HostSource, LauncherStore, NewHost, NewIdentity};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// One row of `L00t.csv`.
#[derive(Debug, Clone)]
pub struct CsvHostRow {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// Key label *hint* — may be empty or `key_id:<id>` (unreliable per readme).
    pub ssh_key: String,
    pub os: String,
}

/// A private key discovered under `ssh_keys/`.
#[derive(Debug, Clone)]
pub struct CsvKeyFile {
    /// SHA-256 fingerprint (16 hex chars), the stable identifier.
    pub fingerprint: String,
    /// Filename base derived from the Termius key label (may be empty).
    pub base: String,
    pub pem_path: PathBuf,
    pub passphrase: Option<String>,
}

#[derive(Debug, Default)]
pub struct CsvImportReport {
    pub hosts_imported: usize,
    pub identities_created: usize,
    /// Hosts that already existed by name. We do NOT touch their other
    /// fields, but we always refresh their stored credentials from the CSV
    /// — see `passwords_refreshed`.
    pub skipped: usize,
    /// Number of host passwords (re)stored in the keyring this run.
    /// Includes brand-new hosts and existing ones that had a credential in
    /// the CSV.
    pub passwords_stored: usize,
    /// Number of identity passphrases (re)stored in the keyring.
    pub passphrases_stored: usize,
    /// Number of keyring writes that failed verification (set succeeded
    /// but a subsequent get didn't return the same value — the backend
    /// is broken / locked / not persisting).
    pub keyring_failures: usize,
}

// ---------------------------------------------------------------------------
// CSV parsing
// ---------------------------------------------------------------------------

/// Parse RFC4180-style CSV text into rows of fields.
///
/// Handles a leading UTF-8 BOM, `""`-escaped quotes inside quoted fields, and
/// both `\n` and `\r\n` line endings.
fn parse_csv(content: &str) -> Vec<Vec<String>> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' => in_quotes = true,
                ',' => record.push(std::mem::take(&mut field)),
                '\r' => {} // swallow; the following '\n' ends the record
                '\n' => {
                    record.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut record));
                }
                _ => field.push(c),
            }
        }
    }

    // Trailing field/record without a final newline.
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        rows.push(record);
    }

    rows
}

/// Parse the contents of `L00t.csv` into host rows (header skipped).
pub fn parse_loot_csv(content: &str) -> Vec<CsvHostRow> {
    let mut rows = parse_csv(content).into_iter();
    // Drop the header line.
    rows.next();

    rows.filter_map(|cols| {
        if cols.iter().all(|c| c.trim().is_empty()) {
            return None;
        }
        let col = |i: usize| cols.get(i).map(String::as_str).unwrap_or("");

        let label = col(0).trim().to_string();
        let host = col(1).trim().to_string();
        if label.is_empty() && host.is_empty() {
            return None;
        }
        let port = col(2).trim().parse::<u16>().unwrap_or(22);

        Some(CsvHostRow {
            label,
            host,
            port,
            username: col(3).trim().to_string(),
            // Preserve password and key hint verbatim (no trimming).
            password: col(4).to_string(),
            ssh_key: col(5).trim().to_string(),
            os: col(6).trim().to_string(),
        })
    })
    .collect()
}

// ---------------------------------------------------------------------------
// Key discovery
// ---------------------------------------------------------------------------

/// Split a `*-<fp>` filename stem into `(base, fingerprint)`.
///
/// Falls back to `(stem, "")` when no trailing 16-hex-char fingerprint is found.
fn split_fingerprint(stem: &str) -> (String, String) {
    if let Some(idx) = stem.rfind('-') {
        let (base, rest) = stem.split_at(idx);
        let fp = &rest[1..];
        if fp.len() == 16 && fp.chars().all(|c| c.is_ascii_hexdigit()) {
            return (base.to_string(), fp.to_string());
        }
    }
    (stem.to_string(), String::new())
}

/// Discover private keys in an `ssh_keys/` directory.
pub fn discover_keys(ssh_keys_dir: &Path) -> Vec<CsvKeyFile> {
    let mut keys = Vec::new();
    let Ok(entries) = std::fs::read_dir(ssh_keys_dir) else {
        return keys;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "pem") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let (base, fingerprint) = split_fingerprint(&stem);

        let passphrase = std::fs::read_to_string(path.with_extension("passphrase"))
            .ok()
            .map(|s| s.trim_end_matches(['\n', '\r']).to_string())
            .filter(|s| !s.is_empty());

        keys.push(CsvKeyFile {
            fingerprint,
            base,
            pem_path: path,
            passphrase,
        });
    }
    keys
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Import a Termius CSV export directory into the launcher store.
///
/// - Locates `L00t.csv` in `export_dir` and parses host rows.
/// - Creates one identity per private key found in `export_dir/ssh_keys/`,
///   copying the key into `~/.ssh/termius_<name>` and storing any passphrase.
/// - Creates hosts, linking them to identities when the `SSH_Key` hint matches a
///   discovered key, and storing per-host passwords via `password_store`.
/// - Hosts whose name already exists are skipped.
pub fn import_csv_export(
    export_dir: &Path,
    store: &LauncherStore,
    password_store: &dyn PasswordStore,
) -> Result<CsvImportReport> {
    let mut report = CsvImportReport::default();

    let loot_path = find_loot_csv(export_dir)
        .ok_or_else(|| anyhow::anyhow!("L00t.csv not found in {}", export_dir.display()))?;
    let content = std::fs::read_to_string(&loot_path)
        .with_context(|| format!("reading {}", loot_path.display()))?;
    let rows = parse_loot_csv(&content);

    // 1. Create identities from discovered key files.
    let key_files = discover_keys(&export_dir.join("ssh_keys"));
    // Maps a host's SSH_Key hint (key label / filename base) to an identity id.
    let mut key_by_hint: HashMap<String, i64> = HashMap::new();

    for kf in &key_files {
        let name = if kf.base.is_empty() {
            format!("termius-{}", kf.fingerprint)
        } else {
            kf.base.clone()
        };

        let identity_id = if let Some(existing) = store.get_identity_by_name(&name)? {
            existing.id
        } else {
            let dest = copy_key_into_ssh(&kf.pem_path, &name)?;
            let identity = store.create_identity(&NewIdentity {
                name: name.clone(),
                username: None,
                private_key: Some(dest),
                certificate: None,
                sort_order: 0,
                has_password: kf.passphrase.is_some(),
            })?;
            report.identities_created += 1;
            identity.id
        };
        // Always refresh the stored passphrase, even on an existing identity
        // — same rationale as for host passwords.
        if let Some(passphrase) = &kf.passphrase {
            match store_credential_verified(
                password_store,
                &crate::credentials::identity_key(identity_id),
                passphrase,
            ) {
                Ok(()) => report.passphrases_stored += 1,
                Err(_) => report.keyring_failures += 1,
            }
        }

        key_by_hint.entry(name).or_insert(identity_id);
        if !kf.base.is_empty() {
            key_by_hint.entry(kf.base.clone()).or_insert(identity_id);
        }
    }

    // 2. Create hosts. If a host with the same name already exists we keep
    // its other fields intact but we always refresh the stored password
    // from the CSV — fixes the case where a previous import marked the
    // host as having a password but the keyring write didn't actually
    // persist (broken backend, locked wallet on a different login session,
    // etc.).
    for row in &rows {
        let name = if row.label.is_empty() {
            row.host.clone()
        } else {
            row.label.clone()
        };
        if name.is_empty() {
            report.skipped += 1;
            continue;
        }
        let has_password = !row.password.is_empty();

        // Either create the row or look up the existing one.
        let host_id = match store.get_host_by_name(&name)? {
            Some(existing) => {
                report.skipped += 1;
                existing.id
            }
            None => {
                let identity_id = resolve_key_hint(&row.ssh_key, &key_by_hint);
                let username = (!row.username.is_empty()).then(|| row.username.clone());
                let host = store.create_host(&NewHost {
                    name: name.clone(),
                    address: row.host.clone(),
                    port: row.port,
                    username,
                    identity_id,
                    os_icon: os_icon_for(&row.os),
                    source: HostSource::Launcher,
                    has_password,
                    ..Default::default()
                })?;
                report.hosts_imported += 1;
                host.id
            }
        };

        if has_password {
            match store_credential_verified(
                password_store,
                &crate::credentials::host_key(host_id),
                &row.password,
            ) {
                Ok(()) => report.passwords_stored += 1,
                Err(_) => report.keyring_failures += 1,
            }
        }
    }

    Ok(report)
}

/// Set a credential in the keyring and verify by reading it back. Returns
/// `Err` if the backend reports success but a subsequent get doesn't return
/// the same value — typical when the secret service / D-Bus / kernel keyring
/// isn't actually persisting writes.
fn store_credential_verified(store: &dyn PasswordStore, key: &str, value: &str) -> Result<()> {
    store
        .set(key, value)
        .with_context(|| format!("set {key}"))?;
    match store.get(key) {
        Ok(Some(roundtrip)) if roundtrip == value => Ok(()),
        Ok(Some(_)) => anyhow::bail!(
            "keyring roundtrip for {key} returned a different value — \
             check that only one keyring backend is active"
        ),
        Ok(None) => anyhow::bail!(
            "keyring write for {key} silently dropped — no D-Bus session / \
             locked wallet / no secret service?"
        ),
        Err(e) => Err(e).with_context(|| format!("verify {key}")),
    }
}

/// Resolve a host's `SSH_Key` hint to an identity id, if mappable.
///
/// Empty hints and `key_id:<id>` placeholders are treated as "no key".
fn resolve_key_hint(hint: &str, key_by_hint: &HashMap<String, i64>) -> Option<i64> {
    let hint = hint.trim();
    if hint.is_empty() || hint.starts_with("key_id:") {
        return None;
    }
    key_by_hint.get(hint).copied()
}

/// Map a Termius OS hint to one of the app's known `os_icon` names.
fn os_icon_for(os: &str) -> Option<String> {
    let os = os.trim().to_ascii_lowercase();
    match os.as_str() {
        "ubuntu" => Some("ubuntu".to_string()),
        "debian" => Some("debian".to_string()),
        "" => None,
        // Any other recognised OS becomes the generic glyph.
        _ => Some("generic".to_string()),
    }
}

/// Best-effort guess of where the manual export lives, used to prefill the
/// import prompt. Checks the current directory and `~/Downloads` for a folder
/// containing `L00t.csv`. Returns `None` when nothing obvious is found.
pub fn default_export_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(&home).join("Downloads"));
        candidates.push(PathBuf::from(home));
    }
    candidates
        .into_iter()
        .find(|dir| find_loot_csv(dir).is_some())
}

/// Locate `L00t.csv` inside the export directory (case-insensitive).
fn find_loot_csv(dir: &Path) -> Option<PathBuf> {
    let exact = dir.join("L00t.csv");
    if exact.exists() {
        return Some(exact);
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("l00t.csv"))
        {
            return Some(path);
        }
    }
    None
}

/// Copy a private key into `~/.ssh/termius_<name>` with `0600` permissions.
///
/// Existing files are left untouched.
fn copy_key_into_ssh(src: &Path, name: &str) -> Result<PathBuf> {
    let home =
        std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME environment variable not set"))?;
    let ssh_dir = PathBuf::from(home).join(".ssh");
    std::fs::create_dir_all(&ssh_dir)?;

    let safe_name = name.replace(['/', '\\', ' '], "_");
    let dest = ssh_dir.join(format!("termius_{safe_name}"));

    if !dest.exists() {
        let content =
            std::fs::read(src).with_context(|| format!("reading key {}", src.display()))?;
        std::fs::write(&dest, content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(dest)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_csv_handles_bom_and_quotes() {
        let content = "\u{feff}Label,Host,Port\n\"a,b\",\"line1\nline2\",22\n\"quote\"\"d\",x,1\n";
        let rows = parse_csv(content);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], vec!["Label", "Host", "Port"]);
        assert_eq!(rows[1], vec!["a,b", "line1\nline2", "22"]);
        assert_eq!(rows[2], vec!["quote\"d", "x", "1"]);
    }

    #[test]
    fn parse_loot_csv_skips_header_and_empty_rows() {
        let content = "Label,Host,Port,Username,Password,SSH_Key,OS\n\
            web,10.0.0.1,22,admin,secret,mykey,ubuntu\n\
            \n\
            ,10.0.0.2,2222,root,,,debian\n";
        let rows = parse_loot_csv(content);
        assert_eq!(rows.len(), 2);

        assert_eq!(rows[0].label, "web");
        assert_eq!(rows[0].host, "10.0.0.1");
        assert_eq!(rows[0].port, 22);
        assert_eq!(rows[0].username, "admin");
        assert_eq!(rows[0].password, "secret");
        assert_eq!(rows[0].ssh_key, "mykey");
        assert_eq!(rows[0].os, "ubuntu");

        // Empty label, no password/key.
        assert_eq!(rows[1].label, "");
        assert_eq!(rows[1].host, "10.0.0.2");
        assert_eq!(rows[1].port, 2222);
        assert_eq!(rows[1].password, "");
    }

    #[test]
    fn parse_loot_csv_defaults_bad_port_to_22() {
        let content = "Label,Host,Port,Username,Password,SSH_Key,OS\nh,1.2.3.4,notaport,u,,,\n";
        let rows = parse_loot_csv(content);
        assert_eq!(rows[0].port, 22);
    }

    #[test]
    fn split_fingerprint_extracts_trailing_hex() {
        let (base, fp) = split_fingerprint("my-key-0123456789abcdef");
        assert_eq!(base, "my-key");
        assert_eq!(fp, "0123456789abcdef");

        // No valid fingerprint suffix.
        let (base, fp) = split_fingerprint("plainname");
        assert_eq!(base, "plainname");
        assert_eq!(fp, "");
    }

    #[test]
    fn resolve_key_hint_ignores_placeholders() {
        let mut map = HashMap::new();
        map.insert("mykey".to_string(), 7);
        assert_eq!(resolve_key_hint("mykey", &map), Some(7));
        assert_eq!(resolve_key_hint("", &map), None);
        assert_eq!(resolve_key_hint("key_id:abc", &map), None);
        assert_eq!(resolve_key_hint("unknown", &map), None);
    }

    #[test]
    fn os_icon_for_maps_known_oses() {
        assert_eq!(os_icon_for("ubuntu"), Some("ubuntu".to_string()));
        assert_eq!(os_icon_for("Debian"), Some("debian".to_string()));
        assert_eq!(os_icon_for("fedora"), Some("generic".to_string()));
        assert_eq!(os_icon_for(""), None);
    }

    #[test]
    fn import_csv_export_imports_hosts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("L00t.csv"),
            "Label,Host,Port,Username,Password,SSH_Key,OS\n\
             web,10.0.0.1,22,admin,pw1,,ubuntu\n\
             db,10.0.0.2,5432,root,,,\n",
        )
        .unwrap();

        let store = LauncherStore::open_in_memory().unwrap();
        let pw = crate::credentials::NoopPasswordStore;
        let report = import_csv_export(dir.path(), &store, &pw).unwrap();

        assert_eq!(report.hosts_imported, 2);
        assert_eq!(report.identities_created, 0);
        assert_eq!(report.skipped, 0);

        let web = store.get_host_by_name("web").unwrap().unwrap();
        assert_eq!(web.address, "10.0.0.1");
        assert_eq!(web.port, 22);
        assert_eq!(web.username.as_deref(), Some("admin"));
        assert!(web.has_password);
        assert_eq!(web.os_icon.as_deref(), Some("ubuntu"));
        assert_eq!(web.source, HostSource::Launcher);

        let db = store.get_host_by_name("db").unwrap().unwrap();
        assert_eq!(db.port, 5432);
        assert!(!db.has_password);
    }

    #[test]
    fn import_csv_export_skips_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("L00t.csv"),
            "Label,Host,Port,Username,Password,SSH_Key,OS\nweb,10.0.0.1,22,admin,,,\n",
        )
        .unwrap();

        let store = LauncherStore::open_in_memory().unwrap();
        let pw = crate::credentials::NoopPasswordStore;

        let first = import_csv_export(dir.path(), &store, &pw).unwrap();
        assert_eq!(first.hosts_imported, 1);
        let second = import_csv_export(dir.path(), &store, &pw).unwrap();
        assert_eq!(second.hosts_imported, 0);
        assert_eq!(second.skipped, 1);
    }

    #[test]
    fn discover_keys_reads_pem_and_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("ssh_keys");
        std::fs::create_dir_all(&keys).unwrap();
        std::fs::write(
            keys.join("mykey-0123456789abcdef.pem"),
            "-----BEGIN PRIVATE KEY-----\nx\n-----END PRIVATE KEY-----\n",
        )
        .unwrap();
        std::fs::write(keys.join("mykey-0123456789abcdef.passphrase"), "hunter2\n").unwrap();
        // A bare key with no fingerprint suffix and no passphrase.
        std::fs::write(keys.join("plain.pem"), "data").unwrap();

        let mut found = discover_keys(&keys);
        found.sort_by(|a, b| a.base.cmp(&b.base));
        assert_eq!(found.len(), 2);

        assert_eq!(found[0].base, "mykey");
        assert_eq!(found[0].fingerprint, "0123456789abcdef");
        assert_eq!(found[0].passphrase.as_deref(), Some("hunter2"));

        assert_eq!(found[1].base, "plain");
        assert_eq!(found[1].fingerprint, "");
        assert_eq!(found[1].passphrase, None);
    }

    #[test]
    fn import_csv_export_errors_without_loot_csv() {
        let dir = tempfile::tempdir().unwrap();
        let store = LauncherStore::open_in_memory().unwrap();
        let pw = crate::credentials::NoopPasswordStore;
        assert!(import_csv_export(dir.path(), &store, &pw).is_err());
    }
}
