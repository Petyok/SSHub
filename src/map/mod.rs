//! Network map model (issue #79): the jump-host forest behind the map tab.
//!
//! Pure data, no UI. A host's *chain* is the ordered list of hops `ssh` takes
//! to reach it. Edges are real jumps only: the host's own ProxyJump, a group
//! default it inherits, or an ssh_config `ProxyJump` (as reported by
//! `ssh -G`). Groups and tags never create an edge.
//!
//! Chain rule (matches what `ssh -J` does, checked against `ssh -v`, and
//! `LauncherStore::expand_jump`): the first token of a host's jump list also
//! brings that hop's own chain when the hop is an ssh_config host (ssh itself
//! follows its `ProxyJump`); every later token is pinned by the `-J` list. A
//! launcher hop is expanded to `user@addr`, so its own jump setting is never
//! followed. That is why one hop can sit under two parents. `ProxyJump none` is an empty chain.
//!
//! Hop tokens are untrusted text (imports write them): anything
//! `parse_adhoc_target` refuses, such as a leading `-`, is an `Invalid` hop,
//! never a connectable one. A `ProxyCommand` is one opaque hop with a fixed
//! label; its text is never stored or shown.

mod build;
pub mod dot;
pub mod filter;
pub mod hops;

pub use build::collect_hosts;
pub use filter::MapFilter;

use std::collections::{BTreeMap, HashMap, HashSet};

/// Where one hop of a chain was configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JumpSource {
    /// The host's own ProxyJump field.
    Own,
    /// A group default; carries the group's display path.
    Inherited { group: String },
    /// `ProxyJump` from ssh_config (via `ssh -G`).
    SshConfig,
}

/// One host as the map sees it: effective values, inheritance already applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapHost {
    pub name: String,
    pub address: String,
    pub user: Option<String>,
    pub port: u16,
    /// Group display path (`prod/db`), `_ungrouped` when none.
    pub group: String,
    pub tags: Vec<String>,
    /// Effective ProxyJump list (comma separated), `None` when direct.
    pub jump: Option<String>,
    pub jump_source: Option<JumpSource>,
    pub ssh_config: bool,
    /// ssh_config sets a `ProxyCommand`: an opaque hop, never parsed.
    pub proxy_command: bool,
}

/// A hop token that is not a saved host: `[user@]host[:port]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RawHop {
    pub user: Option<String>,
    /// Without brackets for an IPv6 literal.
    pub host: String,
    pub port: Option<u16>,
    pub ipv6: bool,
}

impl RawHop {
    /// `user@host:port` as `ssh -J` takes it (IPv6 hosts bracketed).
    pub fn label(&self) -> String {
        let mut s = String::new();
        if let Some(u) = &self.user {
            s.push_str(u);
            s.push('@');
        }
        match (self.ipv6, self.port) {
            (true, Some(p)) => s.push_str(&format!("[{}]:{p}", self.host)),
            (true, None) => s.push_str(&format!("[{}]", self.host)),
            (false, Some(p)) => s.push_str(&format!("{}:{p}", self.host)),
            (false, None) => s.push_str(&self.host),
        }
        s
    }
}

/// Parse one hop token (`[user@]host[:port]`, optionally `ssh://`-prefixed).
/// Shares the ad-hoc connect parser, so a token ssh could read as an option
/// (leading `-`), or with whitespace or control characters, is refused.
pub fn parse_raw_hop(token: &str) -> Option<RawHop> {
    let hop = parse_raw_hop_unchecked(token)?;
    // OpenSSH runs the `-J` hop helper (`exec ssh -l <user> ... -W host:port`)
    // through a shell without quoting the user, and itself accepts `$`, `\`,
    // `=` and `/` in one: `a$IFS-oProxyCommand=touch\$IFS/x@h` becomes an
    // injected option. A blocklist cannot keep up with that, so only what a real
    // login or host name is made of gets through. Non-ASCII letters stay in a
    // login (a Windows `Администратор`): no shell gives them a meaning.
    let user_ok = |u: &str| {
        !u.starts_with('-')
            && u.chars()
                .all(|c| c.is_alphanumeric() || ".+-_%@".contains(c))
    };
    let host_ok = |h: &str, ipv6: bool| {
        h.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_%".contains(c) || (ipv6 && c == ':'))
    };
    if !hop.user.as_deref().is_none_or(user_ok) || !host_ok(&hop.host, hop.ipv6) {
        return None;
    }
    Some(hop)
}

