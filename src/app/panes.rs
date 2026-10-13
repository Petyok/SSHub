//! Split view (#73): the session body divided into 2 to 4 panes.
//!
//! The layout is a binary tree of fixed depth 2: the root splits the body in
//! two, and each half may split once more, so at most four panes. Ratios are
//! always equal. Panes hold [`SessionId`]s, never indices into
//! `App::sessions`, because that `Vec` shifts on every close.
//!
//! The one geometry function is [`App::pane_views`]: the renderer, the mouse,
//! the theme protection pass and the per-frame resize all read the pane rects
//! from it, so they cannot disagree about where a session is drawn.

use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

use super::*;
use crate::session::SessionId;

/// How many splits a pane can sit under: two, so at most four panes.
const MAX_DEPTH: usize = 2;

/// Direction of the rule between the two halves of a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDir {
    /// Side by side, divided by a vertical rule.
    Vertical,
    /// Stacked, divided by a horizontal rule.
    Horizontal,
}

/// A layout tree. Leaves are numbered in reading order (first half before
/// second half, depth first), which is also the focus order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneTree<T> {
    Leaf(T),
    Split {
        dir: SplitDir,
        a: Box<PaneTree<T>>,
        b: Box<PaneTree<T>>,
    },
}

impl<T: Clone> PaneTree<T> {
    pub fn split_of(dir: SplitDir, a: T, b: T) -> Self {
        Self::Split {
            dir,
            a: Box::new(Self::Leaf(a)),
            b: Box::new(Self::Leaf(b)),
        }
    }

    pub fn leaves(&self) -> Vec<&T> {
        match self {
            Self::Leaf(t) => vec![t],
            Self::Split { a, b, .. } => {
                let mut out = a.leaves();
                out.extend(b.leaves());
                out
            }
        }
    }

    fn leaves_mut(&mut self) -> Vec<&mut T> {
        match self {
            Self::Leaf(t) => vec![t],
            Self::Split { a, b, .. } => {
                let mut out = a.leaves_mut();
                out.extend(b.leaves_mut());
                out
            }
        }
    }

    pub fn pane_count(&self) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Split { a, b, .. } => a.pane_count() + b.pane_count(),
        }
    }

    /// Path to leaf `i`: `false` takes the first half, `true` the second.
    fn path(&self, i: usize) -> Option<Vec<bool>> {
        fn walk<T>(node: &PaneTree<T>, i: &mut usize, path: &mut Vec<bool>) -> bool {
            match node {
                PaneTree::Leaf(_) if *i == 0 => true,
                PaneTree::Leaf(_) => {
                    *i -= 1;
                    false
                }
                PaneTree::Split { a, b, .. } => {
                    for (half, second) in [(a, false), (b, true)] {
                        path.push(second);
                        if walk(half, i, path) {
                            return true;
                        }
                        path.pop();
                    }
                    false
                }
            }
        }
        let mut path = Vec::new();
        walk(self, &mut { i }, &mut path).then_some(path)
    }

    fn node_mut(&mut self, path: &[bool]) -> &mut Self {
        match (self, path.split_first()) {
            (Self::Split { a, b, .. }, Some((second, rest))) => {
                (if *second { b } else { a }).node_mut(rest)
            }
            (node, _) => node,
        }
    }

    /// Whether leaf `i` exists and is above the depth limit, so it can split.
    pub fn can_split(&self, i: usize) -> bool {
        self.path(i).is_some_and(|p| p.len() < MAX_DEPTH)
    }

    /// Split leaf `i` in `dir`, putting `new` right after it (leaf `i + 1`).
    /// `false`, and nothing changes, when leaf `i` is already at the depth
    /// limit or does not exist.
    pub fn split(&mut self, i: usize, dir: SplitDir, new: T) -> bool {
        let Some(path) = self.path(i).filter(|p| p.len() < MAX_DEPTH) else {
            return false;
        };
        let node = self.node_mut(&path);
        let old = node.clone();
        *node = Self::Split {
            dir,
            a: Box::new(old),
            b: Box::new(Self::Leaf(new)),
        };
        true
    }

    /// Remove leaf `i`; its sibling takes the parent's place. `false` when the
    /// tree is a single leaf (nothing to remove it from) or `i` is out of range.
    pub fn remove(&mut self, i: usize) -> bool {
        let Some(path) = self.path(i).filter(|p| !p.is_empty()) else {
            return false;
        };
        let (last, parent_path) = path.split_last().expect("non-empty path");
        let parent = self.node_mut(parent_path);
        let Self::Split { a, b, .. } = parent else {
            return false;
        };
        let sibling = if *last { a.as_ref() } else { b.as_ref() }.clone();
        *parent = sibling;
        true
    }

    /// Leaf rects in reading order, and the one-cell rules between them.
    pub fn layout(&self, area: Rect) -> (Vec<Rect>, Vec<Rect>) {
        let mut panes = Vec::new();
        let mut rules = Vec::new();
        self.layout_into(area, &mut panes, &mut rules);
        (panes, rules)
    }

    fn layout_into(&self, area: Rect, panes: &mut Vec<Rect>, rules: &mut Vec<Rect>) {
        match self {
            Self::Leaf(_) => panes.push(area),
            Self::Split { dir, a, b } => {
                let (first, rule, second) = halves(area, *dir);
                a.layout_into(first, panes, rules);
                rules.push(rule);
                b.layout_into(second, panes, rules);
            }
        }
    }

    pub fn map<U>(&self, f: &mut impl FnMut(&T) -> U) -> PaneTree<U> {
        match self {
            Self::Leaf(t) => PaneTree::Leaf(f(t)),
            Self::Split { dir, a, b } => PaneTree::Split {
                dir: *dir,
                a: Box::new(a.map(f)),
                b: Box::new(b.map(f)),
            },
        }
    }
}

