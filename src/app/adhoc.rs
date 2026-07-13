//! Ad-hoc "connect without saving" target parsing and launch.
//!
//! When the `/` palette query looks like a `[user@]host[:port]` destination
//! that matches none of the saved hosts, the palette offers an extra row that
//! spawns a one-off ssh session. All parsing here is pure and injection-safe:
//! a crafted query can never smuggle ssh options because the destination is
//! validated (no leading `-`, no whitespace/control) and always placed AFTER a
//! `--` end-of-options marker in the argv.

use super::*;

/// A parsed ad-hoc connection target derived from the palette query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdhocTarget {
    /// Optional `user@` prefix.
    pub user: Option<String>,
    /// Host or IP, stored WITHOUT surrounding brackets for IPv6 literals.
    pub host: String,
    /// Optional explicit port (1..=65535).
    pub port: Option<u16>,
    /// True when the host was written as a bracketed IPv6 literal (`[::1]`).
    pub ipv6: bool,
}

impl AdhocTarget {
    /// Human-facing destination label: `user@host` when a user is known,
    /// otherwise just `host`.
    pub fn label(&self) -> String {
        match &self.user {
            Some(u) => format!("{u}@{}", self.host),
            None => self.host.clone(),
        }
    }
}

/// Pure. Parse `"[user@]host[:port]"`, including bracketed IPv6
/// `"[user@][::1]:22"`.
///
/// Returns `None` when the input is not a safe connectable target:
/// empty host, host OR user beginning with `-`, any whitespace/control
/// character anywhere, an `@` inside the host portion, or a port outside
/// `1..=65535`. `host` is stored WITHOUT surrounding brackets; `ipv6=true`
/// records that it came from a bracketed literal.
pub fn parse_adhoc_target(input: &str) -> Option<AdhocTarget> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    // Reject anything with embedded whitespace or control characters — those
    // never belong in a hostname and are a classic smuggling vector.
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }

    // Optional `user@` prefix, split on the FIRST '@'. IPv6 literals never
    // contain '@', so this is unambiguous.
    let (user, rest) = match s.split_once('@') {
        Some((u, r)) => {
            if u.is_empty() || u.starts_with('-') {
                return None;
            }
            (Some(u.to_string()), r)
        }
        None => (None, s),
    };

    if rest.is_empty() {
        return None;
    }
    // A second '@' in the host portion is malformed.
    if rest.contains('@') {
        return None;
    }

    let (host, port, ipv6) = if let Some(after_open) = rest.strip_prefix('[') {
        // Bracketed IPv6: host is between '[' and ']'; a port may follow the
        // closing bracket as ":<port>".
        let close = after_open.find(']')?;
        let host = &after_open[..close];
        let tail = &after_open[close + 1..];
        let port = if tail.is_empty() {
            None
        } else {
            // Only a ":<port>" suffix is valid after the bracket.
            let p = tail.strip_prefix(':')?;
            Some(parse_port(p)?)
        };
        (host.to_string(), port, true)
    } else {
        // Non-bracket form. Accept a ":port" suffix ONLY when the host part has
        // no other ':' — i.e. exactly one ':' total. A bare IPv6 like
        // "fe80::1" (multiple colons, unbracketed) is treated wholly as a host,
        // never as host:port.
        match rest.matches(':').count() {
            0 => (rest.to_string(), None, false),
            1 => {
                let (h, p) = rest.split_once(':').unwrap();
                (h.to_string(), Some(parse_port(p)?), false)
            }
            _ => (rest.to_string(), None, false),
        }
    };

    if host.is_empty() || host.starts_with('-') {
        return None;
    }

    Some(AdhocTarget {
        user,
        host,
        port,
        ipv6,
    })
}

/// Parse a port string, accepting only `1..=65535`.
fn parse_port(s: &str) -> Option<u16> {
    match s.parse::<u16>() {
        Ok(p) if p >= 1 => Some(p),
        _ => None,
    }
}

