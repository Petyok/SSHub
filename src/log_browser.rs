//! Read-only browser over the session-log files written by [`crate::session_log`].
//!
//! Logs live under `<data_dir>/logs/<host-dir>/<secs>-<pid>-<id>[-serial].log`.
//! This module enumerates those hosts and segments, reads a bounded slice of a
//! segment for viewing, strips terminal control sequences so a raw PTY
//! transcript renders as plain text, and does a simple case-insensitive search.
//! Large transcripts are never slurped whole: reads are capped.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Cap on bytes read from one segment into the viewer. Segments rotate at the
/// configured size (default 10 MiB); the viewer reads at most this much.
pub const VIEWER_READ_CAP: u64 = 4 * 1024 * 1024;

/// One host directory under `logs/` that holds at least one segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogHost {
    /// Directory name, e.g. `web-3` (managed) or `bastion` (config alias).
    pub dir_name: String,
    pub segment_count: usize,
    pub total_bytes: u64,
    /// Newest segment start time (unix secs), parsed from the file name.
    pub latest_secs: Option<u64>,
}

/// One rotated log segment file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogSegment {
    pub path: PathBuf,
    pub file_name: String,
    pub bytes: u64,
    pub started_secs: Option<u64>,
}

/// The `logs/` root under a profile data dir.
pub fn logs_root(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// Enumerate host directories that contain at least one `.log` segment,
/// newest-active first.
pub fn list_log_hosts(logs_root: &Path) -> Vec<LogHost> {
    let mut hosts = Vec::new();
    let Ok(entries) = fs::read_dir(logs_root) else {
        return hosts;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let segs = list_segments(&path);
        if segs.is_empty() {
            continue;
        }
        hosts.push(LogHost {
            dir_name: entry.file_name().to_string_lossy().into_owned(),
            segment_count: segs.len(),
            total_bytes: segs.iter().map(|s| s.bytes).sum(),
            latest_secs: segs.iter().filter_map(|s| s.started_secs).max(),
        });
    }
    hosts.sort_by(|a, b| {
        b.latest_secs
            .cmp(&a.latest_secs)
            .then_with(|| a.dir_name.cmp(&b.dir_name))
    });
    hosts
}

/// List a host directory's `.log` segments, newest first.
pub fn list_segments(host_dir: &Path) -> Vec<LogSegment> {
    let mut segs = Vec::new();
    let Ok(entries) = fs::read_dir(host_dir) else {
        return segs;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".log") {
            continue;
        }
        segs.push(LogSegment {
            path: entry.path(),
            bytes: entry.metadata().map(|m| m.len()).unwrap_or(0),
            started_secs: parse_started_secs(&name),
            file_name: name,
        });
    }
    segs.sort_by(|a, b| {
        b.started_secs
            .cmp(&a.started_secs)
            .then_with(|| b.file_name.cmp(&a.file_name))
    });
    segs
}

/// Parse the leading unix-seconds prefix from a segment file name
/// (`<secs>-<pid>-<open_id>[-serial].log`).
pub fn parse_started_secs(file_name: &str) -> Option<u64> {
    file_name
        .split(['-', '.'])
        .next()
        .and_then(|s| s.parse().ok())
}

/// Read up to `cap` bytes of `path`, strip terminal control sequences, and
/// return display lines plus whether the file was longer than `cap`.
///
/// Returns `None` when the file is missing or unreadable, so callers can tell a
/// genuinely empty segment from one that has been pruned or cannot be opened.
pub fn read_segment_lines(path: &Path, cap: u64) -> Option<(Vec<String>, bool)> {
    let file = fs::File::open(path).ok()?;
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut buf = Vec::new();
    file.take(cap).read_to_end(&mut buf).ok()?;
    // Drop a trailing partial multibyte char left by the byte cap so it does not
    // render as U+FFFD.
    let valid = match std::str::from_utf8(&buf) {
        Ok(_) => buf.len(),
        Err(e) => e.valid_up_to(),
    };
    let text = String::from_utf8_lossy(&buf[..valid]);
    let clean = strip_ansi(&text);
    // A single trailing newline would otherwise add one phantom empty line and
    // throw off the "line N/N" count.
    let body = clean.strip_suffix('\n').unwrap_or(&clean);
    let lines = body.split('\n').map(|l| l.to_string()).collect();
    Some((lines, len > cap))
}

/// Whether `name` is a single `*.log` path component safe to join onto the logs
/// root: no path separators, no `..`, and the `.log` suffix a segment always
/// carries. Guards the bookmark-stored file name against escaping the directory.
pub fn is_safe_segment_name(name: &str) -> bool {
    !name.is_empty()
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && name.ends_with(".log")
}

/// Case-insensitive substring search; returns matching line indices in order.
pub fn search_lines(lines: &[String], query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.to_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect()
}