fn parse_raw_hop_unchecked(token: &str) -> Option<RawHop> {
    let token = token.trim();
    let token = token.strip_prefix("ssh://").unwrap_or(token);
    // ssh splits the user on the LAST '@', so `john@corp.com@10.0.0.1` is user
    // `john@corp.com` on host `10.0.0.1`; the ad-hoc parser takes the first.
    if let Some((user, host)) = token.rsplit_once('@').filter(|(u, _)| u.contains('@')) {
        if user.starts_with('-') || user.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return None;
        }
        let t = crate::app::adhoc::parse_adhoc_target(host)?;
        return Some(RawHop {
            user: Some(user.to_string()),
            host: t.host,
            port: t.port,
            ipv6: t.ipv6,
        });
    }
    let t = crate::app::adhoc::parse_adhoc_target(token)?;
    // `-J` has no bare IPv6 literal (ssh reads the last colon as a port): it
    // must be bracketed, which the ad-hoc connect parser does not insist on.
    if !t.ipv6 && t.host.contains(':') {
        return None;
    }
    Some(RawHop {
        user: t.user,
        host: t.host,
        port: t.port,
        ipv6: t.ipv6,
    })
}

/// `ProxyJump none` / `-J none`: no jump at all.
pub fn is_jump_none(jump: &str) -> bool {
    jump.trim().eq_ignore_ascii_case("none")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HopKind {
    /// Index into [`Map::hosts`].
    Saved(usize),
    Raw(RawHop),
    /// The opaque ProxyCommand of host `n`.
    ProxyCommand(usize),
    /// A token ssh could not safely take as a host.
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub kind: HopKind,
    pub source: JumpSource,
    /// The hop as written in the jump list (`user@host:port` included), what
    /// `ssh -J` is handed. A ProxyCommand hop carries its fixed label.
    pub token: String,
}

impl Hop {
    pub fn key(&self) -> HopKey {
        match &self.kind {
            HopKind::Saved(s) => HopKey::Saved(*s),
            HopKind::Raw(r) => HopKey::Raw(r.clone()),
            HopKind::ProxyCommand(h) => HopKey::ProxyCmd(*h),
            HopKind::Invalid(t) => HopKey::Invalid(t.clone()),
        }
    }
}

/// How a host is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chain {
    /// Hops in order; empty = direct.
    Hops(Vec<Hop>),
    /// A jump loop, in order, the first host repeated at the end.
    Cycle(Vec<usize>),
}

/// Identity of a hop node, for the trie and blast radius.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HopKey {
    Saved(usize),
    Raw(RawHop),
    /// The opaque ProxyCommand of host `n`.
    ProxyCmd(usize),
    Invalid(String),
}

/// Fixed label of a ProxyCommand hop: the command text is never shown.
pub const PROXY_CMD_LABEL: &str = "proxy cmd";

/// A jump token as a saved host: the whole token is a host name, or a
/// `[user@]name[:port]` form of one (ssh then looks the name up in config).
fn lookup_saved(index: &HashMap<&str, usize>, token: &str) -> Option<usize> {
    if let Some(&h) = index.get(token) {
        return Some(h);
    }
    let raw = parse_raw_hop(token)?;
    if raw.ipv6 {
        return None;
    }
    index.get(raw.host.as_str()).copied()
}

