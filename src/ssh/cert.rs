//! SSH certificate introspection via `ssh-keygen -L`.
//!
//! Display and badge code never parses `ssh-keygen` output itself; everything
//! flows through [`status_for`] so this module's parser — tested against the
//! real tool (see `docs/oracle-tests.md`) — is the single place that can
//! misread a cert. Unparseable output degrades to [`CertBadge::Unreadable`],
//! never to a panic or a wrong validity claim.

use std::path::Path;
use std::process::Command;

/// Override for the `ssh-keygen` binary (tests point it at a missing path to
/// prove the fail-open behavior without touching `PATH`).
fn keygen_program() -> String {
    std::env::var("SSHUB_SSH_KEYGEN")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "ssh-keygen".to_string())
}

/// Parsed `ssh-keygen -L -f <cert>` output.
#[derive(Debug, Clone, PartialEq)]
pub struct CertInfo {
    /// e.g. `ssh-ed25519-cert-v01@openssh.com user certificate`.
    pub cert_type: String,
    pub key_id: String,
    pub serial: String,
    pub principals: Vec<String>,
    /// Epoch seconds; `None` means no lower bound (`forever`).
    pub valid_after: Option<i64>,
    /// Epoch seconds; `None` means no upper bound (`forever`).
    pub valid_before: Option<i64>,
    pub always_valid: bool,
    pub signing_ca: String,
}

/// Validity of a parsed cert at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validity {
    Valid,
    Expired,
    NotYetValid,
    /// The `Valid:` line was missing or unparseable — shown, not guessed.
    Unknown,
}

impl CertInfo {
    pub fn validity(&self, now_epoch: i64) -> Validity {
        if self.always_valid {
            return Validity::Valid;
        }
        match (self.valid_after, self.valid_before) {
            (None, None) => Validity::Unknown,
            (Some(after), _) if now_epoch < after => Validity::NotYetValid,
            (_, Some(before)) if now_epoch >= before => Validity::Expired,
            // Bounded below only, or inside the window: the `after` arm above
            // already excluded the too-early case.
            _ => Validity::Valid,
        }
    }
}

/// Why no cert info could be shown. Returned by [`inspect_certificate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertProblem {
    /// The path does not exist.
    Missing,
    /// `ssh-keygen` is absent, failed, or its output could not be parsed.
    Unreadable,
}

/// Run `ssh-keygen -L -f <path>` and parse the result. Fails open: every
/// failure mode maps to [`CertProblem`], never to an error the UI must handle.
pub fn inspect_certificate(path: &Path) -> Result<CertInfo, CertProblem> {
    let expanded = crate::ssh::expand_tilde(&path.to_string_lossy());
    if !expanded.exists() {
        return Err(CertProblem::Missing);
    }
    let output = Command::new(keygen_program())
        .arg("-L")
        .arg("-f")
        .arg(&expanded)
        .output()
        .map_err(|_| CertProblem::Unreadable)?;
    if !output.status.success() {
        return Err(CertProblem::Unreadable);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_keygen_l(&stdout).ok_or(CertProblem::Unreadable)
}

/// Pure parser over `ssh-keygen -L` stdout. Returns `None` when the output is
/// not a recognizable cert dump (cf. issue #7: never trust the shape blindly).
///
/// The dump is `Key: value` lines at one indent level with list continuations
/// (principals, extensions) indented one level deeper; the first line is the
/// bare file path. Anything else — including a `Valid:` line in an unknown
/// shape — yields `None` rather than a guessed window.
pub fn parse_keygen_l(output: &str) -> Option<CertInfo> {
    let mut fields: Vec<(String, String, Vec<String>)> = Vec::new();
    for line in output.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.starts_with("                ") {
            // Continuation of the previous field (e.g. one principal per line).
            if let Some((_, _, extra)) = fields.last_mut() {
                let item = trimmed_end.trim();
                if !item.is_empty() {
                    extra.push(item.to_string());
                }
            }
            continue;
        }
        if !trimmed_end.starts_with("        ") {
            continue;
        }
        let (key, value) = trimmed_end.trim().split_once(':')?;
        fields.push((key.trim().to_string(), value.trim().to_string(), Vec::new()));
    }
    let value_of = |name: &str| fields.iter().find(|(k, _, _)| k == name);
    let cert_type = value_of("Type")?.1.clone();
    if !cert_type.contains("-cert-v01@openssh.com") {
        return None;
    }
    let key_id = value_of("Key ID")
        .map(|(_, v, _)| v.trim_matches('"').to_string())
        .unwrap_or_default();
    let serial = value_of("Serial")
        .map(|(_, v, _)| v.clone())
        .unwrap_or_default();
    let signing_ca = value_of("Signing CA")
        .map(|(_, v, _)| v.clone())
        .unwrap_or_default();
    let principals = match value_of("Principals") {
        None => Vec::new(),
        Some((_, v, extra)) => {
            if v == "(none)" {
                Vec::new()
            } else if v.is_empty() {
                extra.clone()
            } else {
                std::iter::once(v.clone())
                    .chain(extra.iter().cloned())
                    .collect()
            }
        }
    };
    let valid_raw = value_of("Valid")
        .map(|(_, v, _)| v.clone())
        .unwrap_or_default();
    let (always_valid, valid_after, valid_before) = parse_validity(&valid_raw)?;
    Some(CertInfo {
        cert_type,
        key_id,
        serial,
        principals,
        valid_after,
        valid_before,
        always_valid,
        signing_ca,
    })
}