/// Strip ANSI/VT escape sequences and stray control bytes so a raw PTY
/// transcript renders as plain text. Tabs and newlines survive; carriage
/// returns and other C0 controls are dropped.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: params/intermediates up to a final byte 0x40..=0x7e.
                Some('[') => {
                    for f in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&f) {
                            break;
                        }
                    }
                }
                // String sequences (OSC / DCS / APC / PM / SOS) carry a payload
                // terminated by BEL (0x07) or ST (ESC \); drop the whole thing
                // so e.g. a tmux DCS passthrough does not spill into the view.
                Some(']') | Some('P') | Some('_') | Some('^') | Some('X') => {
                    while let Some(f) = chars.next() {
                        if f == '\u{07}' {
                            break;
                        }
                        if f == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                // Charset-select escapes take one more byte.
                Some('(') | Some(')') => {
                    chars.next();
                }
                // Any other two-char escape (ESC 7/8/M/D/E/H/=, …): the second
                // byte was already consumed by `chars.next()` above.
                _ => {}
            },
            '\n' | '\t' => out.push(c),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_seg(dir: &Path, name: &str, body: &[u8]) {
        fs::create_dir_all(dir).unwrap();
        let mut f = fs::File::create(dir.join(name)).unwrap();
        f.write_all(body).unwrap();
    }

    #[test]
    fn strip_ansi_removes_escapes_and_controls() {
        let raw = "\u{1b}[31mred\u{1b}[0m\ttab\r\nplain\u{1b}]0;title\u{07}done";
        assert_eq!(strip_ansi(raw), "red\ttab\nplaindone");
    }

    #[test]
    fn parse_started_secs_reads_prefix() {
        assert_eq!(parse_started_secs("1700000000-42-0.log"), Some(1700000000));
        assert_eq!(
            parse_started_secs("1700000000-42-0-3.log"),
            Some(1700000000)
        );
        assert_eq!(parse_started_secs("weird.log"), None);
    }

    #[test]
    fn lists_hosts_and_segments_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_seg(&root.join("web-1"), "1000-1-0.log", b"hello\n");
        write_seg(&root.join("web-1"), "2000-1-0.log", b"world\nagain\n");
        write_seg(&root.join("db-2"), "1500-1-0.log", b"db\n");
        // A dir with no .log segments is skipped.
        fs::create_dir_all(root.join("empty")).unwrap();

        let hosts = list_log_hosts(root);
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].dir_name, "web-1"); // latest 2000 > db-2's 1500
        assert_eq!(hosts[0].segment_count, 2);
        assert_eq!(hosts[1].dir_name, "db-2");

        let segs = list_segments(&root.join("web-1"));
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].file_name, "2000-1-0.log"); // newest first
        assert_eq!(segs[1].file_name, "1000-1-0.log");
    }

    #[test]
    fn read_and_search_segment() {
        let tmp = tempfile::tempdir().unwrap();
        let seg = tmp.path().join("100-1-0.log");
        {
            let mut f = fs::File::create(&seg).unwrap();
            f.write_all(b"\x1b[32m$ ls -la\x1b[0m\nfile-a\nFILE-B\n")
                .unwrap();
        }
        let (lines, truncated) = read_segment_lines(&seg, VIEWER_READ_CAP).unwrap();
        assert!(!truncated);
        assert_eq!(lines[0], "$ ls -la");
        assert_eq!(lines[1], "file-a");
        // The trailing newline does not add a phantom empty line.
        assert_eq!(lines.len(), 3);

        let hits = search_lines(&lines, "file");
        assert_eq!(hits, vec![1, 2]); // case-insensitive
        assert!(search_lines(&lines, "").is_empty());
    }

    #[test]
    fn read_marks_truncation_past_the_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let seg = tmp.path().join("100-1-0.log");
        {
            let mut f = fs::File::create(&seg).unwrap();
            f.write_all(&[b'a'; 100]).unwrap();
        }
        let (lines, truncated) = read_segment_lines(&seg, 10).unwrap();
        assert!(truncated);
        assert_eq!(lines[0].len(), 10);
    }

    #[test]
    fn safe_segment_name_rejects_path_escapes() {
        assert!(is_safe_segment_name("1700000000-1-0.log"));
        assert!(!is_safe_segment_name("../secret.log"));
        assert!(!is_safe_segment_name("/etc/passwd"));
        assert!(!is_safe_segment_name("sub/dir.log"));
        assert!(!is_safe_segment_name("notalog.txt"));
        assert!(!is_safe_segment_name(""));
    }

    #[test]
    fn read_missing_segment_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_segment_lines(&tmp.path().join("gone.log"), VIEWER_READ_CAP).is_none());
    }

    #[test]
    fn strip_ansi_drops_dcs_payload() {
        // A DCS string (ESC P … ST) must not leak its payload into the view.
        let raw = "before\u{1b}Psome-dcs-payload\u{1b}\\after";
        assert_eq!(strip_ansi(raw), "beforeafter");
        // APC (ESC _ … ST) likewise.
        let apc = "a\u{1b}_deadbeef\u{1b}\\b";
        assert_eq!(strip_ansi(apc), "ab");
    }
}