/// Pure. Build an injection-safe ssh argv for an ad-hoc target.
///
/// Shape: `["ssh", "-v", ("-p", "<port>")?, "--", "<user@host>" | "<host>"]`.
/// The destination is placed AFTER a `--` end-of-options marker so a host that
/// somehow began with `-` (already rejected by the parser) could never be read
/// as an ssh option.
pub fn build_adhoc_argv(t: &AdhocTarget) -> Vec<String> {
    let mut argv = vec!["ssh".to_string(), "-v".to_string()];
    if let Some(port) = t.port {
        argv.push("-p".to_string());
        argv.push(port.to_string());
    }
    argv.push("--".to_string());
    argv.push(t.label());
    argv
}

impl App {
    /// Parse the current palette query as an ad-hoc target, then SUPPRESS it
    /// when it duplicates a saved host: returns `None` if any host entry's
    /// `name()` or `display_name()` equals the target host (case-insensitive)
    /// or equals the raw trimmed query. Used by the palette-rebuild path to
    /// (re)fill `self.palette_adhoc`.
    pub(crate) fn compute_palette_adhoc(&self) -> Option<AdhocTarget> {
        let target = parse_adhoc_target(&self.palette_query)?;
        let host_lc = target.host.to_lowercase();
        let raw_lc = self.palette_query.trim().to_lowercase();
        for entry in &self.hosts {
            let name_lc = entry.name().to_lowercase();
            let disp_lc = entry.display_name().to_lowercase();
            if name_lc == host_lc || disp_lc == host_lc || name_lc == raw_lc || disp_lc == raw_lc {
                return None;
            }
        }
        Some(target)
    }

    /// Leave the palette, build the ssh argv, and spawn an embedded ad-hoc
    /// session. No stored credential is involved (`pending_secret` = `None`).
    pub(crate) fn connect_adhoc(&mut self, target: AdhocTarget) -> Result<()> {
        self.mode = AppMode::Normal;
        let argv = build_adhoc_argv(&target);
        let label = target.label();
        let meta = crate::session::SessionMeta {
            user: target.user.clone(),
            address: Some(target.host.clone()),
            port: target.port.or(Some(22)),
            ..Default::default()
        };
        self.spawn_embedded_session(argv, label.clone(), meta, None, &label)
    }
}

#[cfg(test)]
mod tests {
    use super::{build_adhoc_argv, parse_adhoc_target, AdhocTarget};

    #[test]
    fn parses_user_at_host() {
        let t = parse_adhoc_target("root@example.com").unwrap();
        assert_eq!(t.user.as_deref(), Some("root"));
        assert_eq!(t.host, "example.com");
        assert_eq!(t.port, None);
        assert!(!t.ipv6);
        assert_eq!(t.label(), "root@example.com");
    }

    #[test]
    fn parses_host_port() {
        let t = parse_adhoc_target("example.com:2222").unwrap();
        assert_eq!(t.user, None);
        assert_eq!(t.host, "example.com");
        assert_eq!(t.port, Some(2222));
        assert_eq!(t.label(), "example.com");
    }

    #[test]
    fn parses_user_host_port() {
        let t = parse_adhoc_target("admin@10.0.0.1:22").unwrap();
        assert_eq!(t.user.as_deref(), Some("admin"));
        assert_eq!(t.host, "10.0.0.1");
        assert_eq!(t.port, Some(22));
    }

    #[test]
    fn parses_bare_host() {
        let t = parse_adhoc_target("myhost").unwrap();
        assert_eq!(t.user, None);
        assert_eq!(t.host, "myhost");
        assert_eq!(t.port, None);
    }

    #[test]
    fn parses_bracketed_ipv6_with_port() {
        let t = parse_adhoc_target("[::1]:22").unwrap();
        assert_eq!(t.host, "::1");
        assert_eq!(t.port, Some(22));
        assert!(t.ipv6);
    }

    #[test]
    fn parses_bracketed_ipv6_no_port() {
        let t = parse_adhoc_target("[2001:db8::1]").unwrap();
        assert_eq!(t.host, "2001:db8::1");
        assert_eq!(t.port, None);
        assert!(t.ipv6);
    }