fn expand(
    hosts: &[MapHost],
    index: &HashMap<&str, usize>,
    idx: usize,
    visiting: &mut Vec<usize>,
) -> Result<Vec<Hop>, Vec<usize>> {
    let host = &hosts[idx];
    if host.proxy_command {
        return Ok(vec![Hop {
            kind: HopKind::ProxyCommand(idx),
            source: JumpSource::SshConfig,
            token: PROXY_CMD_LABEL.to_string(),
        }]);
    }
    let Some(jump) = host.jump.as_deref().filter(|j| !is_jump_none(j)) else {
        return Ok(Vec::new());
    };
    let source = host.jump_source.clone().unwrap_or(JumpSource::Own);
    let mut out = Vec::new();
    for (i, token) in jump
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .enumerate()
    {
        let Some(h) = lookup_saved(index, token) else {
            let kind = match parse_raw_hop(token) {
                Some(raw) => HopKind::Raw(raw),
                None => HopKind::Invalid(token.to_string()),
            };
            out.push(Hop {
                kind,
                source: source.clone(),
                token: token.to_string(),
            });
            continue;
        };
        if let Some(pos) = visiting.iter().position(|&v| v == h) {
            let mut cycle = visiting[pos..].to_vec();
            cycle.push(h);
            return Err(cycle);
        }
        // Only ssh itself follows a hop's own chain, and it only knows
        // ssh_config: a launcher hop is expanded to `user@addr`, so its own
        // jump setting is never applied.
        if i == 0 && hosts[h].ssh_config {
            visiting.push(h);
            let own = expand(hosts, index, h, visiting);
            visiting.pop();
            out.extend(own?);
        }
        out.push(Hop {
            kind: HopKind::Saved(h),
            source: source.clone(),
            token: token.to_string(),
        });
    }
    Ok(out)
}

/// Resolve the chain of `hosts[idx]`. `index` maps a host name to its index.
/// The visited set is per resolution path (so a hop shared by two branches is
/// not a loop) and seeded with the target.
fn resolve_chain(hosts: &[MapHost], index: &HashMap<&str, usize>, idx: usize) -> Chain {
    match expand(hosts, index, idx, &mut vec![idx]) {
        Ok(hops) => Chain::Hops(hops),
        Err(cycle) => Chain::Cycle(cycle),
    }
}

/// One trie node: a distinct chain prefix.
#[derive(Debug, Clone)]
pub struct Node {
    pub key: HopKey,
    pub parent: Option<usize>,
    pub depth: usize,
    pub children: Vec<usize>,
    /// Hosts whose chain ends here, grouped by host group, names sorted.
    pub cargo: Vec<(String, Vec<usize>)>,
    /// Hosts behind this node at any depth.
    pub sigma: usize,
}

/// One line of the map, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A group of root hops, by index in [`Map::clusters`] (only on big maps).
    Cluster(usize),
    /// The folded tail of an open cluster.
    More(usize),
    /// A hop node (depth-first: a parent, then its children).
    Node(usize),
    /// A jump loop, by index in [`Map::cycles`].
    Cycle(usize),
    /// The hosts with no jump.
    Direct,
    /// One group of the expanded direct-only bucket, by index in
    /// [`Map::direct_groups`].
    DirectGroup(usize),
}

/// More root hops than this are grouped into clusters.
pub const CLUSTER_ROOTS: usize = 24;

/// A group of root hops on a big map: the hop's own group, or for ungrouped
/// ssh_config hops the domain suffix, then the /24.
#[derive(Debug, Clone)]
pub struct Cluster {
    pub name: String,
    /// Root trie nodes, biggest subtree first.
    pub roots: Vec<usize>,
    /// Hosts behind all of them.
    pub sigma: usize,
}

/// `10.20.4.0/24` for an IPv4 address, the domain after the first label for a
/// name (`db.corp.example` -> `*.corp.example`), the whole text otherwise.
fn address_cluster(address: &str) -> String {
    if let Ok(ip) = address.parse::<std::net::Ipv4Addr>() {
        let o = ip.octets();
        return format!("{}.{}.{}.0/24", o[0], o[1], o[2]);
    }
    match address.split_once('.') {
        Some((_, rest)) if rest.contains('.') => format!("*.{rest}"),
        _ => address.to_string(),
    }
}

