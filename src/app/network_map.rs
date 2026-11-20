//! Network map tab (issue #79): cursor, navigation and actions over
//! [`crate::map::Map`]. The model is pure; this is the stateful side.

use super::*;
use std::collections::{HashMap, HashSet};

use crate::map::{HopKey, Map, Row};

/// What the map cursor is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapSel {
    /// A hop node (any ancestor on the row's path).
    Hop(usize),
    /// One host glyph.
    Host(usize),
    /// A jump loop, by index in [`Map::cycles`].
    Cycle(usize),
    /// The direct-only bucket.
    Direct,
    /// A cluster of root hops, by index in [`Map::clusters`].
    Cluster(usize),
    /// The folded tail of an open cluster.
    More(usize),
}

/// Root hops of an open cluster shown before the tail folds.
pub const CLUSTER_SHOWN: usize = 12;
/// Extra hops one reveal step shows.
pub const REVEAL_STEP: usize = 20;

/// Run state of a tunnel, for the tunnel lens colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelState {
    Running,
    Reconnecting,
    Stopped,
}

/// A tunnel as the map shows it.
#[derive(Debug, Clone)]
pub struct HostTunnel {
    pub id: i64,
    pub kind: crate::store::TunnelType,
    pub state: TunnelState,
    pub summary: String,
}

/// Blast-radius lens: one hop and every host that depends on it.
pub struct BlastLens {
    pub key: HopKey,
    pub hosts: HashSet<usize>,
}

/// Which lens re-skins the map.
pub enum Lens {
    None,
    Blast(BlastLens),
    Tunnel,
}

/// How one row is drawn under the active lens and filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    Normal,
    /// Out of the lens, or only kept as context for a filter match.
    Dim,
    /// Inside a blast radius.
    Warn,
}

/// What the host picker is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsePurpose {
    /// `u` with a hop chosen: pick the host that takes it as its jump.
    Apply,
    /// `e` with no host under the cursor: pick the host to edit.
    Edit,
    /// `u` with no hop chosen: pick the host to use as the jump, then pick
    /// who takes it.
    PickHop,
}

/// `u`: pick the host that should use a hop as its jump.
#[derive(Clone)]
pub struct UsePicker {
    pub purpose: UsePurpose,
    /// The jump token to apply (host name or `user@host:port`); empty unless
    /// the purpose is [`UsePurpose::Apply`].
    pub token: String,
    pub query: String,
    pub selected: usize,
    /// Add after the host's own hops instead of replacing them.
    pub append: bool,
    /// (host index, would loop), in name order.
    pub hosts: Vec<(usize, bool)>,
}

impl UsePicker {
    /// Hosts that survive the query.
    pub fn visible(&self, map: &Map) -> Vec<(usize, bool)> {
        let q = self.query.to_lowercase();
        self.hosts
            .iter()
            .copied()
            .filter(|(h, _)| q.is_empty() || map.hosts[*h].name.to_lowercase().contains(&q))
            .collect()
    }
}

/// `#` tag picker: feeds `tag:` terms into the filter. Typing narrows the
/// list (fuzzy, best first).
#[derive(Clone)]
pub struct TagPicker {
    pub items: Vec<String>,
    pub query: String,
    /// Index into [`TagPicker::visible`].
    pub selected: usize,
}

impl TagPicker {
    pub fn new(items: Vec<String>) -> Self {
        Self {
            items,
            query: String::new(),
            selected: 0,
        }
    }

    /// `items` indices that survive the query, best match first.
    pub fn visible(&self) -> Vec<usize> {
        let items: Vec<Vec<String>> = self.items.iter().map(|t| vec![t.clone()]).collect();
        crate::search::fuzzy_indices(&self.query, &items)
    }
}

/// The map model plus the cursor, lens and filter. `pos` walks along the
/// selected row's path: first each hop from the root down to the row's own
/// node, then its host glyphs, left to right.
pub struct MapView {
    pub map: Map,
    /// Every row of the map; `rows` is the part the filter keeps.
    all_rows: Vec<Row>,
    pub rows: Vec<Row>,
    /// Rows (indexes into `rows`) kept only as context above a filter match.
    pub context: HashSet<usize>,
    /// Hosts the filter matches; `None` while no filter is set.
    pub matched: Option<HashSet<usize>>,
    pub row: usize,
    pub pos: usize,
    pub lens: Lens,
    pub query: String,
    /// The filter line owns the keyboard.
    pub editing: bool,
    pub picker: Option<TagPicker>,
    /// The map panel takes the whole body (no ribbon, legend or facts).
    pub zoomed: bool,
    /// Set by the renderer: the map is drawn as an outline, which has no room
    /// for the pickers, so they do not open.
    pub outline: std::cell::Cell<bool>,
    pub use_picker: Option<UsePicker>,
    /// Clusters (by name) whose hops are listed.
    pub open_clusters: HashSet<String>,
    /// Extra hops revealed per open cluster, in [`REVEAL_STEP`] units.
    pub revealed: HashMap<String, usize>,
    /// The direct-only bucket is expanded into one strip per group.
    pub direct_open: bool,
    /// Host index by managed id, built with the map (for the tunnel lens).
    pub host_by_id: HashMap<i64, usize>,
    /// Per trie node: some host behind it (or on it) matches the filter.
    match_node: Vec<bool>,
}

impl MapView {
    pub fn new(map: Map) -> Self {
        let mut open_clusters = HashSet::new();
        // The biggest cluster starts open so the tree is visible.
        if let Some(c) = map.clusters.first() {
            open_clusters.insert(c.name.clone());
        }
        let mut view = Self {
            map,
            all_rows: Vec::new(),
            rows: Vec::new(),
            context: HashSet::new(),
            matched: None,
            row: 0,
            pos: 0,
            lens: Lens::None,
            query: String::new(),
            editing: false,
            picker: None,
            zoomed: false,
            outline: std::cell::Cell::new(false),
            use_picker: None,
            open_clusters,
            revealed: HashMap::new(),
            direct_open: false,
            match_node: Vec::new(),
            host_by_id: HashMap::new(),
        };
        view.all_rows = view.build_rows();
        view.rows = view.all_rows.clone();
        view
    }

    /// Nothing jumps anywhere: the map has no edge to draw (design D07).
    pub fn edgeless(&self) -> bool {
        self.map.is_edgeless()
    }

    pub fn clustered(&self) -> bool {
        !self.map.clusters.is_empty()
    }

    /// Roots of cluster `c` that are listed: the top of the Σ order, every
    /// root a filter matches, and whatever reveal steps added.
    fn shown_roots(&self, c: usize) -> Vec<usize> {
        let cl = &self.map.clusters[c];
        let extra = self.revealed.get(&cl.name).copied().unwrap_or(0);
        let top = CLUSTER_SHOWN + extra * REVEAL_STEP;
        cl.roots
            .iter()
            .enumerate()
            .filter(|(i, &r)| *i < top || self.match_node.get(r).copied().unwrap_or(false))
            .map(|(_, &r)| r)
            .collect()
    }

