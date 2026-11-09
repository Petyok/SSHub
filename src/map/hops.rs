//! Pure helpers behind the hop editor: the comma separated ProxyJump string as
//! a list, raw hop validation, routes already in use, and the hosts a hop
//! must not be (cycle guard).

use std::collections::HashSet;

use super::{is_jump_none, parse_raw_hop, HopKey, Map};

/// The hop tokens of a stored ProxyJump value. `none` (direct) and an empty
/// value are both an empty list.
pub fn split(value: &str) -> Vec<String> {
    if is_jump_none(value) {
        return Vec::new();
    }
    value
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

/// Back to the stored form: comma separated, no spaces.
pub fn join(hops: &[String]) -> String {
    hops.join(",")
}

/// A hop typed as a raw `[user@]host[:port]`: the canonical token to store, or
/// why it cannot be one. Reuses the ad-hoc connect parser, so an option-like
/// token (leading `-`) or one with whitespace is refused, and a comma would
/// split it into two hops.
pub fn validate_raw(input: &str) -> Result<String, &'static str> {
    let input = input.trim();
    if input.is_empty() {
        return Err("type [user@]host[:port]");
    }
    if input.contains(',') {
        return Err("one hop at a time, no commas");
    }
    match parse_raw_hop(input) {
        Some(raw) => Ok(raw.label()),
        None => Err("not a valid [user@]host[:port]"),
    }
}

/// Why a stored token cannot be saved as a hop, if it cannot.
pub fn token_issue(token: &str) -> Option<&'static str> {
    parse_raw_hop(token)
        .is_none()
        .then_some("not a valid [user@]host[:port]")
}

/// Why a stored ProxyJump value must not reach `ssh -J`, if it must not.
pub fn jump_problem(value: &str) -> Option<String> {
    split(value)
        .into_iter()
        .find_map(|t| token_issue(&t).map(|e| format!("ProxyJump hop {t:?}: {e}")))
}

/// Whether every hop of a stored ProxyJump value is one valid
/// `[user@]host[:port]` (`none` and empty are valid). Anything else must not
/// be handed to `ssh -J`.
pub fn jump_is_valid(value: &str) -> bool {
    split(value).iter().all(|t| token_issue(t).is_none())
}

/// A route already in use: the hops from you to some hop, and how many hosts
/// sit behind its last hop at any depth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub tokens: Vec<String>,
    pub hosts: usize,
}

/// Distinct chains in use, biggest first (`limit` of them). Chains through a
/// ProxyCommand or an invalid hop are not reusable and are skipped.
pub fn routes_in_use(map: &Map, limit: usize) -> Vec<Route> {
    let mut routes: Vec<Route> = Vec::new();
    for n in 0..map.nodes.len() {
        let mut tokens = Vec::new();
        let mut ok = true;
        for p in map.node_path(n) {
            match &map.nodes[p].key {
                HopKey::Saved(h) => tokens.push(map.hosts[*h].name.clone()),
                HopKey::Raw(r) => tokens.push(r.label()),
                HopKey::ProxyCmd(_) | HopKey::Invalid(_) => ok = false,
            }
        }
        if ok && !routes.iter().any(|r| r.tokens == tokens) {
            routes.push(Route {
                tokens,
                hosts: map.nodes[n].sigma,
            });
        }
    }
    routes.sort_by(|a, b| b.hosts.cmp(&a.hosts).then_with(|| a.tokens.cmp(&b.tokens)));
    routes.truncate(limit);
    routes
}