/// Split `area` into two equal halves around a one-cell rule. An odd cell goes
/// to the second half.
fn halves(area: Rect, dir: SplitDir) -> (Rect, Rect, Rect) {
    match dir {
        SplitDir::Vertical => {
            let first = area.width.saturating_sub(1) / 2;
            let rule = area.width.min(1);
            let second = area.width - first - rule;
            (
                Rect::new(area.x, area.y, first, area.height),
                Rect::new(area.x + first, area.y, rule, area.height),
                Rect::new(area.x + first + rule, area.y, second, area.height),
            )
        }
        SplitDir::Horizontal => {
            let first = area.height.saturating_sub(1) / 2;
            let rule = area.height.min(1);
            let second = area.height - first - rule;
            (
                Rect::new(area.x, area.y, area.width, first),
                Rect::new(area.x, area.y + first, area.width, rule),
                Rect::new(area.x, area.y + first + rule, area.width, second),
            )
        }
    }
}

/// The live split view. The tree always has at least two leaves; a layout that
/// collapses to one pane goes back to the classic view (`App::panes = None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panes {
    pub tree: PaneTree<Option<SessionId>>,
    /// Leaf index of the focused pane. Keys, paste, the clipboard relay and
    /// the ghost follow it through `App::active_session`.
    pub focused: usize,
    /// The focused pane fills the whole body; the others keep their size.
    pub zoomed: bool,
}

impl Panes {
    fn focused_id(&self) -> Option<SessionId> {
        self.tree
            .leaves()
            .get(self.focused)
            .copied()
            .copied()
            .flatten()
    }

    /// Whether a split key would add a pane: not while zoomed, and only while
    /// the focused pane is above the depth limit.
    pub fn can_split(&self) -> bool {
        !self.zoomed && self.tree.can_split(self.focused)
    }
}

/// One pane drawn this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneView {
    pub rect: Rect,
    /// Index into `App::sessions`; `None` for an empty pane.
    pub session: Option<usize>,
    pub focused: bool,
}

impl App {
    pub(crate) fn session_index(&self, id: SessionId) -> Option<usize> {
        self.sessions.iter().position(|s| s.id == id)
    }

    /// Every pane of the session view at terminal size `frame_area`, in leaf
    /// order. The classic view is one pane holding the active session; a zoomed
    /// split is one pane holding the focused session.
    pub(crate) fn pane_views(&self, frame_area: Rect) -> Vec<PaneView> {
        let body = crate::session::render::remote_pty_rect(frame_area);
        let Some(panes) = self.panes.as_ref() else {
            return vec![PaneView {
                rect: body,
                session: self.active_session.filter(|&i| i < self.sessions.len()),
                focused: true,
            }];
        };
        let index = |id: Option<SessionId>| id.and_then(|id| self.session_index(id));
        if panes.zoomed {
            return vec![PaneView {
                rect: body,
                session: index(panes.focused_id()),
                focused: true,
            }];
        }
        let (rects, _) = panes.tree.layout(body);
        panes
            .tree
            .leaves()
            .into_iter()
            .zip(rects)
            .enumerate()
            .map(|(i, (id, rect))| PaneView {
                rect,
                session: index(*id),
                focused: i == panes.focused,
            })
            .collect()
    }