/// Split a `Valid:` value into `(always, after_epoch, before_epoch)`.
/// `ssh-keygen` renders the window in local time, so the timestamps are read
/// back with `mktime` — the same zone the tool printed them in.
fn parse_validity(raw: &str) -> Option<(bool, Option<i64>, Option<i64>)> {
    if raw == "always" || raw == "forever" {
        return Some((true, None, None));
    }
    if let Some(rest) = raw.strip_prefix("from ") {
        let (after, before) = rest.split_once(" to ")?;
        return Some((
            false,
            Some(parse_cert_time(after)?),
            Some(parse_cert_time(before)?),
        ));
    }
    if let Some(after) = raw.strip_prefix("after ") {
        return Some((false, Some(parse_cert_time(after)?), None));
    }
    if let Some(before) = raw.strip_prefix("before ") {
        return Some((false, None, Some(parse_cert_time(before)?)));
    }
    None
}

/// `2024-01-01T00:00:00` in local time → epoch seconds.
fn parse_cert_time(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    let (date, time) = raw.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day): (i32, i32, i32) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    if d.next().is_some() {
        return None;
    }
    let mut t = time.split(':');
    let (hh, mm, ss): (i32, i32, i32) = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
    );
    if t.next().is_some() {
        return None;
    }
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    let tm = libc::tm {
        tm_sec: ss,
        tm_min: mm,
        tm_hour: hh,
        tm_mday: day,
        tm_mon: m - 1,
        tm_year: y - 1900,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: -1,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    // SAFETY: `tm` is a fully-initialized local; `mktime` only reads it (and
    // normalizes a copy for the DST probe) under the process timezone, which
    // is exactly the zone `ssh-keygen` rendered the timestamp in.
    let epoch = unsafe { libc::mktime(&tm as *const libc::tm as *mut libc::tm) };
    if epoch < 0 {
        return None;
    }
    Some(epoch as i64)
}

/// Current time as epoch seconds, the basis [`CertInfo::validity`] compares
/// the parsed window against.
pub fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether the cert at `cert_path` was issued for the key at
/// `private_key_path`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMatch {
    Match,
    Mismatch,
    /// Undeterminable (missing file, encrypted key without passphrase,
    /// `ssh-keygen` absent) — callers fail open, never warn.
    Unknown,
}

/// Compare the cert's subject key against `ssh-keygen -y` output for the
/// private key, via fingerprints from the tool itself — no hand-rolled cert
/// blob parsing on either side.
pub fn check_key_match(
    cert_path: &Path,
    private_key_path: &Path,
    passphrase: Option<&str>,
) -> KeyMatch {
    let expanded_cert = crate::ssh::expand_tilde(&cert_path.to_string_lossy());
    let pub_line = match crate::ssh::keyfile::read_public_key(private_key_path, passphrase) {
        Ok(line) => line,
        Err(_) => return KeyMatch::Unknown,
    };
    let dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(_) => return KeyMatch::Unknown,
    };
    // `ssh-keygen -l` needs a file, not a string: stage the `-y` output where
    // only the owner can read it, like the key material it came from.
    let staged = dir.path().join("key.pub");
    if std::fs::write(&staged, format!("{pub_line}\n")).is_err() {
        return KeyMatch::Unknown;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600));
    }
    match (fingerprint(&staged), fingerprint(&expanded_cert)) {
        (Some(a), Some(b)) if a == b => KeyMatch::Match,
        (Some(_), Some(_)) => KeyMatch::Mismatch,
        _ => KeyMatch::Unknown,
    }
}