    /// Every row of the map under the current fold state.
    fn build_rows(&self) -> Vec<Row> {
        fn walk(map: &Map, n: usize, out: &mut Vec<Row>) {
            if !map.folded[n] {
                out.push(Row::Node(n));
            }
            for &c in &map.nodes[n].children {
                walk(map, c, out);
            }
        }
        let map = &self.map;
        let mut rows = Vec::new();
        if map.clusters.is_empty() {
            for &r in &map.roots {
                walk(map, r, &mut rows);
            }
        } else {
            for (ci, c) in map.clusters.iter().enumerate() {
                rows.push(Row::Cluster(ci));
                if self.open_clusters.contains(&c.name) {
                    let shown = self.shown_roots(ci);
                    for &r in &shown {
                        walk(map, r, &mut rows);
                    }
                    if shown.len() < c.roots.len() {
                        rows.push(Row::More(ci));
                    }
                }
            }
            // Hops that belong to no cluster stay plain rows after them.
            let clustered: HashSet<usize> = map
                .clusters
                .iter()
                .flat_map(|c| c.roots.iter().copied())
                .collect();
            for &r in &map.roots {
                if !clustered.contains(&r) {
                    walk(map, r, &mut rows);
                }
            }
        }
        rows.extend((0..map.cycles.len()).map(Row::Cycle));
        if !map.direct.is_empty() {
            rows.push(Row::Direct);
            if self.direct_open {
                rows.extend((0..map.direct_groups.len()).map(Row::DirectGroup));
            }
        }
        rows
    }

    /// Hops on the path to `row`'s node (1 for a loop or the direct bucket).
    pub fn hop_count(&self, row: usize) -> usize {
        match self.rows.get(row) {
            Some(Row::Node(n)) => self.map.nodes[*n].depth + 1,
            _ => 1,
        }
    }

    fn slots(&self, row: usize) -> usize {
        let Some(&r) = self.rows.get(row) else {
            return 0;
        };
        self.hop_count(row) + self.map.row_hosts(r).len()
    }

    pub fn selection(&self) -> Option<MapSel> {
        let r = *self.rows.get(self.row)?;
        let hops = self.hop_count(self.row);
        Some(match r {
            Row::Node(n) => {
                if self.pos < hops {
                    MapSel::Hop(self.map.node_path(n)[self.pos])
                } else {
                    MapSel::Host(*self.map.row_hosts(r).get(self.pos - hops)?)
                }
            }
            Row::Cycle(i) => {
                if self.pos < hops {
                    MapSel::Cycle(i)
                } else {
                    MapSel::Host(*self.map.row_hosts(r).get(self.pos - hops)?)
                }
            }
            Row::Cluster(i) => {
                if self.pos < hops {
                    MapSel::Cluster(i)
                } else {
                    MapSel::Host(*self.map.row_hosts(r).get(self.pos - hops)?)
                }
            }
            Row::More(i) => MapSel::More(i),
            Row::Direct => MapSel::Direct,
            Row::DirectGroup(_) => {
                if self.pos < hops {
                    MapSel::Direct
                } else {
                    MapSel::Host(*self.map.row_hosts(r).get(self.pos - hops)?)
                }
            }
        })
    }

    /// Step to the previous / next row. From a hop the cursor lands on the new
    /// row's own (last) hop, the one drawn on it: keeping the depth would leave
    /// it on an ancestor that this row does not draw. From a glyph it lands on
    /// the same glyph index (clamped), or on the row's own hop when that row
    /// has no glyphs.
    pub fn move_row(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let next = (self.row as isize + delta).clamp(0, self.rows.len() as isize - 1) as usize;
        if next == self.row {
            return;
        }
        let old_hops = self.hop_count(self.row);
        let new_hops = self.hop_count(next);
        let new_glyphs = self.slots(next) - new_hops;
        self.pos = if self.pos < old_hops || new_glyphs == 0 {
            new_hops - 1
        } else {
            new_hops + (self.pos - old_hops).min(new_glyphs - 1)
        };
        self.row = next;
    }

    /// Walk along the row: `-1` toward the root, `+1` toward the glyphs.
    pub fn move_col(&mut self, delta: isize) {
        let max = self.slots(self.row).saturating_sub(1) as isize;
        self.pos = (self.pos as isize + delta).clamp(0, max.max(0)) as usize;
    }

    fn clamp(&mut self) {
        self.row = self.row.min(self.rows.len().saturating_sub(1));
        self.pos = self.pos.min(self.slots(self.row).saturating_sub(1));
    }

    /// Row and `pos` of host `h`'s glyph, if its row is currently shown.
    pub fn locate(&self, h: usize) -> Option<(usize, usize)> {
        for (i, &r) in self.rows.iter().enumerate() {
            if let Some(k) = self.map.row_hosts(r).iter().position(|&x| x == h) {
                return Some((i, self.hop_count(i) + k));
            }
        }
        None
    }

    /// Put the cursor on host `h`'s glyph. `false` when it is not shown.
    pub fn jump_to_host(&mut self, h: usize) -> bool {
        match self.locate(h) {
            Some((row, pos)) => {
                self.row = row;
                self.pos = pos;
                true
            }
            None => false,
        }
    }

    /// Re-derive the visible rows from `query` and the fold state, keeping the
    /// cursor on what it stood on when that is still shown.
    pub fn apply_filter(&mut self) {
        let before = self.selection();
        let filter = crate::map::MapFilter::parse(&self.query);
        self.context.clear();
        if filter.is_empty() {
            self.matched = None;
            self.match_node.clear();
            self.all_rows = self.build_rows();
            self.rows = self.all_rows.clone();
        } else {
            let matched: HashSet<usize> = filter.matching(&self.map).into_iter().collect();
            // A node matches when a host on it or anywhere behind it does, or
            // when the node is itself a matching host. Parents come before
            // their children, so one reverse pass lifts matches up the tree.
            let map = &self.map;
            let mut match_node: Vec<bool> = map
                .nodes
                .iter()
                .map(|n| {
                    matches!(n.key, HopKey::Saved(h) if matched.contains(&h))
                        || n.cargo
                            .iter()
                            .any(|(_, hs)| hs.iter().any(|h| matched.contains(h)))
                })
                .collect();
            for n in (0..map.nodes.len()).rev() {
                if match_node[n] {
                    if let Some(p) = map.nodes[n].parent {
                        match_node[p] = true;
                    }
                }
            }
            self.match_node = match_node;
            self.matched = Some(matched);
            self.all_rows = self.build_rows();
            let map = &self.map;
            let matched = self.matched.as_ref().expect("just set");
            let strong: Vec<bool> = self
                .all_rows
                .iter()
                .map(|&r| match r {
                    Row::Node(n) => {
                        // A folded hop draws no row of its own: its match
                        // shows on the first row below it.
                        let mut folded_match = false;
                        let mut p = map.nodes[n].parent;
                        while let Some(f) = p.filter(|&f| map.folded[f]) {
                            if matches!(map.nodes[f].key, HopKey::Saved(h) if matched.contains(&h))
                            {
                                folded_match = true;
                                break;
                            }
                            p = map.nodes[f].parent;
                        }
                        folded_match
                            || matches!(map.nodes[n].key, HopKey::Saved(h) if matched.contains(&h))
                            || map.row_hosts(r).iter().any(|h| matched.contains(h))
                    }
                    Row::Cluster(c) => map.clusters[c].roots.iter().any(|&r| self.match_node[r]),
                    Row::More(_) => false,
                    Row::Cycle(i) => {
                        map.cycles[i].path.iter().any(|h| matched.contains(h))
                            || map.row_hosts(r).iter().any(|h| matched.contains(h))
                    }
                    Row::Direct => map.direct.iter().any(|h| matched.contains(h)),
                    Row::DirectGroup(_) => map.row_hosts(r).iter().any(|h| matched.contains(h)),
                })
                .collect();
            let depth = |r: Row| match r {
                Row::Node(n) => map.nodes[n].depth,
                _ => 0,
            };
            let mut rows = Vec::new();
            let mut context = HashSet::new();
            for (i, &r) in self.all_rows.iter().enumerate() {
                let d = depth(r);
                let below = self.all_rows[i + 1..]
                    .iter()
                    .take_while(|&&x| matches!(x, Row::Node(_)) && depth(x) > d)
                    .count();
                let has_match_below = (i + 1..=i + below).any(|j| strong[j]);
                if strong[i] || has_match_below {
                    if !strong[i] {
                        context.insert(rows.len());
                    }
                    rows.push(r);
                }
            }
            self.rows = rows;
            self.context = context;
        }
        self.row = self.row.min(self.rows.len().saturating_sub(1));
        match before {
            Some(MapSel::Host(h)) if self.jump_to_host(h) => {}
            Some(MapSel::Hop(n)) => {
                if let Some(i) = self.rows.iter().position(|&r| r == Row::Node(n)) {
                    self.row = i;
                    self.pos = self.hop_count(i) - 1;
                }
            }
            _ => {}
        }
        self.clamp();
    }