    /// The rules between the panes drawn this frame (none when zoomed).
    pub(crate) fn pane_rules(&self, frame_area: Rect) -> Vec<Rect> {
        match self.panes.as_ref() {
            Some(panes) if !panes.zoomed => {
                let body = crate::session::render::remote_pty_rect(frame_area);
                panes.tree.layout(body).1
            }
            _ => Vec::new(),
        }
    }

    /// The grid each session should have this frame: its pane's while it is on
    /// screen, `None` (keep the last size) otherwise. A session out of view is
    /// not resized when the terminal changes; it gets one resize when shown.
    pub(crate) fn session_grids(&self) -> Vec<Option<(u16, u16)>> {
        let mut grids = vec![None; self.sessions.len()];
        if self.session_is_rendered() {
            for view in self.pane_views(self.terminal_area) {
                if let Some(i) = view.session {
                    grids[i] = Some(crate::session::render::grid_of(view.rect));
                }
            }
        }
        grids
    }

    /// The grid a session about to be spawned will be shown at: the focused
    /// pane's in a split view (a new session goes there), else the full body.
    pub(crate) fn new_session_grid(&self) -> (u16, u16) {
        // A test app that never rendered has a zero-sized terminal; keep the
        // floor the spawn sites always had.
        let area = Rect::new(
            0,
            0,
            self.terminal_area.width.max(20),
            self.terminal_area.height.max(3),
        );
        let rect = self
            .pane_views(area)
            .into_iter()
            .find(|v| v.focused)
            .map_or_else(|| crate::session::render::remote_pty_rect(area), |v| v.rect);
        crate::session::render::grid_of(rect)
    }

    /// Reconcile the split with `active_session`, which many paths retarget
    /// (cycling, a new tab, the auth prompt, closing). Runs before each frame
    /// is resized and drawn. Panes of closed sessions become empty; while the
    /// split is on screen the active session is always the focused pane's: if
    /// another pane shows it, focus moves there, otherwise it is put into the
    /// focused pane. On the dashboard `active_session` is only the strip
    /// cursor, so browsing the strip does not rearrange the layout.
    pub(crate) fn sync_panes(&mut self) {
        if self.sessions.is_empty() {
            self.panes = None;
            return;
        }
        let rendered = self.session_is_rendered();
        let Some(panes) = self.panes.as_mut() else {
            return;
        };
        for leaf in panes.tree.leaves_mut() {
            if leaf.is_some_and(|id| !self.sessions.iter().any(|s| s.id == id)) {
                *leaf = None;
            }
        }
        if !rendered {
            return;
        }
        match self.active_session.filter(|&i| i < self.sessions.len()) {
            Some(i) => {
                let id = Some(self.sessions[i].id);
                match panes.tree.leaves().iter().position(|leaf| **leaf == id) {
                    Some(pos) => panes.focused = pos,
                    None => *panes.tree.leaves_mut()[panes.focused] = id,
                }
            }
            None => {
                let id = panes.focused_id();
                self.active_session = id.and_then(|id| self.session_index(id));
            }
        }
    }

    /// Point `active_session` at the focused pane's session and derive the
    /// session mode from its phase.
    pub(crate) fn follow_focused_pane(&mut self) {
        if let Some(panes) = self.panes.as_ref() {
            self.active_session = panes.focused_id().and_then(|id| self.session_index(id));
        }
        if is_session_mode(self.mode) {
            self.mode = match self.active_session().map(|s| &s.phase) {
                Some(crate::session::SessionPhase::Connecting { .. }) => AppMode::Connecting,
                _ => AppMode::Session,
            };
        }
        self.reset_ghost();
    }

    /// The next open session, in strip order after `after`, that no pane shows.
    fn next_unshown_session(&self, after: usize) -> Option<SessionId> {
        let shown: Vec<Option<SessionId>> = self
            .panes
            .as_ref()
            .map(|p| p.tree.leaves().into_iter().copied().collect())
            .unwrap_or_default();
        let len = self.sessions.len();
        (1..len)
            .map(|step| self.sessions[(after + step) % len].id)
            .find(|id| !shown.contains(&Some(*id)))
    }