    #[test]
    fn parses_bracketed_ipv6_with_user() {
        let t = parse_adhoc_target("root@[fe80::1]:2200").unwrap();
        assert_eq!(t.user.as_deref(), Some("root"));
        assert_eq!(t.host, "fe80::1");
        assert_eq!(t.port, Some(2200));
        assert!(t.ipv6);
        assert_eq!(t.label(), "root@fe80::1");
    }

    #[test]
    fn bare_unbracketed_ipv6_is_host_not_host_port() {
        // Multiple colons, no brackets: the whole thing is the host, never
        // split into host:port.
        let t = parse_adhoc_target("fe80::1").unwrap();
        assert_eq!(t.host, "fe80::1");
        assert_eq!(t.port, None);
        assert!(!t.ipv6);
    }

    #[test]
    fn rejects_leading_dash_host() {
        assert!(parse_adhoc_target("-oProxyCommand=x").is_none());
        assert!(parse_adhoc_target("-lroot").is_none());
    }

    #[test]
    fn rejects_leading_dash_user() {
        assert!(parse_adhoc_target("-bad@host").is_none());
    }

    #[test]
    fn rejects_whitespace() {
        assert!(parse_adhoc_target("host name").is_none());
        assert!(parse_adhoc_target("root@ho st").is_none());
        assert!(parse_adhoc_target("host\tx").is_none());
    }

    #[test]
    fn rejects_empty_and_control() {
        assert!(parse_adhoc_target("").is_none());
        assert!(parse_adhoc_target("   ").is_none());
        assert!(parse_adhoc_target("ho\u{0007}st").is_none());
    }

    #[test]
    fn rejects_bad_port() {
        assert!(parse_adhoc_target("host:0").is_none());
        assert!(parse_adhoc_target("host:70000").is_none());
        assert!(parse_adhoc_target("host:abc").is_none());
        assert!(parse_adhoc_target("host:").is_none());
        assert!(parse_adhoc_target("[::1]:99999").is_none());
    }

    #[test]
    fn rejects_empty_user_and_empty_host() {
        assert!(parse_adhoc_target("@host").is_none());
        assert!(parse_adhoc_target("root@").is_none());
    }

    #[test]
    fn rejects_double_at() {
        assert!(parse_adhoc_target("a@b@c").is_none());
    }

    #[test]
    fn build_argv_places_double_dash_before_destination() {
        let t = AdhocTarget {
            user: Some("root".into()),
            host: "example.com".into(),
            port: None,
            ipv6: false,
        };
        let argv = build_adhoc_argv(&t);
        assert_eq!(argv[0], "ssh");
        assert_eq!(argv[1], "-v");
        let dd = argv.iter().position(|a| a == "--").unwrap();
        let dest = argv.iter().position(|a| a == "root@example.com").unwrap();
        assert!(dd < dest, "`--` must precede the destination");
        // No `-p` when there is no port.
        assert!(!argv.iter().any(|a| a == "-p"));
    }

    #[test]
    fn build_argv_includes_port_flag_only_with_port() {
        let t = AdhocTarget {
            user: None,
            host: "10.0.0.1".into(),
            port: Some(2222),
            ipv6: false,
        };
        let argv = build_adhoc_argv(&t);
        let p = argv.iter().position(|a| a == "-p").unwrap();
        assert_eq!(argv[p + 1], "2222");
        let dd = argv.iter().position(|a| a == "--").unwrap();
        // `-p <port>` comes before the `--` marker.
        assert!(p < dd);
        assert_eq!(argv.last().unwrap(), "10.0.0.1");
    }

    #[test]
    fn build_argv_ipv6_destination_has_no_brackets() {
        let t = parse_adhoc_target("[::1]:22").unwrap();
        let argv = build_adhoc_argv(&t);
        assert_eq!(argv.last().unwrap(), "::1");
        assert!(argv.iter().any(|a| a == "--"));
    }
}