/// Second whitespace token of `ssh-keygen -l -E sha256 -f <path>` output
/// (`256 SHA256:… comment (TYPE)`), i.e. the fingerprint and nothing else —
/// comments may legitimately differ between the two sides.
fn fingerprint(path: &Path) -> Option<String> {
    let output = Command::new(keygen_program())
        .arg("-l")
        .arg("-E")
        .arg("sha256")
        .arg("-f")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&output.stdout);
    line.split_whitespace().nth(1).map(str::to_string)
}

/// Compact validity state for the keys-tab badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertBadge {
    Valid,
    Expired,
    NotYetValid,
    Missing,
    Unreadable,
    Mismatch,
}

/// The UI-ready snapshot the keys tab and the identity form share.
#[derive(Debug, Clone, PartialEq)]
pub struct CertStatus {
    pub badge: CertBadge,
    pub key_id: String,
    pub principals: Vec<String>,
    /// Raw `ssh-keygen` display strings (`2024-01-01T00:00:00`), kept verbatim
    /// so the UI repeats the tool instead of reformatting its dates.
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub always_valid: bool,
}

impl CertStatus {
    fn short_date(raw: Option<&str>) -> &str {
        raw.unwrap_or("?").split('T').next().unwrap_or("?")
    }

    /// First principal plus an overflow count (`alice`, `alice+2`).
    fn principals_short(&self) -> String {
        match self.principals.as_slice() {
            [] => String::new(),
            [first] => first.clone(),
            [first, rest @ ..] => format!("{first}+{}", rest.len()),
        }
    }

    /// One-line badge for the identity card, e.g. `cert alice+1→2030-01-01`.
    pub fn badge_text(&self) -> String {
        match self.badge {
            CertBadge::Valid => {
                let to = if self.always_valid {
                    "forever".to_string()
                } else {
                    Self::short_date(self.valid_to.as_deref()).to_string()
                };
                let who = self.principals_short();
                if who.is_empty() {
                    format!("cert valid→{to}")
                } else {
                    format!("cert {who}→{to}")
                }
            }
            CertBadge::Expired => "cert EXPIRED".to_string(),
            CertBadge::NotYetValid => "cert not yet valid".to_string(),
            CertBadge::Missing => "cert missing".to_string(),
            CertBadge::Unreadable => "cert unreadable".to_string(),
            CertBadge::Mismatch => "cert KEY MISMATCH".to_string(),
        }
    }

    /// Full detail line for the identity form.
    pub fn detail_line(&self) -> String {
        let who = if self.principals.is_empty() {
            "no principals".to_string()
        } else {
            format!("principals: {}", self.principals.join(", "))
        };
        let window = if self.always_valid {
            "always valid".to_string()
        } else {
            match (self.valid_from.as_deref(), self.valid_to.as_deref()) {
                (Some(from), Some(to)) => format!("valid {from} → {to}"),
                (Some(from), None) => format!("valid after {from}"),
                (None, Some(to)) => format!("valid before {to}"),
                (None, None) => "validity unknown".to_string(),
            }
        };
        let id = if self.key_id.is_empty() {
            "certificate".to_string()
        } else {
            format!("certificate {:?} ", self.key_id)
        };
        match self.badge {
            CertBadge::Valid => format!("{id}· {who} · {window}"),
            CertBadge::Expired => format!("{id}EXPIRED ({who}; was {window})"),
            CertBadge::NotYetValid => format!("{id}not yet valid ({who}; {window})"),
            CertBadge::Missing => "certificate file not found".to_string(),
            CertBadge::Unreadable => "certificate could not be read (not a cert?)".to_string(),
            CertBadge::Mismatch => {
                format!("{id}{who} · {window} — does not match the private key!")
            }
        }
    }
}