/// A jump loop and the hosts stuck behind it.
#[derive(Debug, Clone)]
pub struct CycleRow {
    pub path: Vec<usize>,
    pub hosts: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct Map {
    pub hosts: Vec<MapHost>,
    pub chains: Vec<Chain>,
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    /// Hosts with no jump that nothing jumps through.
    pub direct: Vec<usize>,
    pub cycles: Vec<CycleRow>,
    /// The direct-only hosts by group, biggest group first.
    pub direct_groups: Vec<(String, Vec<usize>)>,
    /// Root hops grouped, empty unless there are more than [`CLUSTER_ROOTS`].
    pub clusters: Vec<Cluster>,
    /// Final trie node of each host's chain (`None`: direct or cyclic).
    pub host_node: Vec<Option<usize>>,
    /// Hosts that serve as a hop somewhere on the map.
    pub hop_hosts: HashSet<usize>,
    /// Hops of the longest chain.
    pub max_depth: usize,
    /// Nodes that are only a step on the way (one child, no host of their own
    /// ends there): they get no row, the row of the node after them says
    /// `‹ +n hops ›` instead (design D08). A root inside a cluster keeps its row.
    pub folded: Vec<bool>,
}

impl Map {
    pub fn build(hosts: Vec<MapHost>) -> Map {
        let mut index: HashMap<&str, usize> = HashMap::new();
        for (i, h) in hosts.iter().enumerate() {
            index.entry(h.name.as_str()).or_insert(i);
        }
        let chains: Vec<Chain> = (0..hosts.len())
            .map(|i| resolve_chain(&hosts, &index, i))
            .collect();

        let mut order: Vec<usize> = (0..hosts.len()).collect();
        order.sort_by(|&a, &b| hosts[a].name.cmp(&hosts[b].name));

        let mut map = Map {
            host_node: vec![None; hosts.len()],
            ..Map::default()
        };
        let mut child_of: HashMap<(Option<usize>, HopKey), usize> = HashMap::new();
        let mut cargo: Vec<BTreeMap<String, Vec<usize>>> = Vec::new();
        let mut used: HashSet<usize> = HashSet::new();
        let mut loops: BTreeMap<Vec<usize>, CycleRow> = BTreeMap::new();

        for &i in &order {
            let keys: Vec<HopKey> = match &chains[i] {
                Chain::Hops(h) if h.is_empty() => continue,
                Chain::Hops(h) => h.iter().map(Hop::key).collect(),
                Chain::Cycle(path) => {
                    let mut key: Vec<usize> = path.clone();
                    key.sort_unstable();
                    key.dedup();
                    loops
                        .entry(key)
                        .or_insert_with(|| CycleRow {
                            path: path.clone(),
                            hosts: Vec::new(),
                        })
                        .hosts
                        .push(i);
                    continue;
                }
            };
            let mut parent = None;
            for key in keys {
                if let HopKey::Saved(s) = key {
                    used.insert(s);
                }
                let at = *child_of.entry((parent, key.clone())).or_insert_with(|| {
                    let n = map.nodes.len();
                    map.nodes.push(Node {
                        key,
                        parent,
                        depth: parent.map_or(0, |p| map.nodes[p].depth + 1),
                        children: Vec::new(),
                        cargo: Vec::new(),
                        sigma: 0,
                    });
                    cargo.push(BTreeMap::new());
                    match parent {
                        Some(p) => map.nodes[p].children.push(n),
                        None => map.roots.push(n),
                    }
                    n
                });
                parent = Some(at);
            }
            let last = parent.expect("non-empty chain");
            cargo[last]
                .entry(hosts[i].group.clone())
                .or_default()
                .push(i);
            map.host_node[i] = Some(last);
        }

        for (n, groups) in cargo.into_iter().enumerate() {
            map.nodes[n].cargo = groups.into_iter().collect();
            map.nodes[n].sigma = map.nodes[n].cargo.iter().map(|(_, h)| h.len()).sum();
        }
        // Parents are created before their children, so one reverse pass sums
        // every subtree.
        for n in (0..map.nodes.len()).rev() {
            if let Some(p) = map.nodes[n].parent {
                map.nodes[p].sigma += map.nodes[n].sigma;
            }
        }
        // Biggest subtree first, then by label, so the layout is stable.
        let label = |m: &Map, n: usize| match &m.nodes[n].key {
            HopKey::Saved(s) => hosts[*s].name.clone(),
            HopKey::Raw(r) => r.label(),
            HopKey::ProxyCmd(h) => format!("{PROXY_CMD_LABEL} {}", hosts[*h].name),
            HopKey::Invalid(t) => t.clone(),
        };
        let sorted = |list: Vec<usize>, m: &Map| {
            let mut list = list;
            list.sort_by(|&a, &b| {
                m.nodes[b]
                    .sigma
                    .cmp(&m.nodes[a].sigma)
                    .then_with(|| label(m, a).cmp(&label(m, b)))
            });
            list
        };
        let snapshot = map.clone();
        map.roots = sorted(std::mem::take(&mut map.roots), &snapshot);
        for n in 0..map.nodes.len() {
            map.nodes[n].children = sorted(std::mem::take(&mut map.nodes[n].children), &snapshot);
        }

        map.direct = order
            .iter()
            .copied()
            .filter(|i| matches!(&chains[*i], Chain::Hops(h) if h.is_empty()) && !used.contains(i))
            .collect();
        map.hop_hosts = used.clone();
        map.cycles = loops.into_values().collect();
        map.hosts = hosts;
        map.chains = chains;
        map.direct_groups = map.group_direct();
        if map.roots.len() > CLUSTER_ROOTS {
            map.clusters = map.cluster_roots();
        }
        map.max_depth = map.nodes.iter().map(|n| n.depth + 1).max().unwrap_or(0);
        let in_cluster: HashSet<usize> = map
            .clusters
            .iter()
            .flat_map(|c| c.roots.iter().copied())
            .collect();
        map.folded = map
            .nodes
            .iter()
            .enumerate()
            .map(|(n, node)| {
                node.children.len() == 1 && node.cargo.is_empty() && !in_cluster.contains(&n)
            })
            .collect();
        map
    }