    /// After a filter is kept (Enter): open only the clusters that hold a
    /// match and fold the rest.
    pub fn reveal_matches(&mut self) {
        if !self.clustered() || self.matched.is_none() {
            return;
        }
        self.open_clusters = self
            .map
            .clusters
            .iter()
            .filter(|c| c.roots.iter().any(|&r| self.match_node[r]))
            .map(|c| c.name.clone())
            .collect();
        self.apply_filter();
    }

    /// Space: fold or unfold what the cursor is on.
    pub fn toggle_fold(&mut self) {
        match self.selection() {
            Some(MapSel::Cluster(c)) => {
                let name = self.map.clusters[c].name.clone();
                if !self.open_clusters.remove(&name) {
                    self.open_clusters.insert(name);
                }
            }
            Some(MapSel::More(c)) => {
                *self
                    .revealed
                    .entry(self.map.clusters[c].name.clone())
                    .or_default() += 1;
            }
            Some(MapSel::Direct) => self.direct_open = !self.direct_open,
            _ => return,
        }
        self.apply_filter();
    }

    /// Shift+Z: fold every cluster, or when none is open, expand the
    /// direct-only bucket (and fold it again on the next press).
    pub fn fold_all(&mut self) {
        if !self.open_clusters.is_empty() {
            self.open_clusters.clear();
            self.revealed.clear();
        } else {
            self.direct_open = !self.direct_open;
        }
        self.apply_filter();
    }

    /// Whether `row` (index into `rows`) lies inside the active blast radius.
    fn row_in_radius(&self, blast: &BlastLens, row: usize) -> bool {
        match self.rows[row] {
            Row::Node(n) => self
                .map
                .node_path(n)
                .iter()
                .any(|&p| self.map.nodes[p].key == blast.key),
            Row::Cycle(i) => self.map.cycles[i]
                .hosts
                .iter()
                .any(|h| blast.hosts.contains(h)),
            Row::Cluster(c) => self.map.clusters[c]
                .roots
                .iter()
                .any(|&r| self.node_in_radius(blast, r)),
            Row::More(_) | Row::Direct | Row::DirectGroup(_) => false,
        }
    }

    /// Whether `node` or anything below it is on the blast radius' path.
    fn node_in_radius(&self, blast: &BlastLens, node: usize) -> bool {
        self.map.nodes[node].key == blast.key
            || self.map.nodes[node]
                .children
                .iter()
                .any(|&c| self.node_in_radius(blast, c))
    }

    /// How `row` is drawn under the active lens and filter.
    pub fn tint(&self, row: usize) -> Tint {
        if let Lens::Blast(b) = &self.lens {
            return if self.row_in_radius(b, row) {
                Tint::Warn
            } else {
                Tint::Dim
            };
        }
        if self.context.contains(&row) {
            Tint::Dim
        } else {
            Tint::Normal
        }
    }

    /// Dependents of the blast lens in reading order.
    fn blast_order(&self, blast: &BlastLens) -> Vec<usize> {
        let mut at: Vec<(usize, usize, usize)> = blast
            .hosts
            .iter()
            .filter_map(|&h| self.locate(h).map(|(r, p)| (r, p, h)))
            .collect();
        at.sort_unstable();
        at.into_iter().map(|(_, _, h)| h).collect()
    }

    /// `n` / `N`: move the cursor to the next / previous dependent.
    pub fn blast_step(&mut self, delta: isize) {
        let Lens::Blast(b) = &self.lens else {
            return;
        };
        let order = self.blast_order(b);
        if order.is_empty() {
            return;
        }
        let here = match self.selection() {
            Some(MapSel::Host(h)) => order.iter().position(|&x| x == h),
            _ => None,
        };
        let next = match (here, delta > 0) {
            (Some(i), true) => (i + 1) % order.len(),
            (Some(i), false) => (i + order.len() - 1) % order.len(),
            (None, true) => 0,
            (None, false) => order.len() - 1,
        };
        self.jump_to_host(order[next]);
    }

    /// The hop the blast lens would start from at the cursor.
    pub fn blast_key_at_cursor(&self) -> Option<HopKey> {
        match self.selection()? {
            MapSel::Hop(n) => Some(self.map.nodes[n].key.clone()),
            MapSel::Host(h) => Some(HopKey::Saved(h)),
            _ => None,
        }
    }

    /// Tab in the filter line: complete the last term. `group:`, `tag:` and
    /// `via:` take a value from what the map knows; a bare word completes to a
    /// term prefix. Repeating Tab cycles through the candidates.
    pub fn complete_filter(&mut self) {
        let at = crate::map::filter::last_term_start(&self.query);
        let (head, last) = (self.query[..at].to_string(), self.query[at..].to_string());
        let prefix = ["group:", "tag:", "via:"]
            .into_iter()
            .find(|p| last.starts_with(p))
            .unwrap_or("");
        let val = last[prefix.len()..].trim_start_matches('"');
        let mut cands: Vec<String> = match prefix {
            "tag:" => self.all_tags(),
            "group:" => self.map.hosts.iter().map(|h| h.group.clone()).collect(),
            "via:" => self
                .map
                .nodes
                .iter()
                .map(|n| self.map.key_label(&n.key))
                .collect(),
            _ => ["group:", "tag:", "via:"].map(String::from).to_vec(),
        };
        cands.sort();
        cands.dedup();
        let lower = val.to_lowercase();
        let hits: Vec<&String> = cands
            .iter()
            .filter(|c| c.to_lowercase().starts_with(&lower))
            .collect();
        if hits.is_empty() {
            return;
        }
        let next = hits
            .iter()
            .position(|c| c.to_lowercase() == lower)
            .map_or(0, |i| (i + 1) % hits.len());
        self.query = match prefix {
            "" => format!("{head}{}", hits[next]),
            p => format!(
                "{head}{}",
                crate::map::filter::quote_term(p.trim_end_matches(':'), hits[next])
            ),
        };
    }

    pub fn filter_active(&self) -> bool {
        self.matched.is_some()
    }

    /// Sorted, deduplicated tags of every host.
    pub fn all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .map
            .hosts
            .iter()
            .flat_map(|h| h.tags.iter().cloned())
            .collect();
        tags.sort();
        tags.dedup();
        tags
    }
}

impl App {
    pub(crate) fn switch_to_map_tab(&mut self) {
        self.active_tab = 5;
        // The tunnel lens and the facts read the tunnel list.
        if let Err(e) = self.reload_tunnels() {
            self.host_notice = Some(format!("tunnels: {e:#}"));
        }
        if self.map_view.is_none() {
            self.rebuild_map();
        }
    }