/// Inspect `cert_path` (plus an optional key for the mismatch check) and
/// reduce everything to one UI snapshot. Never fails: the worst case is a
/// `Missing`/`Unreadable` badge.
pub fn status_for(
    cert_path: &Path,
    key_path: Option<&Path>,
    passphrase: Option<&str>,
) -> CertStatus {
    let blank = |badge| CertStatus {
        badge,
        key_id: String::new(),
        principals: Vec::new(),
        valid_from: None,
        valid_to: None,
        always_valid: false,
    };
    let (info, display) = match inspect_full(cert_path) {
        Ok(pair) => pair,
        Err(CertProblem::Missing) => return blank(CertBadge::Missing),
        Err(CertProblem::Unreadable) => return blank(CertBadge::Unreadable),
    };
    let validity = info.validity(now_epoch());
    let mismatch = key_path
        .is_some_and(|key| check_key_match(cert_path, key, passphrase) == KeyMatch::Mismatch);
    let badge = match (validity, mismatch) {
        (Validity::Expired, _) => CertBadge::Expired,
        (Validity::NotYetValid, _) => CertBadge::NotYetValid,
        (Validity::Unknown, _) => CertBadge::Unreadable,
        (Validity::Valid, true) => CertBadge::Mismatch,
        (Validity::Valid, false) => CertBadge::Valid,
    };
    CertStatus {
        badge,
        key_id: display.key_id,
        principals: display.principals,
        valid_from: display.valid_from,
        valid_to: display.valid_to,
        always_valid: display.always_valid,
    }
}