    /// Split the focused pane in `dir` (the whole body when not split yet). The
    /// new pane takes the next open session nobody sees, else stays empty, and
    /// gets the focus. Refused while zoomed and at four panes.
    pub(crate) fn split_pane(&mut self, dir: SplitDir) {
        let active = self.active_session.filter(|&i| i < self.sessions.len());
        match self.panes.as_ref() {
            None => {
                let Some(active) = active else {
                    return;
                };
                let id = self.sessions[active].id;
                let other = self
                    .next_unshown_session(active)
                    .filter(|other| *other != id);
                self.panes = Some(Panes {
                    tree: PaneTree::split_of(dir, Some(id), other),
                    focused: 1,
                    zoomed: false,
                });
            }
            Some(panes) if panes.zoomed => return,
            Some(panes) => {
                let focused = panes.focused;
                let other = self.next_unshown_session(active.unwrap_or(0));
                let panes = self.panes.as_mut().expect("checked above");
                if !panes.tree.split(focused, dir, other) {
                    self.pane_notice("at most four panes, and each half splits once".into());
                    return;
                }
                panes.focused = focused + 1;
            }
        }
        self.follow_focused_pane();
    }

    /// Focus pane `i` (leaf order). Ignored while zoomed, as dashboard focus is
    /// while a panel is zoomed (#18).
    pub(crate) fn focus_pane(&mut self, i: usize) {
        let Some(panes) = self.panes.as_mut() else {
            return;
        };
        if panes.zoomed || i >= panes.tree.pane_count() || i == panes.focused {
            return;
        }
        panes.focused = i;
        self.follow_focused_pane();
    }

    pub(crate) fn focus_next_pane(&mut self) {
        if let Some(panes) = self.panes.as_ref() {
            self.focus_pane((panes.focused + 1) % panes.tree.pane_count());
        }
    }

    /// Show the focused pane over the whole body, or put it back. A real
    /// resize of the focused session, once each way.
    pub(crate) fn toggle_pane_zoom(&mut self) {
        if let Some(panes) = self.panes.as_mut() {
            panes.zoomed = !panes.zoomed;
        }
    }

    /// Remove the focused pane from the layout. Its session keeps running in
    /// the strip. Down to one pane, the split is over.
    pub(crate) fn close_pane(&mut self) {
        let Some(panes) = self.panes.as_mut() else {
            return;
        };
        panes.tree.remove(panes.focused);
        panes.zoomed = false;
        if panes.tree.pane_count() == 1 {
            let last = *panes.tree.leaves()[0];
            self.collapse_to(last);
            return;
        }
        panes.focused = panes.focused.min(panes.tree.pane_count() - 1);
        self.follow_focused_pane();
    }

    /// Back to the classic view, showing the focused pane's session.
    pub(crate) fn unsplit(&mut self) {
        if let Some(panes) = self.panes.as_ref() {
            let focused = panes.focused_id();
            self.collapse_to(focused);
        }
    }

    fn collapse_to(&mut self, id: Option<SessionId>) {
        self.panes = None;
        self.active_session = id
            .and_then(|id| self.session_index(id))
            .or((!self.sessions.is_empty()).then_some(0));
        self.follow_focused_pane();
    }

    /// Put session `index` into the focused pane (the switcher, `Alt+S`). A
    /// session is in at most one pane: if another pane shows it, the two panes
    /// swap sessions. Without a split this is plain retargeting.
    pub(crate) fn show_in_focused_pane(&mut self, index: usize) {
        let Some(id) = self.sessions.get(index).map(|s| s.id) else {
            return;
        };
        if let Some(panes) = self.panes.as_mut() {
            let focused = panes.focused;
            let mut leaves = panes.tree.leaves_mut();
            let current = *leaves[focused];
            if let Some(other) = leaves.iter().position(|leaf| **leaf == Some(id)) {
                *leaves[other] = current;
            }
            *leaves[focused] = Some(id);
        }
        self.active_session = Some(index);
    }

    /// Split-view keys in the session view. `true` when `key` was one of them.
    pub(crate) fn handle_key_panes(&mut self, key: &KeyEvent) -> bool {
        if self.is_action(KeyAction::PaneSplitSideBySide, key) {
            self.split_pane(SplitDir::Vertical);
        } else if self.is_action(KeyAction::PaneSplitStacked, key) {
            self.split_pane(SplitDir::Horizontal);
        } else if self.is_action(KeyAction::PaneFocusNext, key) {
            self.focus_next_pane();
        } else if self.is_action(KeyAction::PaneZoom, key) {
            self.toggle_pane_zoom();
        } else if self.is_action(KeyAction::PaneClose, key) {
            self.close_pane();
        } else if self.is_action(KeyAction::PaneUnsplit, key) {
            self.unsplit();
        } else {
            return false;
        }
        true
    }