    /// Group the root hops: `roots` is already biggest-first, so each cluster's
    /// members keep that order.
    fn cluster_roots(&self) -> Vec<Cluster> {
        let mut by_name: BTreeMap<String, Cluster> = BTreeMap::new();
        for &r in &self.roots {
            let name = match &self.nodes[r].key {
                HopKey::Saved(h) => {
                    let host = &self.hosts[*h];
                    if host.group == "_ungrouped" {
                        address_cluster(&host.address)
                    } else {
                        host.group.clone()
                    }
                }
                HopKey::Raw(raw) => address_cluster(&raw.host),
                HopKey::ProxyCmd(_) => PROXY_CMD_LABEL.to_string(),
                HopKey::Invalid(_) => "invalid hops".to_string(),
            };
            let c = by_name.entry(name.clone()).or_insert_with(|| Cluster {
                name,
                roots: Vec::new(),
                sigma: 0,
            });
            c.roots.push(r);
            c.sigma += self.nodes[r].sigma;
        }
        // A cluster of one hop is just that hop: it stays a plain row.
        let mut clusters: Vec<Cluster> = by_name
            .into_values()
            .filter(|c| c.roots.len() > 1)
            .collect();
        clusters.sort_by(|a, b| b.sigma.cmp(&a.sigma).then_with(|| a.name.cmp(&b.name)));
        clusters
    }

    fn group_direct(&self) -> Vec<(String, Vec<usize>)> {
        let mut by: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &h in &self.direct {
            by.entry(self.hosts[h].group.as_str()).or_default().push(h);
        }
        let mut groups: Vec<(String, Vec<usize>)> =
            by.into_iter().map(|(g, h)| (g.to_string(), h)).collect();
        groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        groups
    }

    /// Hosts drawn as glyphs on `row`, left to right. A cluster's strip is its
    /// hops themselves (hops are hosts too).
    pub fn row_hosts(&self, row: Row) -> Vec<usize> {
        match row {
            Row::Node(n) => self.nodes[n]
                .cargo
                .iter()
                .flat_map(|(_, h)| h.iter().copied())
                .collect(),
            Row::Cluster(c) => self.clusters[c]
                .roots
                .iter()
                .filter_map(|&r| match self.nodes[r].key {
                    HopKey::Saved(h) => Some(h),
                    _ => None,
                })
                .collect(),
            // The strip groups a loop's hosts by group name (sorted): walk
            // them in that drawn order.
            Row::Cycle(i) => {
                let mut hs = self.cycles[i].hosts.clone();
                hs.sort_by(|a, b| self.hosts[*a].group.cmp(&self.hosts[*b].group));
                hs
            }
            Row::DirectGroup(i) => self.direct_groups[i].1.clone(),
            Row::Direct | Row::More(_) => Vec::new(),
        }
    }

