//! Importer for PuTTY saved sessions.
//!
//! PuTTY stores sessions in two shapes depending on the platform:
//!
//! - **Windows** — under the registry key
//!   `HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\<name>`. Users hand
//!   these to us as a `regedit /e` `.reg` export (usually UTF-16LE with a BOM).
//!   Values look like `"HostName"="1.2.3.4"` or `"PortNumber"=dword:00000016`.
//! - **Unix** — one plain-text file per session under `~/.putty/sessions/`,
//!   with `Key=Value` lines and a **decimal** `PortNumber`.
//!
//! Both encode the session name (percent-encoded, so `My Server` becomes
//! `My%20Server`). We only import connections whose `Protocol` is `ssh` (or
//! absent, which historically defaulted to ssh); RDP/telnet/raw/serial entries
//! are counted as non-SSH and skipped. The `Default Settings` pseudo-session,
//! which carries an empty `HostName`, is dropped silently.
//!
//! Everything here is hand-rolled byte/line scanning — no xml/regex/ini crate,
//! matching the house style of `termius_csv.rs`.

use std::path::Path;

use anyhow::{Context, Result};

use super::HostImportReport;
use crate::store::{HostSource, LauncherStore, NewHost};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single SSH session extracted from a PuTTY store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PuttyHost {
    pub name: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
}

/// The result of parsing a PuTTY store (a `.reg` export or a sessions dir).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PuttyParse {
    /// SSH sessions with a non-empty name and hostname.
    pub hosts: Vec<PuttyHost>,
    /// Entries skipped because their `Protocol` was not `ssh`.
    pub non_ssh_skipped: usize,
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Hex nibble value of a single ASCII byte, or `None` when not a hex digit.
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Percent-decode a PuTTY session name (`My%20Server` -> `My Server`).
///
/// Decodes `%XX` byte escapes then interprets the result as UTF-8 (lossily).
/// Stray `%` that is not followed by two hex digits is passed through verbatim.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Unescape a `.reg` string value: `\\` -> `\` and `\"` -> `"`.
fn unescape_reg(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Session builder shared by both formats
// ---------------------------------------------------------------------------

/// A parsed `.reg` value: either a string or a `dword` (already masked to u16).
enum RegValue {
    Str(String),
    Dword(u16),
}

impl RegValue {
    fn into_string(self) -> String {
        match self {
            RegValue::Str(s) => s,
            RegValue::Dword(n) => n.to_string(),
        }
    }

    /// Coerce to a port. `dword` values are taken directly; string values are
    /// parsed as decimal (Unix format), defaulting to 22 on garbage.
    fn into_port(self) -> u16 {
        match self {
            RegValue::Dword(n) => n,
            RegValue::Str(s) => s.trim().parse::<u16>().unwrap_or(22),
        }
    }
}

/// Accumulates the four keys we care about for one session, then emits a
/// [`PuttyHost`] (or a skip) into a [`PuttyParse`].
struct SessionBuilder {
    name: String,
    hostname: Option<String>,
    port: Option<u16>,
    protocol: Option<String>,
    username: Option<String>,
}

impl SessionBuilder {
    fn new(name: String) -> Self {
        SessionBuilder {
            name,
            hostname: None,
            port: None,
            protocol: None,
            username: None,
        }
    }

    /// Apply one `Key`/value pair (key matched case-insensitively).
    fn apply(&mut self, key: &str, val: RegValue) {
        match key.trim().to_ascii_lowercase().as_str() {
            "hostname" => self.hostname = Some(val.into_string()),
            "username" => self.username = Some(val.into_string()),
            "protocol" => self.protocol = Some(val.into_string()),
            "portnumber" => self.port = Some(val.into_port()),
            _ => {}
        }
    }

    /// Finalize into `parse`, applying the SSH/HostName filters.
    fn finalize(self, parse: &mut PuttyParse) {
        // A nameless session can't be addressed; drop it silently.
        if self.name.is_empty() {
            return;
        }
        // Protocol filter first, so a non-SSH entry is counted even if its
        // HostName happens to be empty.
        if let Some(proto) = &self.protocol {
            if !proto.trim().eq_ignore_ascii_case("ssh") {
                parse.non_ssh_skipped += 1;
                return;
            }
        }
        let hostname = self.hostname.unwrap_or_default();
        let hostname = hostname.trim();
        // Empty HostName (e.g. "Default Settings") is dropped, NOT counted.
        if hostname.is_empty() {
            return;
        }
        parse.hosts.push(PuttyHost {
            name: self.name,
            hostname: hostname.to_string(),
            port: self.port.unwrap_or(22),
            username: self.username.unwrap_or_default().trim().to_string(),
        });
    }
}

// ---------------------------------------------------------------------------
// Format 1: Windows `.reg` export
// ---------------------------------------------------------------------------

/// Extract the (percent-decoded) session name from a registry key path, or
/// `None` when the path is not under `\PuTTY\Sessions\`.
fn session_name_from_key(path: &str) -> Option<String> {
    if !path.contains(r"\PuTTY\Sessions\") {
        return None;
    }
    let last = path.rsplit('\\').next().unwrap_or("");
    if last.is_empty() {
        return None;
    }
    Some(percent_decode(last))
}

/// Split a `.reg` value line into its quoted key and the raw value text.
///
/// `"HostName"="1.2.3.4"` -> `("HostName", "\"1.2.3.4\"")`
/// `"PortNumber"=dword:00000016` -> `("PortNumber", "dword:00000016")`
fn parse_reg_value_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix('"')?;
    let end = rest.find('"')?;
    let key = &rest[..end];
    let value = rest[end + 1..].strip_prefix('=')?;
    Some((key, value))
}

