//! Pure parsing of remote OS probe output into a canonical OS id.
//!
//! The probe runs `cat /etc/os-release 2>/dev/null || uname -s` on the remote
//! host. This module maps that raw text onto one of the canonical ids used for
//! logo lookup (see [`crate::osinfo::logos`]) and stored in `hosts.os_icon`.

use super::CanonicalOs;

/// Parse remote probe output into a canonical OS id.
///
/// Scans for an `ID=` / `ID_LIKE=` line from `/etc/os-release` (stripping
/// surrounding single/double quotes), applying the alias table below. If no
/// `ID` is present, falls back to interpreting a bare `uname -s` line. Returns
/// `None` for anything unrecognized.
pub fn parse_os(output: &str) -> Option<CanonicalOs> {
    let mut id: Option<&str> = None;
    let mut id_like: Option<String> = None;

    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("ID=") {
            id = Some(unquote(rest));
        } else if let Some(rest) = line.strip_prefix("ID_LIKE=") {
            id_like = Some(unquote(rest).to_string());
        }
    }

    if let Some(id) = id {
        return map_os_release_id(id, id_like.as_deref());
    }

    // No os-release ID line — fall back to a bare `uname -s` token.
    map_uname(output)
}

/// Strip a single pair of surrounding single or double quotes.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Map an `/etc/os-release` `ID` (with optional `ID_LIKE`) to a canonical id.
fn map_os_release_id(id: &str, id_like: Option<&str>) -> Option<CanonicalOs> {
    let id = id.trim().to_ascii_lowercase();

    let mapped: Option<CanonicalOs> = match id.as_str() {
        "arch" => Some("arch"),
        "ubuntu" => Some("ubuntu"),
        "debian" => Some("debian"),
        "alpine" => Some("alpine"),
        "fedora" => Some("fedora"),
        "rocky" => Some("rocky"),
        "rhel" => Some("rhel"),
        "centos" => Some("centos"),
        "almalinux" => Some("almalinux"),
        "linuxmint" => Some("linuxmint"),
        "manjaro" => Some("manjaro"),
        "pop" => Some("popos"),
        "kali" => Some("kali"),
        "gentoo" => Some("gentoo"),
        "void" => Some("void"),
        "nixos" => Some("nixos"),
        "endeavouros" => Some("endeavouros"),
        "freebsd" => Some("freebsd"),
        "macos" => Some("macos"),
        "windows" => Some("windows"),
        s if s.starts_with("opensuse") => Some("opensuse"),
        _ => None,
    };

    if let Some(m) = mapped {
        return Some(m);
    }

    // Unknown ID — consult ID_LIKE for a family fallback.
    if let Some(like) = id_like {
        return map_id_like(like);
    }

    None
}

/// Derive a canonical id from a (possibly space-separated) `ID_LIKE` value.
fn map_id_like(id_like: &str) -> Option<CanonicalOs> {
    let like = id_like.to_ascii_lowercase();
    for token in like.split_whitespace() {
        let mapped: Option<CanonicalOs> = match token {
            "rhel" => Some("rhel"),
            "centos" => Some("centos"),
            "fedora" => Some("fedora"),
            "debian" => Some("debian"),
            "ubuntu" => Some("ubuntu"),
            "arch" => Some("arch"),
            "suse" | "opensuse" => Some("opensuse"),
            _ => None,
        };
        if mapped.is_some() {
            return mapped;
        }
    }
    // Generic Linux family without a specific match.
    if like.contains("linux") {
        return Some("linux");
    }
    None
}

/// Map a bare `uname -s` line to a canonical id.
fn map_uname(output: &str) -> Option<CanonicalOs> {
    for line in output.lines() {
        match line.trim() {
            "Darwin" => return Some("macos"),
            "FreeBSD" => return Some("freebsd"),
            "Linux" => return Some("linux"),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_arch() {
        let out = "NAME=\"Arch Linux\"\nID=arch\nPRETTY_NAME=\"Arch Linux\"\n";
        assert_eq!(parse_os(out), Some("arch"));
    }

    #[test]
    fn parses_ubuntu_quoted() {
        let out = "ID=ubuntu\nID_LIKE=debian\n";
        assert_eq!(parse_os(out), Some("ubuntu"));
    }

    #[test]
    fn strips_double_quotes() {
        assert_eq!(parse_os("ID=\"debian\"\n"), Some("debian"));
    }

    #[test]
    fn strips_single_quotes() {
        assert_eq!(parse_os("ID='fedora'\n"), Some("fedora"));
    }

    #[test]
    fn pop_maps_to_popos() {
        assert_eq!(
            parse_os("ID=pop\nID_LIKE=\"ubuntu debian\"\n"),
            Some("popos")
        );
    }

    #[test]
    fn linuxmint_stays() {
        assert_eq!(parse_os("ID=linuxmint\n"), Some("linuxmint"));
    }

    #[test]
    fn opensuse_leap_collapses() {
        assert_eq!(parse_os("ID=opensuse-leap\n"), Some("opensuse"));
    }

    #[test]
    fn opensuse_tumbleweed_collapses() {
        assert_eq!(parse_os("ID=opensuse-tumbleweed\n"), Some("opensuse"));
    }

    #[test]
    fn rhel_family_keeps_own_id() {
        assert_eq!(parse_os("ID=rhel\n"), Some("rhel"));
        assert_eq!(parse_os("ID=centos\n"), Some("centos"));
        assert_eq!(parse_os("ID=rocky\n"), Some("rocky"));
    }

    #[test]
    fn almalinux_keeps_own_id() {
        // almalinux has no logo but still maps to a canonical id string.
        assert_eq!(parse_os("ID=almalinux\n"), Some("almalinux"));
    }

    #[test]
    fn unknown_id_falls_back_to_id_like() {
        assert_eq!(
            parse_os("ID=mydistro\nID_LIKE=\"rhel fedora\"\n"),
            Some("rhel")
        );
    }

    #[test]
    fn unknown_id_like_linux_family() {
        assert_eq!(parse_os("ID=mydistro\nID_LIKE=linux\n"), Some("linux"));
    }

    #[test]
    fn uname_darwin() {
        assert_eq!(parse_os("Darwin\n"), Some("macos"));
    }

    #[test]
    fn uname_freebsd() {
        assert_eq!(parse_os("FreeBSD\n"), Some("freebsd"));
    }

    #[test]
    fn uname_linux() {
        assert_eq!(parse_os("Linux\n"), Some("linux"));
    }

    #[test]
    fn unknown_returns_none() {
        assert_eq!(parse_os("SunOS\n"), None);
        assert_eq!(parse_os(""), None);
        assert_eq!(parse_os("garbage output"), None);
    }

    #[test]
    fn os_release_wins_over_uname() {
        // Combined output: os-release ID takes precedence over any trailing token.
        let out = "ID=debian\nLinux\n";
        assert_eq!(parse_os(out), Some("debian"));
    }
}