    /// No jump edge anywhere: every host is reached directly.
    pub fn is_edgeless(&self) -> bool {
        self.nodes.is_empty() && self.cycles.is_empty()
    }

    /// Whether `h` is reached with no jump and is no hop itself, so a ping from
    /// here says something about it.
    pub fn reached_directly(&self, h: usize) -> bool {
        matches!(&self.chains[h], Chain::Hops(c) if c.is_empty()) && !self.hop_hosts.contains(&h)
    }

    /// `ssh -J a,b target`, hops as the map shows them (names, `user@host:port`
    /// for raw hops; a ProxyCommand hop is not part of the string).
    pub fn ssh_command(&self, h: usize) -> String {
        match &self.chains[h] {
            Chain::Hops(hops) => {
                let jump: Vec<&str> = hops
                    .iter()
                    .filter(|hop| !matches!(hop.kind, HopKind::ProxyCommand(_)))
                    .map(|hop| hop.token.as_str())
                    .collect();
                if jump.is_empty() {
                    format!("ssh {}", self.hosts[h].name)
                } else {
                    format!("ssh -J {} {}", jump.join(","), self.hosts[h].name)
                }
            }
            Chain::Cycle(_) => "jump loop: not connectable".into(),
        }
    }

    /// The command for the hop at trie node `node`: `ssh` to it through the
    /// hops above it.
    pub fn node_command(&self, node: usize) -> String {
        let path = self.node_path(node);
        let label = |n: usize| match &self.nodes[n].key {
            HopKey::Saved(h) => Some(self.hosts[*h].name.clone()),
            HopKey::Raw(r) => Some(r.label()),
            HopKey::ProxyCmd(_) | HopKey::Invalid(_) => None,
        };
        let Some(target) = label(node) else {
            return String::new();
        };
        let above: Vec<String> = path[..path.len() - 1]
            .iter()
            .filter_map(|&n| label(n))
            .collect();
        if above.is_empty() {
            format!("ssh {target}")
        } else {
            format!("ssh -J {} {target}", above.join(","))
        }
    }

    /// Trie nodes from the root down to `node`.
    pub fn node_path(&self, node: usize) -> Vec<usize> {
        let mut path = vec![node];
        while let Some(p) = self.nodes[*path.last().expect("non-empty")].parent {
            path.push(p);
        }
        path.reverse();
        path
    }

    /// Display name of a hop key.
    pub fn key_label(&self, key: &HopKey) -> String {
        match key {
            HopKey::Saved(s) => self.hosts[*s].name.clone(),
            HopKey::Raw(r) => r.label(),
            HopKey::ProxyCmd(_) => PROXY_CMD_LABEL.to_string(),
            HopKey::Invalid(t) => t.clone(),
        }
    }

    /// Hop keys `host` depends on, in order. A cyclic host depends on every
    /// host of its loop.
    pub fn chain_keys(&self, host: usize) -> Vec<HopKey> {
        match &self.chains[host] {
            Chain::Hops(h) => h.iter().map(Hop::key).collect(),
            Chain::Cycle(path) => path[..path.len() - 1]
                .iter()
                .map(|&s| HopKey::Saved(s))
                .collect(),
        }
    }

    /// Blast radius of a hop: every host whose resolved chain contains it at
    /// any depth, counted across all occurrences; hosts stuck in a loop count
    /// as dependents of the loop's hops. The hop's own host is not in its own
    /// radius.
    pub fn blast_radius(&self, key: &HopKey) -> Vec<usize> {
        let own = match key {
            HopKey::Saved(s) => Some(*s),
            _ => None,
        };
        (0..self.hosts.len())
            .filter(|&i| Some(i) != own)
            .filter(|&i| self.chain_keys(i).contains(key))
            .collect()
    }
}

#[cfg(test)]
mod tests;