/// Names that must not become a hop of `edited`: the host itself and every
/// ssh_config host that already depends on it (ssh follows such a hop's own
/// `ProxyJump`, so it would close a loop), including loops that are already
/// there. A launcher host is expanded to `user@addr` and its own jump is never
/// followed (`Map::build`'s `expand`), so it can never close a loop.
pub fn forbidden_hops(map: &Map, edited: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    out.insert(edited.to_string());
    if let Some(h) = map.hosts.iter().position(|h| h.name == edited) {
        for d in map
            .blast_radius(&HopKey::Saved(h))
            .into_iter()
            .filter(|&d| map.hosts[d].ssh_config)
        {
            out.insert(map.hosts[d].name.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{JumpSource, MapHost};

    fn host(name: &str, jump: Option<&str>, ssh_config: bool) -> MapHost {
        MapHost {
            name: name.into(),
            address: format!("{name}.x"),
            port: 22,
            group: "g".into(),
            jump: jump.map(String::from),
            jump_source: jump.map(|_| JumpSource::Own),
            ssh_config,
            ..Default::default()
        }
    }

    #[test]
    fn split_join_none_and_blanks() {
        assert_eq!(split("a, b ,,c"), ["a", "b", "c"]);
        assert_eq!(split("none"), Vec::<String>::new());
        assert_eq!(split("  "), Vec::<String>::new());
        assert_eq!(join(&split("a, b")), "a,b");
    }

    #[test]
    fn raw_validation_refuses_option_like_and_multi_hop_input() {
        assert_eq!(
            validate_raw("ops@10.0.0.1:2222").unwrap(),
            "ops@10.0.0.1:2222"
        );
        assert_eq!(validate_raw("[::1]:22").unwrap(), "[::1]:22");
        for bad in [
            "-oProxyCommand=id",
            "ops@-oProxyCommand=id",
            "a b",
            "a,b",
            "",
            "host:99999",
            "host:0",
        ] {
            assert!(validate_raw(bad).is_err(), "{bad:?} must be refused");
        }
        assert!(token_issue("bastion-eu").is_none());
        assert!(token_issue("-J").is_some());
    }

    #[test]
    fn cycle_guard_names_the_host_and_everything_that_depends_on_it() {
        // c jumps via b, b via a (ssh_config hosts chain): a must not take b or c.
        let map = Map::build(vec![
            host("a", None, true),
            host("b", Some("a"), true),
            host("c", Some("b"), true),
            host("free", None, false),
        ]);
        let f = forbidden_hops(&map, "a");
        assert!(f.contains("a") && f.contains("b") && f.contains("c"));
        assert!(!f.contains("free"));
        // A brand new host (not in the map yet) only forbids itself.
        assert_eq!(forbidden_hops(&map, "new").len(), 1);
    }

    #[test]
    fn launcher_dependents_are_not_loops_but_ssh_config_ones_are() {
        // Launcher `d` jumps via `e`; `e` may take `d` as a hop: ssh gets
        // `d`'s user@addr and never follows d's own jump.
        let map = Map::build(vec![
            host("e", None, false),
            host("d", Some("e"), false),
            host("s", Some("e"), true),
        ]);
        let f = forbidden_hops(&map, "e");
        assert!(f.contains("e") && f.contains("s"));
        assert!(!f.contains("d"));
        // Oracle: the map really reports a cycle for the ssh_config one only.
        let mut hs = vec![
            host("e", Some("d"), false),
            host("d", Some("e"), false),
            host("s", Some("e"), true),
        ];
        assert!(matches!(
            Map::build(hs.clone()).chains[0],
            crate::map::Chain::Hops(_)
        ));
        hs[0].jump = Some("s".into());
        assert!(matches!(
            Map::build(hs).chains[0],
            crate::map::Chain::Cycle(_)
        ));
    }

    #[test]
    fn routes_in_use_are_distinct_and_biggest_first() {
        let map = Map::build(vec![
            host("bastion", None, false),
            host("jump", Some("bastion"), true),
            host("db-1", Some("jump"), false),
            host("db-2", Some("jump"), false),
            host("web", Some("bastion"), false),
        ]);
        let routes = routes_in_use(&map, 8);
        assert_eq!(routes[0].tokens, ["bastion"]);
        assert_eq!(routes[0].hosts, 4);
        assert_eq!(routes[1].tokens, ["bastion", "jump"]);
        assert!(routes.iter().all(|r| !r.tokens.is_empty()));
        let mut seen = HashSet::new();
        assert!(routes.iter().all(|r| seen.insert(r.tokens.clone())));
    }

    /// `ssh -G -J <token>` succeeds.
    fn ssh_takes(token: &str) -> bool {
        std::process::Command::new("ssh")
            .args(["-F", "/dev/null", "-G", "-J", token, "x"])
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// Tokens some ssh takes that this parser still refuses, on purpose: an
    /// empty port (`h:`) is a typo here, not a hop.
    const STRICTER: &[&str] = &["h:", HOSTILE];

    /// ssh takes this, and hands the user to a shell: it splits into `-l a` and
    /// an injected `-oProxyCommand=...`. We must refuse it whatever ssh says.
    const HOSTILE: &str = "a$IFS-oProxyCommand=touch\\$IFS/tmp/x@127.0.0.1:1";

    /// Characters outside what a login or host name is made of. ssh takes some
    /// of them (`$`, `=`, `/`, `\\`) and hands the user to a shell, so we refuse
    /// them whatever the local ssh says; a leading `-` likewise. A login may
    /// hold any letter (`Администратор`), a host name ASCII only.
    fn outside_the_allowlist(token: &str) -> bool {
        let rest = token.strip_prefix("ssh://").unwrap_or(token);
        let (user, host) = rest.rsplit_once('@').unwrap_or(("", rest));
        user.chars()
            .any(|c| !(c.is_alphanumeric() || ".+-_%@".contains(c)))
            || host
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || ".-_%:[]".contains(c)))
            || rest.starts_with('-')
            || rest.split('@').any(|p| p.starts_with('-'))
    }

    #[test]
    fn oracle_a_token_we_accept_is_a_token_ssh_dash_j_accepts() {
        if std::process::Command::new("ssh")
            .arg("-V")
            .output()
            .is_err()
        {
            crate::test_support::tool_missing("ssh");
            return;
        }
        for token in [
            "bastion",
            "ops@10.0.0.1:2222",
            "[::1]:22",
            "a@[::1]:2",
            "john@corp.com@10.0.0.1",
            "a@b@c@d",
            "ssh://u@h:22",
            "ssh://h",
            "h:",
            "-oProxyCommand=id",
            "ops@-oProxyCommand=id",
            "a b",
            "host:99999",
            "host:0",
            "@h",
            "u@",
            "[::1",
            "[::1]x",
            "::1",
            "u@h:22:33",
            "h;id",
            "h$(id)",
            "u;x@h",
            "-h",
            "h\tx",
            HOSTILE,
            "u@$HOME",
            "a=b@h",
            "a/b@h",
            "ops.dev+x%y@h-1.example:22",
            "Администратор@10.0.0.245:22",
            "u@é",
        ] {
            let ours = token_issue(token).is_none();
            let theirs = ssh_takes(token);
            // Never hand ssh a hop it would refuse...
            assert!(!ours || theirs, "we accept {token:?}, ssh -J refuses it");
            // ...and refuse only what ssh refuses, bar the deliberate cases.
            assert!(
                ours || !theirs || STRICTER.contains(&token) || outside_the_allowlist(token),
                "we refuse {token:?}, which ssh -J takes"
            );
        }
    }

    #[test]
    fn we_refuse_the_shell_injecting_user_even_though_ssh_takes_it() {
        if std::process::Command::new("ssh")
            .arg("-V")
            .output()
            .is_err()
        {
            crate::test_support::tool_missing("ssh");
            return;
        }
        // Newer OpenSSH refuses it too (9.6 added checks); older takes it and
        // hands the user to a shell. Either way we must refuse.
        if !ssh_takes(HOSTILE) {
            eprintln!("note: this ssh refuses the hostile hop itself");
        }
        assert!(token_issue(HOSTILE).is_some());
        assert!(validate_raw(HOSTILE).is_err());
        assert!(!jump_is_valid(&format!("ok,{HOSTILE}")));
        assert!(jump_problem(HOSTILE).is_some());
    }

    #[test]
    fn the_user_is_everything_before_the_last_at_sign() {
        let hop = parse_raw_hop("john@corp.com@10.0.0.1:2200").unwrap();
        assert_eq!(hop.user.as_deref(), Some("john@corp.com"));
        assert_eq!((hop.host.as_str(), hop.port), ("10.0.0.1", Some(2200)));
        assert_eq!(hop.label(), "john@corp.com@10.0.0.1:2200");
        assert!(parse_raw_hop("-x@a@b").is_none());
        assert!(parse_raw_hop("a@b c@d").is_none());
        assert!(parse_raw_hop("a@b@").is_none());
        // Allowlist: a login or host name, nothing a shell would expand.
        for bad in [
            HOSTILE, "u@$HOME", "a=b@h", "a/b@h", "a\\b@h", "u@h$x", "u@h/x", "u@é",
        ] {
            assert!(parse_raw_hop(bad).is_none(), "{bad:?}");
        }
        for good in [
            "ops.dev+x%y@h-1.example:22",
            "john@corp.com@10.0.0.1",
            "é@h",
            "Администратор@10.0.0.245:22",
            "[::1]:22",
            "a@[fe80::1%eth0]:2",
        ] {
            assert!(parse_raw_hop(good).is_some(), "{good:?}");
        }
    }
}