/// Interpret the raw value text of a `.reg` line.
fn reg_value(raw: &str) -> RegValue {
    let raw = raw.trim();
    if let Some(inner) = raw.strip_prefix('"') {
        let s = inner.strip_suffix('"').unwrap_or(inner);
        RegValue::Str(unescape_reg(s))
    } else if let Some(hex) = raw.strip_prefix("dword:") {
        // Ports are 1..=65535; an out-of-range or non-hex dword falls back to
        // the default 22 rather than silently truncating (e.g. 0x10050 -> 80).
        let port = u32::from_str_radix(hex.trim(), 16)
            .ok()
            .filter(|&n| (1..=u16::MAX as u32).contains(&n))
            .map(|n| n as u16)
            .unwrap_or(22);
        RegValue::Dword(port)
    } else {
        RegValue::Str(raw.to_string())
    }
}

/// Parse a Windows `regedit /e` export (already decoded to a `str`).
pub fn parse_reg(text: &str) -> PuttyParse {
    let mut parse = PuttyParse::default();
    let mut current: Option<SessionBuilder> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') {
            // A new section closes the previous one.
            if let Some(builder) = current.take() {
                builder.finalize(&mut parse);
            }
            let inner = line.trim_start_matches('[').trim_end_matches(']');
            current = session_name_from_key(inner).map(SessionBuilder::new);
            continue;
        }
        if let Some(builder) = current.as_mut() {
            if let Some((key, value)) = parse_reg_value_line(line) {
                builder.apply(key, reg_value(value));
            }
        }
    }
    if let Some(builder) = current.take() {
        builder.finalize(&mut parse);
    }
    parse
}

// ---------------------------------------------------------------------------
// Format 2: Unix ~/.putty/sessions/<name>
// ---------------------------------------------------------------------------

/// Parse a `~/.putty/sessions` directory. Each regular file is one session,
/// its filename (percent-decoded) is the session name, and lines are plain
/// `Key=Value` pairs with a **decimal** `PortNumber`.
///
/// A missing or unreadable directory yields an empty result (no panic).
pub fn parse_sessions_dir(dir: &Path) -> PuttyParse {
    let mut parse = PuttyParse::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return parse;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(percent_decode)
            .unwrap_or_default();

        let mut builder = SessionBuilder::new(name);
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                builder.apply(key, RegValue::Str(value.trim().to_string()));
            }
        }
        builder.finalize(&mut parse);
    }
    parse
}

// ---------------------------------------------------------------------------
// Byte decoding
// ---------------------------------------------------------------------------