    /// A short message over the focused pane, or on the dashboard when no
    /// session is focused.
    pub(crate) fn pane_notice(&mut self, msg: String) {
        match self.active_session_mut() {
            Some(session) => session.set_copy_notice(msg),
            None => self.host_notice = Some(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(n: usize) -> PaneTree<usize> {
        // 2 panes side by side, then split the first and the last stacked.
        let mut t = PaneTree::split_of(SplitDir::Vertical, 0, 1);
        if n >= 3 {
            assert!(t.split(0, SplitDir::Horizontal, 2));
        }
        if n >= 4 {
            assert!(t.split(2, SplitDir::Horizontal, 3));
        }
        t
    }

    #[test]
    fn splits_insert_after_the_split_pane_and_stop_at_depth_two() {
        // Three panes: leaves 0 and 1 sit at depth 2, leaf 2 can still split.
        let three = tree(3);
        assert!(!three.can_split(0) && !three.can_split(1) && three.can_split(2));
        let mut panes = Panes {
            tree: PaneTree::split_of(SplitDir::Vertical, None, None),
            focused: 0,
            zoomed: false,
        };
        assert!(panes.can_split());
        panes.zoomed = true;
        assert!(!panes.can_split(), "no split while zoomed");

        let mut t = tree(4);
        // Reading order: first column top/bottom, second column top/bottom.
        assert_eq!(t.leaves(), vec![&0, &2, &1, &3]);
        // Every leaf is at depth 2 now: no fifth pane anywhere.
        for i in 0..4 {
            assert!(!t.can_split(i), "leaf {i}");
            assert!(!t.split(i, SplitDir::Vertical, 9), "leaf {i}");
        }
        assert_eq!(t.pane_count(), 4);
        assert!(!t.split(7, SplitDir::Vertical, 9), "no such leaf");
    }

    #[test]
    fn removing_a_leaf_promotes_its_sibling() {
        let mut t = tree(4);
        assert!(t.remove(1)); // the 2 under the 0
        assert_eq!(t.leaves(), vec![&0, &1, &3]);
        // The promoted 0 is at depth 1 again, so it can split once more.
        assert!(t.split(0, SplitDir::Horizontal, 5));
        assert_eq!(t.leaves(), vec![&0, &5, &1, &3]);
        let mut single = PaneTree::Leaf(0);
        assert!(
            !single.remove(0),
            "a lone leaf has nothing to collapse into"
        );
    }

    #[test]
    fn geometry_tiles_the_body_exactly_for_one_to_four_panes() {
        let body = Rect::new(0, 1, 81, 23);
        let area_of = |r: &Rect| u32::from(r.width) * u32::from(r.height);
        for n in 2..=4 {
            let (panes, rules) = tree(n).layout(body);
            assert_eq!(panes.len(), n);
            assert_eq!(rules.len(), n - 1);
            let all: Vec<Rect> = panes.iter().chain(&rules).copied().collect();
            // No two rects overlap and together they cover the body.
            for (i, a) in all.iter().enumerate() {
                assert_eq!(a.intersection(body), *a, "{a:?} leaves the body");
                for b in &all[i + 1..] {
                    assert!(a.intersection(*b).is_empty(), "{a:?} overlaps {b:?}");
                }
            }
            assert_eq!(
                all.iter().map(area_of).sum::<u32>(),
                area_of(&body),
                "n={n}"
            );
        }
        let (panes, rules) = tree(2).layout(body);
        // 81 columns: 40 + rule + 40.
        assert_eq!(panes[0], Rect::new(0, 1, 40, 23));
        assert_eq!(rules[0], Rect::new(40, 1, 1, 23));
        assert_eq!(panes[1], Rect::new(41, 1, 40, 23));
        // One pane: the body itself.
        assert_eq!(PaneTree::Leaf(0).layout(body), (vec![body], vec![]));
    }

    #[test]
    fn tiny_bodies_never_underflow_and_grids_keep_the_vt100_floor() {
        for (w, h) in [(0, 0), (1, 1), (2, 2), (3, 1)] {
            let (panes, _) = tree(4).layout(Rect::new(0, 0, w, h));
            for p in panes {
                let (rows, cols) = crate::session::render::grid_of(p);
                assert!(rows >= 2 && cols >= 2, "{w}x{h}: {p:?}");
            }
        }
    }

    #[test]
    fn a_saved_layout_round_trips_through_json() {
        let t = tree(3).map(&mut |i| format!("host-{i}"));
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<PaneTree<String>>(&json).unwrap(), t);
    }
}