    /// Rebuild the map from the current host list, keeping what the user has
    /// open: cursor, filter, lens, folds and pickers. Host indexes belong to
    /// one build of the map, so everything that holds one is carried over by
    /// host name.
    pub(crate) fn rebuild_map(&mut self) {
        let hosts = match crate::map::collect_hosts(&self.store, &self.hosts) {
            Ok(h) => h,
            Err(e) => {
                self.map_view = None;
                let msg = format!("map: {e:#}");
                self.host_notice = Some(msg.clone());
                self.map_error = Some(msg);
                return;
            }
        };
        self.map_error = None;
        let mut view = MapView::new(Map::build(hosts));
        view.host_by_id = self
            .hosts
            .iter()
            .enumerate()
            .filter_map(|(i, h)| Some((h.managed_id()?, i)))
            .collect();
        let old = self.map_view.take();
        // A test owner names a hop or host of the map it ran on.
        if old.as_ref().is_none_or(|o| o.map.hosts != view.map.hosts)
            && self
                .test_run
                .as_ref()
                .is_some_and(|r| matches!(r.owner, crate::app::test_connect::TestOwner::Map(_)))
        {
            self.test_run = None;
        }
        if let Some(old) = old {
            let by_name: HashMap<String, usize> = view
                .map
                .hosts
                .iter()
                .enumerate()
                .map(|(i, h)| (h.name.clone(), i))
                .collect();
            let remap = |h: usize| {
                old.map
                    .hosts
                    .get(h)
                    .and_then(|x| by_name.get(&x.name).copied())
            };
            let before = old.selection();
            view.row = old.row;
            view.pos = old.pos;
            view.zoomed = old.zoomed;
            view.open_clusters = old.open_clusters.clone();
            view.revealed = old.revealed.clone();
            view.direct_open = old.direct_open;
            view.query = old.query.clone();
            view.editing = old.editing;
            view.apply_filter();
            if let Some(h) = match before {
                Some(MapSel::Host(h)) => remap(h),
                _ => None,
            } {
                view.jump_to_host(h);
            }
            let tags = view.all_tags();
            view.picker = old.picker.clone().and_then(|mut p| {
                p.items.retain(|t| tags.contains(t));
                p.selected = p.selected.min(p.visible().len().saturating_sub(1));
                (!p.items.is_empty()).then_some(p)
            });
            view.use_picker = old.use_picker.as_ref().and_then(|p| {
                let mut p = p.clone();
                p.hosts = p
                    .hosts
                    .iter()
                    .filter_map(|&(h, loops)| Some((remap(h)?, loops)))
                    .collect();
                p.selected = p.selected.min(p.hosts.len().saturating_sub(1));
                (!p.hosts.is_empty()).then_some(p)
            });
            view.lens = match &old.lens {
                Lens::Blast(b) => {
                    let key = match &b.key {
                        HopKey::Saved(h) => remap(*h).map(HopKey::Saved),
                        HopKey::ProxyCmd(h) => remap(*h).map(HopKey::ProxyCmd),
                        other => Some(other.clone()),
                    };
                    match key {
                        Some(key) => {
                            let hosts: HashSet<usize> =
                                view.map.blast_radius(&key).into_iter().collect();
                            Lens::Blast(BlastLens { key, hosts })
                        }
                        None => Lens::None,
                    }
                }
                Lens::Tunnel => Lens::Tunnel,
                Lens::None => Lens::None,
            };
        }
        self.map_view = Some(view);
    }

    /// Tunnels by the index of the host they run on, for the map.
    pub fn map_tunnels(&self) -> HashMap<usize, Vec<HostTunnel>> {
        // The host-id index is built with the map, not every frame.
        let Some(by_id) = self.map_view.as_ref().map(|v| &v.host_by_id) else {
            return HashMap::new();
        };
        let mut out: HashMap<usize, Vec<HostTunnel>> = HashMap::new();
        for t in &self.tunnels {
            let Some(&idx) = t.host_id.and_then(|id| by_id.get(&id)) else {
                continue;
            };
            let state = if self.tunnel_manager.is_running(t.id) {
                TunnelState::Running
            } else if self.tunnel_manager.is_reconnecting(t.id) {
                TunnelState::Reconnecting
            } else {
                TunnelState::Stopped
            };
            out.entry(idx).or_default().push(HostTunnel {
                id: t.id,
                kind: t.tunnel_type,
                state,
                summary: match t.tunnel_type {
                    crate::store::TunnelType::Dynamic => format!(":{} socks", t.local_port),
                    _ => format!(
                        "{}\u{2192}{}:{}",
                        t.local_port, t.remote_host, t.remote_port
                    ),
                },
            });
        }
        out
    }