/// Decode `.reg` bytes to a `String`, honoring a leading BOM.
///
/// - `EF BB BF` -> UTF-8 (BOM stripped)
/// - `FF FE`    -> UTF-16LE
/// - `FE FF`    -> UTF-16BE
/// - otherwise  -> assumed UTF-8
///
/// All decodes are lossy rather than fallible, and use no external crates.
pub fn decode_reg_bytes(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Import PuTTY sessions into the launcher store.
///
/// - A directory `path` is treated as a `~/.putty/sessions` folder.
/// - Otherwise `path` is read as a `.reg` export (BOM-decoded).
///
/// SSH sessions are inserted as launcher hosts (no tags, `notes = "Imported
/// from PuTTY"`). Existing hosts (matched by name) are skipped, as are entries
/// with an empty name or hostname; non-SSH entries are counted separately.
pub fn import_putty(path: &Path, store: &LauncherStore) -> Result<HostImportReport> {
    let parse = if path.is_dir() {
        parse_sessions_dir(path)
    } else {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        parse_reg(&decode_reg_bytes(&bytes))
    };

    let mut report = HostImportReport {
        skipped_non_ssh: parse.non_ssh_skipped,
        ..Default::default()
    };

    for host in &parse.hosts {
        if host.name.is_empty() || host.hostname.is_empty() {
            continue;
        }
        if store.get_host_by_name(&host.name)?.is_some() {
            report.skipped_existing += 1;
            continue;
        }
        let username = (!host.username.is_empty()).then(|| host.username.clone());
        store.create_host(&NewHost {
            name: host.name.clone(),
            address: host.hostname.clone(),
            port: host.port,
            username,
            notes: Some("Imported from PuTTY".into()),
            source: HostSource::Launcher,
            ..Default::default()
        })?;
        report.imported += 1;
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_REG: &str = r#"Windows Registry Editor Version 5.00

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\My%20Server]
"HostName"="192.168.1.1"
"PortNumber"=dword:00001F90
"Protocol"="ssh"
"UserName"="root"

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Default%20Settings]
"HostName"=""
"Protocol"="ssh"

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\oldbox]
"HostName"="1.2.3.4"
"Protocol"="telnet"
"#;

    #[test]
    fn parse_reg_filters_and_decodes() {
        let parse = parse_reg(SAMPLE_REG);

        // Only "My Server" survives: "Default Settings" (empty HostName) is
        // dropped uncounted, "oldbox" (telnet) is counted as non-SSH.
        assert_eq!(parse.hosts.len(), 1);
        assert_eq!(parse.non_ssh_skipped, 1);

        let h = &parse.hosts[0];
        assert_eq!(h.name, "My Server"); // percent-decoded
        assert_eq!(h.hostname, "192.168.1.1");
        assert_eq!(h.port, 8080); // dword:00001F90 == 0x1F90 == 8080
        assert_eq!(h.username, "root");
    }

    #[test]
    fn parse_reg_defaults_protocol_absent_to_ssh() {
        let text = "[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\noproto]\n\
                    \"HostName\"=\"10.9.8.7\"\n";
        let parse = parse_reg(text);
        assert_eq!(parse.hosts.len(), 1);
        assert_eq!(parse.non_ssh_skipped, 0);
        assert_eq!(parse.hosts[0].hostname, "10.9.8.7");
        assert_eq!(parse.hosts[0].port, 22); // PortNumber absent -> 22
    }

    #[test]
    fn parse_reg_ignores_non_session_keys() {
        // A key that is not under \PuTTY\Sessions\ must not become a host.
        let text = "[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\SshHostKeys]\n\
                    \"HostName\"=\"should.be.ignored\"\n";
        let parse = parse_reg(text);
        assert!(parse.hosts.is_empty());
        assert_eq!(parse.non_ssh_skipped, 0);
    }

    #[test]
    fn decode_reg_bytes_utf16le_with_bom() {
        // FF FE 'a' 'b' 'c' in UTF-16LE.
        let bytes = [0xFF, 0xFE, b'a', 0, b'b', 0, b'c', 0];
        assert_eq!(decode_reg_bytes(&bytes), "abc");
    }

    #[test]
    fn decode_reg_bytes_strips_utf8_bom() {
        let bytes = [0xEF, 0xBB, 0xBF, b'h', b'i'];
        assert_eq!(decode_reg_bytes(&bytes), "hi");
    }

    #[test]
    fn decode_reg_bytes_utf16be_and_edge_cases() {
        // FE FF then 'a' 'b' in UTF-16BE.
        assert_eq!(decode_reg_bytes(&[0xFE, 0xFF, 0, b'a', 0, b'b']), "ab");
        // An odd trailing byte after the BOM is dropped, not mis-read.
        assert_eq!(decode_reg_bytes(&[0xFF, 0xFE, b'a', 0, b'b']), "a");
        // No BOM -> treated as UTF-8.
        assert_eq!(decode_reg_bytes(b"plain"), "plain");
    }

    #[test]
    fn reg_value_dword_port_ranges() {
        // In-range hex is honored.
        assert_eq!(reg_value("dword:00001F90").into_port(), 8080);
        // Out-of-range (> u16::MAX) falls back to 22, not a truncated 80.
        assert_eq!(reg_value("dword:00010050").into_port(), 22);
        // Non-hex garbage falls back to 22.
        assert_eq!(reg_value("dword:zzzz").into_port(), 22);
        // Zero is not a usable port -> 22.
        assert_eq!(reg_value("dword:00000000").into_port(), 22);
    }

    #[test]
    fn unescape_reg_handles_backslashes_and_quotes() {
        assert_eq!(unescape_reg(r"CORP\\jdoe"), r"CORP\jdoe");
        assert_eq!(unescape_reg(r#"say \"hi\""#), r#"say "hi""#);
    }

    #[test]
    fn parse_reg_trims_padded_hostname() {
        let text = "[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\padded]\n\
                    \"HostName\"=\" 10.0.0.1 \"\n\"Protocol\"=\"ssh\"\n";
        let parse = parse_reg(text);
        assert_eq!(parse.hosts.len(), 1);
        assert_eq!(parse.hosts[0].hostname, "10.0.0.1");
    }

    #[test]
    fn parse_sessions_dir_reads_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("webserver"),
            "HostName=10.0.0.5\nPortNumber=2200\nProtocol=ssh\nUserName=deploy\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("legacy"),
            "HostName=10.0.0.6\nProtocol=telnet\n",
        )
        .unwrap();

        let parse = parse_sessions_dir(dir.path());
        assert_eq!(parse.hosts.len(), 1);
        assert_eq!(parse.non_ssh_skipped, 1);

        let h = &parse.hosts[0];
        assert_eq!(h.name, "webserver");
        assert_eq!(h.hostname, "10.0.0.5");
        assert_eq!(h.port, 2200); // decimal in the Unix format
        assert_eq!(h.username, "deploy");
    }

    #[test]
    fn parse_sessions_dir_missing_is_empty() {
        let parse = parse_sessions_dir(Path::new("/nonexistent/putty/sessions/xyz"));
        assert_eq!(parse, PuttyParse::default());
    }

    #[test]
    fn import_putty_inserts_then_dedups() {
        let dir = tempfile::tempdir().unwrap();
        let reg = dir.path().join("putty.reg");
        std::fs::write(&reg, SAMPLE_REG).unwrap();

        let store = LauncherStore::open_in_memory().unwrap();

        let first = import_putty(&reg, &store).unwrap();
        assert_eq!(first.imported, 1);
        assert_eq!(first.skipped_existing, 0);
        assert_eq!(first.skipped_non_ssh, 1);

        let host = store.get_host_by_name("My Server").unwrap().unwrap();
        assert_eq!(host.address, "192.168.1.1");
        assert_eq!(host.port, 8080);
        assert_eq!(host.username.as_deref(), Some("root"));
        assert_eq!(host.notes.as_deref(), Some("Imported from PuTTY"));
        assert_eq!(host.source, HostSource::Launcher);

        // Second run: same session name already present -> skipped_existing.
        let second = import_putty(&reg, &store).unwrap();
        assert_eq!(second.imported, 0);
        assert_eq!(second.skipped_existing, 1);
        assert_eq!(second.skipped_non_ssh, 1);
    }
}