/// One `ssh-keygen -L` run, parsed both ways: epochs for comparison plus the
/// tool's own date strings for display (repeating the tool instead of
/// reformatting its dates, which would need a timezone-aware formatter).
fn inspect_full(cert_path: &Path) -> Result<(CertInfo, CertDisplay), CertProblem> {
    let expanded = crate::ssh::expand_tilde(&cert_path.to_string_lossy());
    if !expanded.exists() {
        return Err(CertProblem::Missing);
    }
    let output = Command::new(keygen_program())
        .arg("-L")
        .arg("-f")
        .arg(&expanded)
        .output()
        .map_err(|_| CertProblem::Unreadable)?;
    if !output.status.success() {
        return Err(CertProblem::Unreadable);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout = stdout.as_ref();
    match (parse_keygen_l(stdout), parse_keygen_l_display(stdout)) {
        (Some(info), Some(display)) => Ok((info, display)),
        _ => Err(CertProblem::Unreadable),
    }
}

/// Display strings from the same dump [`parse_keygen_l`] reads epochs from.
struct CertDisplay {
    key_id: String,
    principals: Vec<String>,
    valid_from: Option<String>,
    valid_to: Option<String>,
    always_valid: bool,
}

/// Same field walk as [`parse_keygen_l`] but keeps the raw `Valid:` bounds.
/// Split out so the epoch parser stays the single validity authority while
/// the UI still shows the tool's own strings.
fn parse_keygen_l_display(output: &str) -> Option<CertDisplay> {
    let mut fields: Vec<(String, String, Vec<String>)> = Vec::new();
    for line in output.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.starts_with("                ") {
            if let Some((_, _, extra)) = fields.last_mut() {
                let item = trimmed_end.trim();
                if !item.is_empty() {
                    extra.push(item.to_string());
                }
            }
            continue;
        }
        if !trimmed_end.starts_with("        ") {
            continue;
        }
        let (key, value) = trimmed_end.trim().split_once(':')?;
        fields.push((key.trim().to_string(), value.trim().to_string(), Vec::new()));
    }
    let value_of = |name: &str| fields.iter().find(|(k, _, _)| k == name);
    if !value_of("Type")?.1.contains("-cert-v01@openssh.com") {
        return None;
    }
    let key_id = value_of("Key ID")
        .map(|(_, v, _)| v.trim_matches('"').to_string())
        .unwrap_or_default();
    let principals = match value_of("Principals") {
        None => Vec::new(),
        Some((_, v, extra)) => {
            if v == "(none)" {
                Vec::new()
            } else if v.is_empty() {
                extra.clone()
            } else {
                std::iter::once(v.clone())
                    .chain(extra.iter().cloned())
                    .collect()
            }
        }
    };
    let valid_raw = value_of("Valid")
        .map(|(_, v, _)| v.clone())
        .unwrap_or_default();
    if valid_raw == "always" || valid_raw == "forever" {
        return Some(CertDisplay {
            key_id,
            principals,
            valid_from: None,
            valid_to: None,
            always_valid: true,
        });
    }
    if let Some(rest) = valid_raw.strip_prefix("from ") {
        let (from, to) = rest.split_once(" to ")?;
        return Some(CertDisplay {
            key_id,
            principals,
            valid_from: Some(from.to_string()),
            valid_to: Some(to.to_string()),
            always_valid: false,
        });
    }
    if let Some(from) = valid_raw.strip_prefix("after ") {
        return Some(CertDisplay {
            key_id,
            principals,
            valid_from: Some(from.to_string()),
            valid_to: None,
            always_valid: false,
        });
    }
    if let Some(to) = valid_raw.strip_prefix("before ") {
        return Some(CertDisplay {
            key_id,
            principals,
            valid_from: None,
            valid_to: Some(to.to_string()),
            always_valid: false,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Generate a throwaway CA + keypair + cert in a tempdir and return their
    /// paths. The oracle: everything asserted below comes from the real
    /// `ssh-keygen`, not from agent-authored fixtures.
    fn make_cert(
        dir: &tempfile::TempDir,
        key_id: &str,
        principals: Option<&str>,
        validity: &str,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping: no ssh-keygen binary");
            panic!("ssh-keygen required for cert oracle tests");
        }
        let ca = dir.path().join("ca");
        let key = dir.path().join("id");
        assert!(Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-f"])
            .arg(&ca)
            .args(["-N", "", "-C", "test ca"])
            .output()
            .unwrap()
            .status
            .success());
        assert!(Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-f"])
            .arg(&key)
            .args(["-N", "", "-C", "test key"])
            .output()
            .unwrap()
            .status
            .success());
        let mut sign = Command::new("ssh-keygen");
        sign.arg("-s").arg(&ca).arg("-I").arg(key_id);
        if let Some(n) = principals {
            sign.arg("-n").arg(n);
        }
        sign.arg("-V").arg(validity);
        sign.arg(format!("{}.pub", key.display()));
        assert!(sign.output().unwrap().status.success());
        let cert = dir.path().join("id-cert.pub");
        assert!(cert.exists(), "ssh-keygen did not write the cert");
        (key, cert)
    }

    #[test]
    fn inspect_real_cert_reports_id_principals_and_window() {
        let dir = tempfile::tempdir().unwrap();
        let (_key, cert) = make_cert(
            &dir,
            "test-key-id",
            Some("alice,root"),
            "20240101000000:20300101000000",
        );

        let info = inspect_certificate(&cert).expect("real cert must parse");
        assert_eq!(info.key_id, "test-key-id");
        assert_eq!(info.principals, vec!["alice", "root"]);
        assert!(!info.always_valid);
        assert!(info.valid_after.is_some() && info.valid_before.is_some());
        assert_eq!(info.validity(now_epoch()), Validity::Valid);
    }

    #[test]
    fn expired_cert_from_real_keygen_classifies_expired() {
        let dir = tempfile::tempdir().unwrap();
        let (_key, cert) = make_cert(&dir, "old", Some("bob"), "20200101000000:20210101000000");

        let info = inspect_certificate(&cert).expect("real cert must parse");
        assert_eq!(info.validity(now_epoch()), Validity::Expired);
        assert_eq!(status_for(&cert, None, None).badge, CertBadge::Expired);
    }

    #[test]
    fn malformed_keygen_output_yields_none() {
        // Canned malformed fixtures: the parser must degrade, never panic and
        // never invent a validity window.
        for garbage in [
            "",
            "not a cert\n",
            "/tmp/x-cert.pub:\n",
            "/tmp/x-cert.pub:\n        Type: ssh-ed25519-cert-v01@openssh.com user certificate\n",
            "/tmp/x-cert.pub:\n        Valid: sometime eventually\n",
            "Type: no indentation at all\nValid: from 2024-01-01T00:00:00 to 2030-01-01T00:00:00\n",
        ] {
            assert_eq!(parse_keygen_l(garbage), None, "input: {garbage:?}");
        }
    }

    #[test]
    fn missing_path_reports_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            inspect_certificate(&dir.path().join("nope-cert.pub")),
            Err(CertProblem::Missing)
        );
    }

    #[test]
    fn hand_built_windows_classify_validity() {
        // Oracle: hand-built epoch windows against a fixed `now` — the
        // expectations are literals, not calls into the code under test.
        let base = CertInfo {
            cert_type: "ssh-ed25519-cert-v01@openssh.com user certificate".into(),
            key_id: "k".into(),
            serial: "0".into(),
            principals: vec!["alice".into()],
            valid_after: None,
            valid_before: None,
            always_valid: true,
            signing_ca: "ca".into(),
        };
        let now = 1_700_000_000; // 2023-11-14
        assert_eq!(base.validity(now), Validity::Valid);

        let windowed = CertInfo {
            always_valid: false,
            valid_after: Some(1_600_000_000),
            valid_before: Some(1_800_000_000),
            ..base.clone()
        };
        assert_eq!(windowed.validity(now), Validity::Valid);
        assert_eq!(windowed.validity(1_900_000_000), Validity::Expired);
        assert_eq!(windowed.validity(1_500_000_000), Validity::NotYetValid);

        let unbounded = CertInfo {
            always_valid: false,
            valid_after: None,
            valid_before: None,
            ..base.clone()
        };
        assert_eq!(unbounded.validity(now), Validity::Unknown);
    }

    #[test]
    fn matching_pair_reports_match() {
        let dir = tempfile::tempdir().unwrap();
        let (key, cert) = make_cert(&dir, "m", Some("alice"), "20240101000000:20300101000000");
        assert_eq!(check_key_match(&cert, &key, None), KeyMatch::Match);
        assert_eq!(status_for(&cert, Some(&key), None).badge, CertBadge::Valid);
    }

    #[test]
    fn crossed_pair_reports_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let (key_a, _cert_a) = make_cert(&dir, "a", Some("alice"), "20240101000000:20300101000000");
        // Second, independent keypair in another dir so nothing is shared.
        let dir2 = tempfile::tempdir().unwrap();
        let (_key_b, cert_b) = make_cert(&dir2, "b", Some("bob"), "20240101000000:20300101000000");
        assert_eq!(check_key_match(&cert_b, &key_a, None), KeyMatch::Mismatch);
        assert_eq!(
            status_for(&cert_b, Some(&key_a), None).badge,
            CertBadge::Mismatch
        );
    }

    #[test]
    fn keygen_absent_is_unreadable_not_panic() {
        let _lock = crate::test_env::lock_home();
        std::env::set_var("SSHUB_SSH_KEYGEN", "/nonexistent/sshub-test-keygen");
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent-cert.pub");
        // Missing short-circuits before any subprocess runs…
        assert_eq!(inspect_certificate(&missing), Err(CertProblem::Missing));
        // …while an existing file with no tool behind it degrades gracefully.
        let present = dir.path().join("present-cert.pub");
        std::fs::write(&present, "ssh-ed25519-cert-v01@openssh.com AAAA\n").unwrap();
        assert_eq!(inspect_certificate(&present), Err(CertProblem::Unreadable));
        assert_eq!(check_key_match(&present, &present, None), KeyMatch::Unknown);
        std::env::remove_var("SSHUB_SSH_KEYGEN");
    }

    #[test]
    fn badge_texts_cover_all_states() {
        // Oracle: hand-built snapshots — badge wording is asserted against
        // literals, never against the implementation's own detail strings.
        let valid = CertStatus {
            badge: CertBadge::Valid,
            key_id: "test-key-id".into(),
            principals: vec!["alice".into(), "root".into()],
            valid_from: Some("2024-01-01T00:00:00".into()),
            valid_to: Some("2030-01-01T00:00:00".into()),
            always_valid: false,
        };
        assert_eq!(valid.badge_text(), "cert alice+1→2030-01-01");
        assert!(valid.detail_line().contains("test-key-id"));
        assert!(valid.detail_line().contains("alice, root"));

        let expired = CertStatus {
            badge: CertBadge::Expired,
            ..valid.clone()
        };
        assert!(expired.badge_text().contains("EXPIRED"));

        let missing = CertStatus {
            badge: CertBadge::Missing,
            key_id: String::new(),
            principals: vec![],
            valid_from: None,
            valid_to: None,
            always_valid: false,
        };
        assert_eq!(missing.badge_text(), "cert missing");

        let mismatch = CertStatus {
            badge: CertBadge::Mismatch,
            ..valid.clone()
        };
        assert!(mismatch.badge_text().contains("MISMATCH"));
        assert!(mismatch.detail_line().contains("does not match"));
    }
}