    pub(crate) fn handle_key_map(&mut self, key: KeyEvent) -> Result<()> {
        // A failed rebuild stays on screen until one succeeds.
        self.host_notice = self.map_error.clone();
        if self.map_view.is_none() && self.map_error.is_some() {
            self.rebuild_map();
        }

        if self
            .map_view
            .as_ref()
            .is_some_and(|v| v.use_picker.is_some())
        {
            return self.handle_key_map_use(key);
        }
        // The tag picker and the filter line own the keyboard while open.
        if self.map_view.as_ref().is_some_and(|v| v.picker.is_some()) {
            return self.handle_key_map_picker(key);
        }
        if self.map_view.as_ref().is_some_and(|v| v.editing) {
            self.handle_key_map_filter(key);
            return Ok(());
        }
        // Broadcast cancel is claimed on every tab, as the footer says.
        if self.try_broadcast_cancel(&key) {
            return Ok(());
        }

        if self.try_tab_switch(&key)? {
            return Ok(());
        }

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            _ if self.is_action(KeyAction::Help, &key) => self.open_help(),
            _ if self.is_action(KeyAction::MapDown, &key) => self.map_move_row(1),
            _ if self.is_action(KeyAction::MapUp, &key) => self.map_move_row(-1),
            _ if self.is_action(KeyAction::MapRight, &key) => self.map_move_col(1),
            _ if self.is_action(KeyAction::MapLeft, &key) => self.map_move_col(-1),
            _ if self.is_action(KeyAction::Connect, &key) => self.map_connect()?,
            _ if self.is_action(KeyAction::Edit, &key) => self.map_edit()?,
            _ if self.is_action(KeyAction::Search, &key) => {
                if let Some(v) = self.map_view.as_mut() {
                    v.editing = true;
                }
            }
            _ if self.is_action(KeyAction::TagFilter, &key) => {
                if self.map_view.as_ref().is_some_and(|v| v.outline.get()) {
                    self.host_notice =
                        Some("the tag picker needs a bigger terminal (80x24)".into());
                    return Ok(());
                }
                if let Some(v) = self.map_view.as_mut() {
                    let items = v.all_tags();
                    if items.is_empty() {
                        self.host_notice = Some("no tags on any host".into());
                    } else {
                        v.picker = Some(TagPicker::new(items));
                    }
                }
            }
            _ if self.is_action(KeyAction::Cancel, &key) => {
                if self.cancel_or_clear_test(false) {
                    return Ok(());
                }
                if let Some(v) = self.map_view.as_mut() {
                    if v.zoomed {
                        v.zoomed = false;
                        return Ok(());
                    }
                    // Lens first, then the filter.
                    if !matches!(v.lens, Lens::None) {
                        v.lens = Lens::None;
                    } else if !v.query.is_empty() {
                        v.query.clear();
                        v.apply_filter();
                    }
                }
            }
            _ if self.is_action(KeyAction::MapBlast, &key) => self.map_toggle_blast(),
            _ if self.is_action(KeyAction::MapNextDependent, &key) => {
                if let Some(v) = self.map_view.as_mut() {
                    v.blast_step(1);
                }
            }
            _ if self.is_action(KeyAction::MapPrevDependent, &key) => {
                if let Some(v) = self.map_view.as_mut() {
                    v.blast_step(-1);
                }
            }
            _ if self.is_action(KeyAction::Broadcast, &key) => self.map_broadcast(),
            _ if self.is_action(KeyAction::MapTunnelLens, &key) => {
                if let Some(v) = self.map_view.as_mut() {
                    v.lens = match v.lens {
                        Lens::Tunnel => Lens::None,
                        _ => Lens::Tunnel,
                    };
                }
            }
            _ if self.is_action(KeyAction::MapToggleTunnel, &key) => self.map_toggle_tunnel()?,
            _ if self.is_action(KeyAction::MapUseAsJump, &key) => self.map_open_use_picker(),
            _ if self.is_action(KeyAction::MapTest, &key) => self.map_start_test(),
            _ if self.is_action(KeyAction::MapCopyCmd, &key) => self.map_copy_command(),
            _ if self.is_action(KeyAction::GroupsManage, &key) => self.enter_group_manage()?,
            _ if self.is_action(KeyAction::TogglePanelZoom, &key) => {
                if let Some(v) = self.map_view.as_mut() {
                    v.zoomed = !v.zoomed;
                }
            }
            _ if self.is_action(KeyAction::AddHost, &key) => self.map_save_raw_hop()?,
            _ if self.is_action(KeyAction::KnownHosts, &key) => self.open_known_hosts(),
            _ if self.is_action(KeyAction::PushKey, &key) => self.map_push_key()?,
            _ if self.is_action(KeyAction::MapSshLog, &key) => self.map_show_ssh_log(),
            _ if self.is_action(KeyAction::MapFold, &key) => {
                if let Some(v) = self.map_view.as_mut() {
                    v.toggle_fold();
                }
            }
            _ if self.is_action(KeyAction::MapFoldAll, &key) => {
                if let Some(v) = self.map_view.as_mut() {
                    v.fold_all();
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Keys while the filter line is open: typing edits the query live, Enter
    /// keeps it, Esc clears it.
    fn handle_key_map_filter(&mut self, key: KeyEvent) {
        let Some(v) = self.map_view.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                v.query.clear();
                v.editing = false;
            }
            KeyCode::Enter => {
                v.editing = false;
                v.reveal_matches();
                return;
            }
            KeyCode::Tab => v.complete_filter(),
            KeyCode::Backspace => {
                v.query.pop();
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                v.query.push(c);
            }
            _ => return,
        }
        v.apply_filter();
    }

    fn handle_key_map_picker(&mut self, key: KeyEvent) -> Result<()> {
        let down = self.is_action(KeyAction::MapDown, &key);
        let up = self.is_action(KeyAction::MapUp, &key);
        let Some(v) = self.map_view.as_mut() else {
            return Ok(());
        };
        let Some(p) = v.picker.as_mut() else {
            return Ok(());
        };
        let visible = p.visible();
        let last = visible.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => v.picker = None,
            KeyCode::Enter => {
                let Some(&i) = visible.get(p.selected) else {
                    self.host_notice = Some("no tag matches the typed text".into());
                    return Ok(());
                };
                let tag = p.items[i].clone();
                let mut terms = crate::map::filter::split_terms(&v.query);
                let unquoted = format!("tag:{tag}");
                if let Some(at) = terms.iter().position(|t| *t == unquoted) {
                    terms.remove(at);
                } else {
                    terms.push(unquoted);
                }
                v.query = crate::map::filter::join_terms(&terms);
                v.picker = None;
                v.apply_filter();
            }
            KeyCode::Down => p.selected = (p.selected + 1).min(last),
            KeyCode::Up => p.selected = p.selected.saturating_sub(1),
            KeyCode::Backspace => {
                p.query.pop();
                p.selected = 0;
            }
            // Letters are query text, even ones bound to map movement.
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                p.query.push(c);
                p.selected = 0;
            }
            _ if down => p.selected = (p.selected + 1).min(last),
            _ if up => p.selected = p.selected.saturating_sub(1),
            _ => {}
        }
        Ok(())
    }

    /// `u`: pick the host that will use the hop under the cursor as its jump.
    /// The hop `u` would use: the one under the cursor, else the host selected
    /// on the hosts tab.
    fn map_hop_for_use(&self) -> Option<HopKey> {
        let v = self.map_view.as_ref()?;
        match v.selection() {
            Some(MapSel::Hop(n)) => return Some(v.map.nodes[n].key.clone()),
            Some(MapSel::Host(h)) => return Some(HopKey::Saved(h)),
            _ => {}
        }
        let name = self.selected_entry()?.name().to_string();
        v.map
            .hosts
            .iter()
            .position(|h| h.name == name)
            .map(HopKey::Saved)
    }

    /// The host `e` would edit: the cursor's, else the hosts-tab selection.
    fn map_host_for_edit(&self) -> Option<usize> {
        let v = self.map_view.as_ref()?;
        let on_cursor = match v.selection() {
            Some(MapSel::Host(h)) => Some(h),
            Some(MapSel::Hop(n)) => match v.map.nodes[n].key {
                HopKey::Saved(h) | HopKey::ProxyCmd(h) => Some(h),
                _ => None,
            },
            Some(MapSel::Cycle(i)) => v.map.cycles[i].path.first().copied(),
            _ => None,
        };
        on_cursor.or_else(|| {
            let name = self.selected_entry()?.name().to_string();
            v.map.hosts.iter().position(|h| h.name == name)
        })
    }

    fn map_open_use_picker(&mut self) {
        let Some(v) = self.map_view.as_ref() else {
            return;
        };
        if v.outline.get() {
            self.host_notice = Some("use as jump needs a bigger terminal (80x24)".into());
            return;
        }
        let Some(hop) = self.map_hop_for_use() else {
            // No hop to start from: pick one, then pick who takes it.
            self.open_host_picker(UsePurpose::PickHop);
            return;
        };
        let v = self.map_view.as_mut().expect("checked above");
        let (token, own) = match &hop {
            HopKey::Saved(h) => (v.map.hosts[*h].name.clone(), Some(*h)),
            HopKey::Raw(r) => (r.label(), None),
            _ => {
                self.host_notice = Some("that hop cannot be used as a jump".into());
                return;
            }
        };
        if let Some(issue) = crate::map::hops::token_issue(&token) {
            self.host_notice = Some(format!("{token:?} cannot be a jump hop: {issue}"));
            return;
        }
        // A loop forms when the hop itself depends on the target.
        let depends: HashSet<usize> = own
            .map(|h| {
                v.map
                    .chain_keys(h)
                    .into_iter()
                    .filter_map(|k| match k {
                        HopKey::Saved(s) => Some(s),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut hosts: Vec<(usize, bool)> = (0..v.map.hosts.len())
            .map(|h| (h, Some(h) == own || depends.contains(&h)))
            .collect();
        hosts.sort_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| v.map.hosts[a.0].name.cmp(&v.map.hosts[b.0].name))
        });
        v.use_picker = Some(UsePicker {
            purpose: UsePurpose::Apply,
            token,
            query: String::new(),
            selected: 0,
            append: false,
            hosts,
        });
    }

    /// A plain host picker (no loop checks): which host to edit, or which host
    /// to use as a jump.
    fn open_host_picker(&mut self, purpose: UsePurpose) {
        let Some(v) = self.map_view.as_mut() else {
            return;
        };
        let mut hosts: Vec<(usize, bool)> = (0..v.map.hosts.len()).map(|h| (h, false)).collect();
        hosts.sort_by(|a, b| v.map.hosts[a.0].name.cmp(&v.map.hosts[b.0].name));
        v.use_picker = Some(UsePicker {
            purpose,
            token: String::new(),
            query: String::new(),
            selected: 0,
            append: false,
            hosts,
        });
    }

    fn handle_key_map_use(&mut self, key: KeyEvent) -> Result<()> {
        let down = self.is_action(KeyAction::MapDown, &key);
        let up = self.is_action(KeyAction::MapUp, &key);
        let Some(v) = self.map_view.as_mut() else {
            return Ok(());
        };
        let Some(p) = v.use_picker.as_mut() else {
            return Ok(());
        };
        let visible = p.visible(&v.map);
        match key.code {
            KeyCode::Esc => v.use_picker = None,
            KeyCode::Tab => p.append = !p.append,
            KeyCode::Backspace => {
                p.query.pop();
                p.selected = 0;
            }
            KeyCode::Enter => {
                let Some(&(h, loops)) = visible.get(p.selected) else {
                    return Ok(());
                };
                if loops {
                    self.host_notice = Some("that would loop: the hop depends on this host".into());
                    return Ok(());
                }
                let (purpose, token, append) = (p.purpose, p.token.clone(), p.append);
                v.use_picker = None;
                match purpose {
                    UsePurpose::Apply => self.map_apply_jump(h, &token, append)?,
                    UsePurpose::Edit => {
                        if self.reveal_host(h) {
                            self.edit_selected_host()?;
                        }
                    }
                    UsePurpose::PickHop => {
                        // The chosen host is the hop: now pick who takes it.
                        if self.reveal_host(h) {
                            self.map_open_use_picker();
                        }
                    }
                }
            }
            _ if down => p.selected = (p.selected + 1).min(visible.len().saturating_sub(1)),
            _ if up => p.selected = p.selected.saturating_sub(1),
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                p.query.push(c);
                p.selected = 0;
            }
            _ => {}
        }
        Ok(())
    }

    /// Set (or append) `token` as a hop of host `h`'s own ProxyJump and save.
    fn map_apply_jump(&mut self, h: usize, token: &str, append: bool) -> Result<()> {
        let Some(entry) = self.hosts.get(h) else {
            return Ok(());
        };
        let Some(m) = entry.managed().filter(|m| m.source == HostSource::Launcher) else {
            self.host_notice =
                Some("ssh_config hosts are read-only here: edit ~/.ssh/config".into());
            return Ok(());
        };
        let (id, name) = (m.id, m.name.clone());
        let mut tokens = if append {
            // Start from what the host uses now, own or inherited.
            let eff = self
                .store
                .resolve_connection(m)
                .ok()
                .and_then(|r| r.proxy_jump)
                .unwrap_or_default();
            crate::map::hops::split(&eff)
        } else {
            Vec::new()
        };
        if !tokens.iter().any(|t| t == token) {
            tokens.push(token.to_string());
        }
        let value = crate::map::hops::join(&tokens);
        // The form refuses to save what ssh -J must not get; so does this.
        if let Some(problem) = crate::map::hops::jump_problem(&value) {
            self.host_notice = Some(format!("{name}: {problem}"));
            return Ok(());
        }
        // A store failure is a message, not the end of every embedded session.
        let saved = self
            .store
            .update_host(
                id,
                &crate::store::HostUpdate {
                    proxy_jump: Some(Some(value.clone())),
                    ..Default::default()
                },
            )
            .and_then(|_| self.reload_hosts());
        self.host_notice = Some(match saved {
            Ok(()) => format!("{name}: ProxyJump = {value}"),
            Err(e) => format!("{name}: cannot save ProxyJump: {e:#}"),
        });
        Ok(())
    }

    /// `B`: blast radius of the hop under the cursor, or off again.
    fn map_toggle_blast(&mut self) {
        let Some(v) = self.map_view.as_mut() else {
            return;
        };
        if matches!(v.lens, Lens::Blast(_)) {
            v.lens = Lens::None;
            return;
        }
        let Some(key) = v.blast_key_at_cursor() else {
            self.host_notice = Some("blast radius needs a hop or host under the cursor".into());
            return;
        };
        let hosts: HashSet<usize> = v.map.blast_radius(&key).into_iter().collect();
        v.lens = Lens::Blast(BlastLens { key, hosts });
    }

    /// `b` inside the blast lens: broadcast to the dependents.
    fn map_broadcast(&mut self) {
        let Some(v) = self.map_view.as_ref() else {
            return;
        };
        let Lens::Blast(b) = &v.lens else {
            self.host_notice = Some("broadcast on the map works from the blast lens (B)".into());
            return;
        };
        let mut ids: Vec<i64> = b
            .hosts
            .iter()
            .filter_map(|&h| self.hosts.get(h).and_then(|e| e.managed_id()))
            .collect();
        ids.sort_unstable();
        let label = format!("blast: {} ({} hosts)", v.map.key_label(&b.key), ids.len());
        self.open_broadcast_for_hosts(ids, label);
    }

    /// `x`: start or stop the tunnel of the host under the cursor.
    fn map_toggle_tunnel(&mut self) -> Result<()> {
        // An ancestor hop drawn on another row's path is not this row's own.
        if self.map_view.as_ref().is_some_and(|v| {
            matches!(v.selection(), Some(MapSel::Hop(_))) && v.pos + 1 < v.hop_count(v.row)
        }) {
            self.host_notice =
                Some("cursor is on an ancestor hop: move to that hop's own row to use x".into());
            return Ok(());
        }
        let host = self.map_view.as_ref().and_then(|v| match v.selection()? {
            MapSel::Host(h) => Some(h),
            MapSel::Hop(n) => match v.map.nodes[n].key {
                HopKey::Saved(h) => Some(h),
                _ => None,
            },
            _ => None,
        });
        let host_id = host
            .and_then(|h| self.hosts.get(h))
            .and_then(|e| e.managed_id());
        let mut on_host = self
            .tunnels
            .iter()
            .filter(|t| host_id.is_some() && t.host_id == host_id)
            .map(|t| t.id);
        let id = on_host.next();
        if on_host.next().is_some() {
            // Which one is meant is not clear from the map.
            self.host_notice = Some("several tunnels on this host: use the tunnels tab".into());
            return Ok(());
        }
        match id {
            Some(id) => {
                self.tunnel_notice = None;
                // A store or ssh failure is a message, not the end of every
                // embedded session.
                if let Err(e) = self.toggle_tunnel_by_id(id) {
                    self.tunnel_notice = Some(format!("tunnel: {e:#}"));
                }
                self.host_notice = self.tunnel_notice.take();
            }
            None => self.host_notice = Some("no tunnel on this host".into()),
        }
        Ok(())
    }

    /// A click on the map body: select the hop or glyph the last frame drew at
    /// (`x`, `y`); a double click connects to it.
    pub(crate) fn map_click(&mut self, x: u16, y: u16, is_double: bool) -> Result<()> {
        if self
            .map_view
            .as_ref()
            .is_none_or(|v| v.picker.is_some() || v.use_picker.is_some() || v.editing)
        {
            return Ok(());
        }
        let hit = self
            .click_map
            .borrow()
            .iter()
            .rev()
            .find(|t| t.area.contains(ratatui::layout::Position::new(x, y)))
            .and_then(|t| match t.hit {
                ClickHit::MapCell { row, pos } => Some((row, pos)),
                _ => None,
            });
        let Some((row, pos)) = hit else {
            return Ok(());
        };
        if let Some(v) = self.map_view.as_mut() {
            v.row = row;
            v.pos = pos;
            v.clamp();
        }
        if is_double {
            self.map_connect()?;
        }
        Ok(())
    }

    /// The host the failed-test actions are about: the hop that failed, else
    /// what the cursor is on. `None` when neither is a saved host.
    fn map_failed_host(&self) -> Option<usize> {
        let by_name = |n: &str| self.hosts.iter().position(|h| h.name() == n);
        if let Some((i, _, _)) = self.test_run.as_ref().and_then(|r| r.failure()) {
            let step = &self.test_run.as_ref()?.steps[i];
            if let Some(h) = step.host.as_deref().and_then(by_name) {
                return Some(h);
            }
        }
        let v = self.map_view.as_ref()?;
        let h = match v.selection()? {
            MapSel::Host(h) => h,
            MapSel::Hop(n) => match v.map.nodes[n].key {
                HopKey::Saved(h) => h,
                _ => return None,
            },
            _ => return None,
        };
        by_name(&v.map.hosts[h].name)
    }

    /// `P`: push a public key to the failed hop (or the host under the cursor)
    /// through the usual identity picker.
    fn map_push_key(&mut self) -> Result<()> {
        let Some(h) = self.map_failed_host() else {
            self.host_notice = Some("no saved host to push a key to here".into());
            return Ok(());
        };
        if self.hosts[h].managed().is_none() {
            self.host_notice = Some("push key needs a saved host".into());
            return Ok(());
        }
        if !self.reveal_host(h) {
            return Ok(());
        }
        self.trigger_push_key_from_hosts()
    }

    /// `l`: the ssh log of the failed hop on the hosts tab, zoomed.
    fn map_show_ssh_log(&mut self) {
        let Some(h) = self.map_failed_host() else {
            self.host_notice = Some("no saved host to show a log for here".into());
            return;
        };
        if !self.reveal_host(h) {
            return;
        }
        self.active_tab = 0;
        self.focused_panel = PanelId::SshLog;
        self.panel_zoomed = true;
    }

    /// `y`: copy the real command for the cursor's path, shell-quoted.
    fn map_copy_command(&mut self) {
        // In the blast lens `y` copies the dependents instead: one name a line.
        if let Some(Lens::Blast(b)) = self.map_view.as_ref().map(|v| &v.lens) {
            let map = &self.map_view.as_ref().expect("lens").map;
            let mut names: Vec<&str> = b
                .hosts
                .iter()
                .map(|&h| map.hosts[h].name.as_str())
                .collect();
            names.sort_unstable();
            let list = names.join("\n");
            self.host_notice = Some(match write_osc52(&list) {
                Ok(()) => format!("copied {} host names", names.len()),
                Err(e) => format!("clipboard copy failed: {e}"),
            });
            return;
        }
        let Some(cmd) = self.map_command_line() else {
            if self.host_notice.is_none() {
                self.host_notice = Some("no command for this row".into());
            }
            return;
        };
        self.host_notice = Some(match write_osc52(&cmd) {
            Ok(()) => format!("copied: {cmd}"),
            Err(e) => format!("clipboard copy failed: {e}"),
        });
    }

    /// `a` on a raw hop: open the new-host form filled from it.
    fn map_save_raw_hop(&mut self) -> Result<()> {
        let raw = self.map_view.as_ref().and_then(|v| match v.selection()? {
            MapSel::Hop(n) => match &v.map.nodes[n].key {
                HopKey::Raw(r) => Some(r.clone()),
                _ => None,
            },
            _ => None,
        });
        let Some(raw) = raw else {
            self.host_notice = Some(
                "a saves a raw hop as a host: put the cursor on one (add hosts on the hosts tab)"
                    .into(),
            );
            return Ok(());
        };
        self.enter_host_form(None, false)?;
        if let Some(f) = self.host_form.as_mut() {
            f.address = raw.host.clone();
            f.name = raw.host.clone();
            f.username = raw.user.unwrap_or_default();
            f.port = raw.port.map(|p| p.to_string()).unwrap_or_default();
            // The form picks up the group the hosts tab happens to stand on;
            // nothing says this hop belongs there.
            f.group_ids.clear();
            f.group_index = 0;
            // Prefilled, not edited: Esc closes without asking.
            f.dirty = false;
        }
        Ok(())
    }

    /// Wheel over the map: move the cursor row (the view follows it).
    pub(crate) fn map_wheel(&mut self, delta: isize) {
        self.map_move_row(delta);
    }

    /// Bracketed paste into the map's text inputs (the host and tag pickers,
    /// the filter line). `true` when the map took the text. Newlines and tabs
    /// become single spaces: every input is one line.
    pub(crate) fn map_paste(&mut self, text: &str) -> bool {
        if self.active_tab != 5 || self.mode != AppMode::Normal {
            return false;
        }
        let Some(v) = self.map_view.as_mut() else {
            return false;
        };
        let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if let Some(p) = v.use_picker.as_mut() {
            p.query.push_str(&line);
            p.selected = 0;
        } else if let Some(p) = v.picker.as_mut() {
            p.query.push_str(&line);
            p.selected = 0;
        } else if v.editing {
            v.query.push_str(&line);
            v.apply_filter();
        } else {
            return false;
        }
        true
    }

    fn map_move_row(&mut self, delta: isize) {
        if let Some(v) = self.map_view.as_mut() {
            v.move_row(delta);
        }
    }

    fn map_move_col(&mut self, delta: isize) {
        if let Some(v) = self.map_view.as_mut() {
            v.move_col(delta);
        }
    }

    /// Enter: connect through the normal connect path for what the cursor is
    /// on. The hosts of a jump loop are not connectable.
    fn map_connect(&mut self) -> Result<()> {
        let Some(v) = self.map_view.as_ref() else {
            return Ok(());
        };
        let Some(sel) = v.selection() else {
            return Ok(());
        };
        if matches!(sel, MapSel::Cluster(_) | MapSel::More(_)) {
            if let Some(v) = self.map_view.as_mut() {
                v.toggle_fold();
            }
            return Ok(());
        }
        let in_loop = |h: usize| matches!(v.map.chains[h], crate::map::Chain::Cycle(_));
        match sel {
            MapSel::Host(h) if in_loop(h) => {
                self.host_notice = Some("jump loop: not connectable, fix the ProxyJump".into());
            }
            MapSel::Host(h) => self.connect_host_at(h)?,
            MapSel::Hop(n) => match v.map.nodes[n].key.clone() {
                HopKey::Saved(h) | HopKey::ProxyCmd(h) => self.connect_host_at(h)?,
                HopKey::Raw(r) => {
                    let target = crate::app::adhoc::AdhocTarget {
                        user: r.user,
                        host: r.host,
                        port: r.port,
                        ipv6: r.ipv6,
                    };
                    self.connect_adhoc(target)?;
                }
                HopKey::Invalid(t) => {
                    self.host_notice = Some(format!("hop {t:?} is not a valid [user@]host[:port]"));
                }
            },
            MapSel::Cycle(_) => {
                // The loop is not connectable: open a host of it to fix it.
                self.map_edit()?;
                self.host_notice = Some(
                    "jump loop, not connectable: clear a ProxyJump in it (ssh_config hosts: edit ~/.ssh/config)"
                        .into(),
                );
            }
            MapSel::Direct => {
                if let Some(v) = self.map_view.as_mut() {
                    v.toggle_fold();
                }
            }
            MapSel::Cluster(_) | MapSel::More(_) => {}
        }
        Ok(())
    }

    /// `e`: open the host form for the host the cursor stands for.
    fn map_edit(&mut self) -> Result<()> {
        match self.map_host_for_edit() {
            Some(h) if self.reveal_host(h) => self.edit_selected_host(),
            Some(_) => Ok(()),
            None => {
                self.open_host_picker(UsePurpose::Edit);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{host, key_char, test_app};
    use crate::map::MapHost;

    fn jump_fleet() -> App {
        let mut leaf = host("leaf");
        leaf.proxy_jump = Some("hub".into());
        let mut app = test_app(vec![
            ("aa", host("aa")),
            ("hub", host("hub")),
            ("leaf", leaf),
        ]);
        app.switch_to_map_tab();
        app
    }

    fn idx(v: &MapView, name: &str) -> usize {
        v.map.hosts.iter().position(|h| h.name == name).unwrap()
    }

    fn mh(name: &str, jump: Option<&str>) -> MapHost {
        MapHost {
            name: name.into(),
            address: format!("{name}.x"),
            port: 22,
            group: "g".into(),
            jump: jump.map(String::from),
            jump_source: jump.map(|_| crate::map::JumpSource::Own),
            ..Default::default()
        }
    }

    #[test]
    fn rebuild_carries_lens_and_picker_by_host_name_not_index() {
        let mut app = jump_fleet();
        let v = app.map_view.as_mut().unwrap();
        let (hub, leaf) = (idx(v, "hub"), idx(v, "leaf"));
        let key = HopKey::Saved(hub);
        let hosts: HashSet<usize> = v.map.blast_radius(&key).into_iter().collect();
        v.lens = Lens::Blast(BlastLens { key, hosts });
        v.use_picker = Some(UsePicker {
            purpose: UsePurpose::PickHop,
            token: String::new(),
            query: "le".into(),
            selected: 0,
            append: false,
            hosts: vec![(leaf, false)],
        });
        // A host sorted before both disappears: every index shifts.
        app.hosts.retain(|h| h.name() != "aa");
        app.rebuild_map();

        let v = app.map_view.as_ref().unwrap();
        let Lens::Blast(b) = &v.lens else {
            panic!("the lens must survive the rebuild")
        };
        let HopKey::Saved(k) = b.key else {
            panic!("a saved hop stays saved")
        };
        assert_eq!(v.map.hosts[k].name, "hub");
        let radius: Vec<&str> = b
            .hosts
            .iter()
            .map(|&h| v.map.hosts[h].name.as_str())
            .collect();
        assert_eq!(radius, ["leaf"]);
        let p = v.use_picker.as_ref().expect("the open picker survives");
        assert_eq!(p.query, "le");
        let listed: Vec<&str> = p
            .hosts
            .iter()
            .map(|&(h, _)| v.map.hosts[h].name.as_str())
            .collect();
        assert_eq!(listed, ["leaf"]);
    }

    #[test]
    fn typing_in_the_use_picker_is_not_taken_by_global_binds() {
        let mut app = jump_fleet();
        app.handle_key(key_char('u')).unwrap();
        assert!(app.map_view.as_ref().unwrap().use_picker.is_some());
        // `S` / `L` open snippets / the log browser when nothing is typing.
        for c in ['S', 'L'] {
            app.handle_key(key_char(c)).unwrap();
        }
        assert_eq!(app.mode, AppMode::Normal);
        let p = app.map_view.as_ref().unwrap().use_picker.as_ref().unwrap();
        assert_eq!(p.query, "SL");
    }

    #[test]
    fn broadcast_cancel_key_acts_on_the_map_tab_instead_of_toggling_a_tunnel() {
        let mut app = jump_fleet();
        app.broadcast_toasts.push(BroadcastToast {
            host: "h".into(),
            text: "t".into(),
            born: std::time::Instant::now(),
        });
        app.handle_key(key_char('x')).unwrap();
        assert!(app.broadcast_toasts.is_empty());
        assert!(
            app.host_notice.is_none(),
            "x must not reach the tunnel toggle"
        );
    }

    #[test]
    fn leaving_the_tab_closes_the_map_inputs() {
        let mut app = jump_fleet();
        let v = app.map_view.as_mut().unwrap();
        v.editing = true;
        v.picker = Some(TagPicker::new(vec!["a".into()]));
        app.reset_tab_input_state();
        let v = app.map_view.as_ref().unwrap();
        assert!(!v.editing && v.picker.is_none() && v.use_picker.is_none());
    }

    #[test]
    fn typing_in_the_tag_picker_narrows_it() {
        let mut app = jump_fleet();
        app.map_view.as_mut().unwrap().picker = Some(TagPicker::new(vec![
            "db".into(),
            "pci".into(),
            "prod".into(),
        ]));
        // Letters are query text, not map or global binds.
        for c in "pci".chars() {
            app.handle_key(key_char(c)).unwrap();
        }
        assert_eq!(app.mode, AppMode::Normal);
        let p = app.map_view.as_ref().unwrap().picker.as_ref().unwrap();
        assert_eq!(p.query, "pci");
        assert_eq!(p.visible(), vec![1]);
        app.handle_key(crate::app::tests::key(crossterm::event::KeyCode::Enter))
            .unwrap();
        assert_eq!(app.map_view.as_ref().unwrap().query, "tag:pci");
    }

    #[test]
    fn paste_lands_in_the_filter_line_only_while_it_is_open() {
        let mut app = jump_fleet();
        assert!(!app.map_paste("hub"));
        app.map_view.as_mut().unwrap().editing = true;
        assert!(app.map_paste("tag:x\nhub"));
        assert_eq!(app.map_view.as_ref().unwrap().query, "tag:x hub");
    }

    #[test]
    fn filter_matching_a_folded_hop_keeps_a_row() {
        // edge -> mid -> leaf: `edge` has one child and no hosts of its own,
        // so it is folded and draws no row.
        let map = Map::build(vec![
            mh("edge-gw", None),
            mh("mid", None),
            mh("leaf", Some("edge-gw,mid")),
        ]);
        let mut v = MapView::new(map);
        v.query = "edge-gw".into();
        v.apply_filter();
        assert!(
            !v.rows.is_empty(),
            "the match must not vanish with the fold"
        );
    }

    #[test]
    fn x_on_an_ancestor_hop_says_so_and_touches_nothing() {
        let mut app = jump_fleet();
        app.map_view = Some(MapView::new(Map::build(vec![
            mh("edge-gw", None),
            mh("mid", None),
            mh("leaf", Some("edge-gw,mid")),
        ])));
        let v = app.map_view.as_mut().unwrap();
        v.row = 0;
        v.pos = 0;
        assert!(v.hop_count(0) >= 2, "row 0 must carry an ancestor hop");
        assert!(matches!(v.selection(), Some(MapSel::Hop(_))));
        app.map_toggle_tunnel().unwrap();
        assert!(app.host_notice.as_deref().unwrap().contains("ancestor hop"));
    }

    #[test]
    fn rebuild_error_outlives_the_next_map_key() {
        let mut app = jump_fleet();
        app.map_error = Some("map: boom".into());
        app.handle_key_map(key_char('j')).unwrap();
        assert_eq!(app.host_notice.as_deref(), Some("map: boom"));
        app.map_error = None;
        app.handle_key_map(key_char('j')).unwrap();
        assert_eq!(app.host_notice, None);
    }

    #[test]
    fn tab_completion_and_tag_picker_quote_tags_with_spaces() {
        let mut a = mh("a", None);
        a.tags = vec!["on call".into()];
        let mut v = MapView::new(Map::build(vec![a]));
        v.query = "tag:on".into();
        v.complete_filter();
        assert_eq!(v.query, "tag:\"on call\"");
        v.apply_filter();
        assert_eq!(v.matched.as_ref().map(|m| m.len()), Some(1));
    }

    #[test]
    fn ctrl_h_is_backspace_in_the_filter_line_not_settings() {
        let mut app = jump_fleet();
        app.map_view.as_mut().unwrap().editing = true;
        app.map_view.as_mut().unwrap().query = "ab".into();
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.map_view.as_ref().unwrap().query, "a");
    }

    #[test]
    fn loop_row_walks_hosts_in_drawn_group_order() {
        let mut a = mh("a", Some("b"));
        let mut b = mh("b", Some("a"));
        a.group = "zeta".into();
        b.group = "alpha".into();
        // Only ssh_config hops follow their own jump, so only they can loop.
        a.ssh_config = true;
        b.ssh_config = true;
        let map = Map::build(vec![a, b]);
        let row = Row::Cycle(0);
        let order: Vec<&str> = map
            .row_hosts(row)
            .into_iter()
            .map(|h| map.hosts[h].group.as_str())
            .collect();
        assert_eq!(order, ["alpha", "zeta"]);
    }
}
