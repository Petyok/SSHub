//! Network map tab (issue #79, variant D): hops as a left-to-right tree, hosts
//! as status glyphs behind their last hop, a path ribbon and facts below.
//!
//! Layout and copy follow `docs/network-map/mockups/D*.txt`. Every colour is an
//! existing theme role. Status is honest about its source: `●`/`●` come from the
//! local ICMP ping (reachability from this machine, not through the jump), so a
//! missing reply reads as unknown (`○`), never as a down hop.

use std::collections::{BTreeMap, HashMap, HashSet};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use unicode_width::UnicodeWidthStr;

use crate::app::network_map::{HostTunnel, Lens, MapSel, MapView, Tint, TunnelState, REVEAL_STEP};
use crate::app::test_connect::{TestOwner, TestRun};
use crate::app::{App, ClickHit};
use crate::map::{Chain, Hop, HopKey, HopKind, JumpSource, Map, Row, PROXY_CMD_LABEL};
use crate::theme::catalog::{ColorRole, StyleRole};
use crate::theme::model::ResolvedTheme;

/// Most glyphs one group shows before `+n`.
/// Glyphs one group shows before `+n` (6 under 120 columns).
fn max_glyphs(width: u16) -> usize {
    if width >= 120 {
        10
    } else {
        6
    }
}

/// Terminal cells a string takes.
fn cells(s: &str) -> u16 {
    s.width().min(u16::MAX as usize) as u16
}

/// Write `s` at (`x`, `y`), clipped at `max_x` (exclusive); returns the x after it.
fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style, max_x: u16) -> u16 {
    if x >= max_x {
        return x;
    }
    let (nx, _) = buf.set_stringn(x, y, s, (max_x - x) as usize, style);
    nx
}

#[derive(Clone, Copy)]
struct Styles {
    text: Style,
    bright: Style,
    muted: Style,
    dim: Style,
    accent: Style,
    ok: Style,
    warn: Style,
    err: Style,
    info: Style,
    unknown: Style,
    selected: Style,
}

impl Styles {
    fn of(theme: &ResolvedTheme) -> Self {
        let fg = |r| Style::default().fg(theme.color(r));
        Self {
            text: theme.style(StyleRole::TextPrimary),
            bright: theme.style(StyleRole::TextBright),
            muted: theme.style(StyleRole::TextMuted),
            dim: theme.style(StyleRole::TextDim),
            accent: theme.style(StyleRole::FocusIndicator),
            ok: fg(ColorRole::StatusSuccess),
            warn: fg(ColorRole::StatusWarning),
            err: fg(ColorRole::StatusError),
            info: fg(ColorRole::StatusInfo),
            unknown: fg(ColorRole::StatusUnknown),
            selected: theme.style(StyleRole::SelectionActive),
        }
    }
}

/// The test run that belongs to what the cursor is on.
fn map_test(app: &App, sel: Option<MapSel>) -> Option<&TestRun> {
    let sel = sel?;
    app.test_run
        .as_ref()
        .filter(|r| matches!(&r.owner, TestOwner::Map(s) if *s == sel))
}

/// Put each step's outcome on the matching ribbon card.
fn apply_test(cards: &mut [Card], run: &TestRun, st: &Styles) {
    if cards.len() != run.states.len() {
        return;
    }
    for (c, state) in cards.iter_mut().zip(&run.states) {
        let (glyph, text) = crate::app::hop_editor::step_text(state);
        c.line3 = format!("{glyph} {text}");
        if state.is_failure() {
            c.border = st.err;
        }
    }
}

/// Everything a frame of the map reads.
struct Ctx<'a> {
    app: &'a App,
    view: &'a MapView,
    st: &'a Styles,
    /// Tunnels by host index (tunnel lens, facts).
    tunnels: HashMap<usize, Vec<HostTunnel>>,
    max_glyphs: usize,
    test: Option<&'a TestRun>,
}

impl Ctx<'_> {
    /// The glyph and style of one host under the filter, lens and row tint.
    fn glyph(&self, host: usize, tint: Tint) -> (char, Style) {
        let st = self.st;
        let base = host_status(self.app, st, host);
        let dim = ('\u{b7}', st.dim);
        if let Some(over) = self.test_glyph(host) {
            return over;
        }
        if self
            .view
            .matched
            .as_ref()
            .is_some_and(|m| !m.contains(&host))
        {
            return dim;
        }
        match &self.view.lens {
            Lens::Blast(b) => {
                return if b.hosts.contains(&host) {
                    (base.glyph, st.warn)
                } else {
                    (base.glyph, st.dim)
                };
            }
            Lens::Tunnel => {
                return match self.tunnels.get(&host) {
                    Some(t) => ('\u{21c4}', tunnel_style(st, t[0].state)),
                    None => dim,
                };
            }
            Lens::None => {}
        }
        if tint == Tint::Dim {
            return (base.glyph, st.dim);
        }
        (base.glyph, base.style)
    }
}

impl Ctx<'_> {
    /// `»` while a test connect runs on this host, `×` after one failed on it.
    /// A hop is only ever shown down because a test of it failed.
    fn test_glyph(&self, host: usize) -> Option<(char, Style)> {
        use crate::ssh::test_connect::StepState;
        let run = self.test?;
        let name = &self.view.map.hosts[host].name;
        run.steps
            .iter()
            .zip(&run.states)
            .find(|(step, _)| step.host.as_deref() == Some(name.as_str()))
            .and_then(|(_, state)| match state {
                StepState::Running => Some(('\u{bb}', self.st.warn)),
                StepState::Failed { .. } => Some(('\u{d7}', self.st.err)),
                _ => None,
            })
    }
}

fn tunnel_style(st: &Styles, state: TunnelState) -> Style {
    match state {
        TunnelState::Running => st.ok,
        TunnelState::Reconnecting => st.warn,
        TunnelState::Stopped => st.muted,
    }
}

fn tunnel_letter(kind: crate::store::TunnelType) -> char {
    match kind {
        crate::store::TunnelType::Local => 'L',
        crate::store::TunnelType::Remote => 'R',
        crate::store::TunnelType::Dynamic => 'D',
    }
}

/// Status of one host as drawn: glyph, style and latency when known.
struct Status {
    glyph: char,
    style: Style,
    ms: Option<u32>,
}

fn host_status(app: &App, st: &Styles, host: usize) -> Status {
    let Some(view) = app.map_view.as_ref() else {
        return Status {
            glyph: '○',
            style: st.unknown,
            ms: None,
        };
    };
    let name = &view.map.hosts[host].name;
    if app
        .sessions
        .iter()
        .any(|s| !s.local_shell && s.config.host_name == *name)
    {
        return Status {
            glyph: '▶',
            style: st.accent.add_modifier(Modifier::BOLD),
            ms: None,
        };
    }
    let samples = app.ping_data.get(name).map(Vec::as_slice);
    match crate::ping::classify_ping(samples) {
        crate::ping::PingClass::Online => Status {
            glyph: '●',
            style: st.ok,
            ms: samples.and_then(|s| s.last().copied()),
        },
        crate::ping::PingClass::Slow => Status {
            glyph: '●',
            style: st.warn,
            ms: samples.and_then(|s| s.last().copied()),
        },
        // A host reached directly is pinged the way the hosts tab shows it, so a
        // missing reply reads as down. Behind a jump (or as a hop) no reply is
        // not a verdict: ICMP from here says nothing about the path through
        // the jump, and only a failed test connect marks those down.
        crate::ping::PingClass::Unreachable if view.map.reached_directly(host) => Status {
            glyph: '\u{d7}',
            style: st.err,
            ms: None,
        },
        _ => Status {
            glyph: '○',
            style: st.unknown,
            ms: None,
        },
    }
}

/// `ms` text for a latency, empty when unknown.
fn ms_text(ms: Option<u32>) -> String {
    ms.map(|m| format!("{m}ms")).unwrap_or_default()
}

/// Layout constants: columns are offsets inside the map panel.
#[derive(Clone, Copy)]
struct Cols {
    hop1: u16,
    hop2: u16,
    cargo: u16,
    /// Hop-column step: names are clipped to fit it.
    step: u16,
    latency: bool,
    /// Width of the name column and of the latency text in hop nodes, set
    /// per frame from what is visible so the latencies line up.
    name_w: usize,
    ms_w: usize,
}

impl Cols {
    fn for_width(w: u16) -> Self {
        if w >= 120 {
            Self {
                hop1: 11,
                hop2: 38,
                cargo: 65,
                step: 27,
                latency: true,
                name_w: 0,
                ms_w: 0,
            }
        } else {
            Self {
                hop1: 9,
                hop2: 30,
                cargo: 51,
                step: 21,
                latency: false,
                name_w: 0,
                ms_w: 0,
            }
        }
    }

    fn trunk1(&self) -> u16 {
        self.hop1 - 3
    }

    fn trunk2(&self) -> u16 {
        self.hop2 - 3
    }
}

pub fn render_map(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let st = Styles::of(theme);
    // `area` is the dashboard body: the terminal minus 8 rows of chrome, so the
    // design's "80x24" is a body of 80 columns by 16 rows, and "40x12" is 4.
    if area.width < 40 || area.height < 4 {
        // Nothing is drawn for the pickers either: they must not open unseen.
        if let Some(v) = app.map_view.as_ref() {
            v.outline.set(true);
        }
        let buf = frame.buffer_mut();
        put(
            buf,
            area.x + 1,
            area.y + area.height / 2,
            "map needs 40x12, press 1 for hosts",
            st.muted,
            area.x + area.width,
        );
        return;
    }
    let Some(view) = app.map_view.as_ref() else {
        put(
            frame.buffer_mut(),
            area.x + 1,
            area.y + 1,
            app.map_error
                .as_deref()
                .unwrap_or("the map could not be built"),
            st.err,
            area.x + area.width,
        );
        return;
    };

    let ctx = Ctx {
        app,
        view,
        st: &st,
        tunnels: app.map_tunnels(),
        max_glyphs: max_glyphs(area.width),
        test: map_test(app, view.selection()),
    };
    view.outline.set(area.width < 80 || area.height < 16);
    if view.outline.get() {
        draw_outline(frame.buffer_mut(), area, &ctx);
        return;
    }

    // The mockups leave one column on each side of the panels on wide screens.
    let margin = u16::from(area.width >= 132);
    let area = Rect::new(
        area.x + margin,
        area.y,
        area.width.saturating_sub(2 * margin),
        area.height,
    );

    // The map owns the whole body when there is nothing to put under it (no
    // jump edges: D07), when direct-only is expanded (D14), or when zoomed.
    let whole = view.zoomed || view.direct_open || view.edgeless();
    let full = area.width >= 118 && area.height >= 30;
    let (ribbon_h, bottom_h) = if whole {
        (0, 0)
    } else if full {
        (10, 6)
    } else if area.height >= 16 {
        (4, 0)
    } else {
        (0, 0)
    };
    let map_h = area.height - ribbon_h - bottom_h;
    let map_area = Rect::new(area.x, area.y, area.width, map_h);
    let ribbon_area = Rect::new(area.x, area.y + map_h, area.width, ribbon_h);
    let bottom_area = Rect::new(area.x, area.y + map_h + ribbon_h, area.width, bottom_h);

    let buf = frame.buffer_mut();
    let sel = view.selection();
    let title = map_title(&ctx);
    let inner = panel(buf, map_area, &title, st.accent, st.bright);
    if view.edgeless() && !view.direct_open {
        draw_empty(buf, inner, &ctx);
    } else {
        draw_map(buf, inner, &ctx);
    }
    if let Some(p) = &view.picker {
        draw_tag_picker(buf, inner, p, &st);
    }
    if let Some(p) = &view.use_picker {
        draw_use_picker(buf, inner, view, p, &st);
    }

    if ribbon_h > 0 {
        // The full ribbon is titled `path ribbon`; the one-line one (D09) names
        // the host and its group.
        let ribbon_title = if full {
            format!("path ribbon \u{2500} {}", sel_title(view, sel))
        } else {
            let group = match sel {
                Some(MapSel::Host(h)) => Some(view.map.hosts[h].group.clone()),
                Some(MapSel::Hop(n)) => match view.map.nodes[n].key {
                    HopKey::Saved(h) => Some(view.map.hosts[h].group.clone()),
                    _ => None,
                },
                _ => None,
            };
            match group {
                Some(g) => format!("{} \u{2500} {g}", sel_title(view, sel)),
                None => sel_title(view, sel),
            }
        };
        let inner = panel(buf, ribbon_area, &ribbon_title, st.dim, st.bright);
        if full {
            draw_ribbon_full(buf, inner, &ctx, sel);
        } else {
            draw_ribbon_line(buf, inner, app, view, sel, &st);
        }
    }
    if bottom_h > 0 {
        let left_w = (bottom_area.width / 2).min(79);
        let legend = Rect::new(bottom_area.x, bottom_area.y, left_w, bottom_h);
        let facts = Rect::new(
            bottom_area.x + left_w + 1,
            bottom_area.y,
            bottom_area.width.saturating_sub(left_w + 1),
            bottom_h,
        );
        let inner = panel(buf, legend, "legend", st.dim, st.bright);
        draw_legend(buf, inner, &st);
        let facts_title = facts_title(&ctx, sel);
        let inner = panel(buf, facts, &facts_title, st.dim, st.bright);
        draw_facts(buf, inner, &ctx, sel);
    }
}

/// Below 80x24 the tree is an outline: one hop per row, depth as indent, the
/// selected path on the last line (cut from the left when it is too long).
fn draw_outline(buf: &mut Buffer, area: Rect, ctx: &Ctx) {
    let (view, st) = (ctx.view, ctx.st);
    let right = area.x + area.width;
    let map = &view.map;
    let list_h = area.height.saturating_sub(2) as usize;
    let title = crate::tui::text::ellipsize(&map_title(ctx), area.width as usize - 1);
    put(buf, area.x, area.y, &title, st.bright, right);
    let first = (view.row + 1).saturating_sub(list_h);
    for (k, &row) in view.rows.iter().enumerate().skip(first).take(list_h) {
        let y = area.y + 1 + (k - first) as u16;
        let (indent, text, n) = match row {
            Row::Node(n) => {
                let node = &map.nodes[n];
                let label = match &node.key {
                    HopKey::Saved(h) => {
                        let g = host_status(ctx.app, st, *h).glyph;
                        format!("{g} {}", map.hosts[*h].name)
                    }
                    other => map.key_label(other),
                };
                (node.depth + 1, label, node.sigma)
            }
            Row::Cluster(c) => (
                0,
                format!(
                    "{} {}",
                    if view.open_clusters.contains(&map.clusters[c].name) {
                        '\u{25be}'
                    } else {
                        '\u{25b8}'
                    },
                    map.clusters[c].name
                ),
                map.clusters[c].sigma,
            ),
            Row::More(c) => (1, "\u{2026} more hops".into(), map.clusters[c].roots.len()),
            Row::Cycle(i) => (
                0,
                format!("\u{21bb} loop {}", map.hosts[map.cycles[i].path[0]].name),
                map.cycles[i].hosts.len(),
            ),
            Row::Direct => (0, "\u{22ef} direct-only".into(), map.direct.len()),
            Row::DirectGroup(g) => (
                1,
                map.direct_groups[g].0.clone(),
                map.direct_groups[g].1.len(),
            ),
        };
        let style = if k == view.row {
            st.selected
        } else {
            match view.tint(k) {
                Tint::Dim => st.dim,
                Tint::Warn => st.warn,
                Tint::Normal => st.text,
            }
        };
        let tree = if indent == 0 {
            String::new()
        } else {
            format!("{}\u{251c} ", "  ".repeat(indent - 1))
        };
        let line = format!("{tree}{text}");
        let room = (area.width as usize).saturating_sub(6);
        put(
            buf,
            area.x,
            y,
            &crate::tui::text::ellipsize(&line, room),
            style,
            right,
        );
        put(
            buf,
            right.saturating_sub(5),
            y,
            &format!("{n:>4}"),
            st.bright,
            right,
        );
    }
    // Selected path, cut from the left.
    let (cards, _) = cards_for(view, view.selection(), st);
    let mut path = String::from("you");
    for c in &cards {
        path.push_str(" \u{203a} ");
        path.push_str(&c.name);
    }
    let w = area.width as usize;
    let shown = if path.chars().count() > w {
        let tail: String = path
            .chars()
            .rev()
            .take(w.saturating_sub(1))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("\u{2026}{tail}")
    } else {
        path
    };
    put(
        buf,
        area.x,
        area.y + area.height - 1,
        &shown,
        st.accent,
        right,
    );
}

/// A bordered panel with a title; returns the inner rect.
fn panel(buf: &mut Buffer, area: Rect, title: &str, border: Style, title_style: Style) -> Rect {
    let r = area.x + area.width - 1;
    let b = area.y + area.height - 1;
    for x in area.x + 1..r {
        buf.set_string(x, area.y, "\u{2500}", border);
        buf.set_string(x, b, "\u{2500}", border);
    }
    for y in area.y + 1..b {
        buf.set_string(area.x, y, "\u{2502}", border);
        buf.set_string(r, y, "\u{2502}", border);
    }
    buf.set_string(area.x, area.y, "\u{250c}", border);
    buf.set_string(r, area.y, "\u{2510}", border);
    buf.set_string(area.x, b, "\u{2514}", border);
    buf.set_string(r, b, "\u{2518}", border);
    let t = crate::tui::text::ellipsize(title, area.width.saturating_sub(6) as usize);
    put(buf, area.x + 2, area.y, &format!(" {t} "), title_style, r);
    Rect::new(
        area.x + 1,
        area.y + 1,
        area.width - 2,
        area.height.saturating_sub(2),
    )
}

fn sel_title(view: &MapView, sel: Option<MapSel>) -> String {
    match sel {
        Some(MapSel::Hop(n)) => view.map.key_label(&view.map.nodes[n].key),
        Some(MapSel::Host(h)) => match view.map.chains[h] {
            Chain::Cycle(_) => format!("{} \u{b7} cycle", view.map.hosts[h].name),
            _ => view.map.hosts[h].name.clone(),
        },
        Some(MapSel::Cycle(i)) => format!(
            "{} \u{b7} cycle",
            view.map.hosts[view.map.cycles[i].path[0]].name
        ),
        Some(MapSel::Direct) => "direct-only".into(),
        Some(MapSel::Cluster(c)) => format!("cluster {}", view.map.clusters[c].name),
        Some(MapSel::More(c)) => format!("more hops in {}", view.map.clusters[c].name),
        None => String::new(),
    }
}

/// The map panel title: the active lens, then the filter.
fn map_title(ctx: &Ctx) -> String {
    let view = ctx.view;
    if let Some(run) = ctx.test {
        return format!(
            "map \u{2500} testing path to {} \u{b7} {}",
            run.title,
            run.headline()
        );
    }
    match &view.lens {
        Lens::Blast(b) => {
            return format!(
                "map \u{2500} blast radius \u{b7} {} hosts depend on {}",
                b.hosts.len(),
                view.map.key_label(&b.key)
            )
        }
        Lens::Tunnel => {
            return "map \u{2500} tunnel lens \u{b7} \u{21c4} on hops and hosts that carry a tunnel"
                .into()
        }
        Lens::None => {}
    }
    if view.editing || view.filter_active() {
        return format!("map \u{2500} filter \u{b7} {}", view.query.trim());
    }
    if view.direct_open {
        return format!(
            "map \u{2500} {} hosts \u{b7} {} hop rows \u{b7} direct-only expanded",
            view.map.hosts.len(),
            view.map.nodes.len() + view.map.cycles.len()
        );
    }
    if view.clustered() {
        return format!(
            "map \u{2500} {} hosts \u{b7} {} jump hosts \u{b7} chains up to {} hops \u{b7} clustered",
            thousands(view.map.hosts.len()),
            view.map.hop_hosts.len(),
            view.map.max_depth
        );
    }
    "map \u{2500} hops as a tree \u{b7} hosts as glyphs behind their last hop".into()
}

/// `1,840`.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The map with no jump edge at all (design D07): a dim preview of what a row
/// looks like, then what to do about it.
fn draw_empty(buf: &mut Buffer, inner: Rect, ctx: &Ctx) {
    let (view, st) = (ctx.view, ctx.st);
    let right = inner.x + inner.width;
    let n = view.map.hosts.len();
    let kb = &ctx.app.config.keybinds;
    let k = |a: crate::config::KeyAction| key_label(kb.primary(a));

    let preview = [
        "\u{25c9} you \u{2500}\u{252c}\u{2500}\u{25b8}\u{25cf} bastion \u{2500}\u{252c}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508} group \u{25cf}\u{25cf}\u{25cf}\u{25cf}\u{25cf}\u{25cf}     \u{3a3} 9",
        "       \u{2502}             \u{2514}\u{2500}\u{25b8}\u{25cf} jump \u{2508}\u{2508}\u{2508} group \u{25cf}\u{25cf}\u{25cf}        \u{3a3} 3",
        "       \u{2514}\u{2500}\u{25b8}\u{25cf} bastion \u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508} group \u{25cf}\u{25cf}\u{25cf}\u{25cf}       \u{3a3} 4",
    ];
    // (text, key part, style of the rest)
    let mut lines: Vec<(String, usize, Style)> = Vec::new();
    let plain = |t: String, style: Style| (t, 0usize, style);
    if n == 0 {
        lines.push(plain("No hosts yet.".into(), st.bright));
        lines.push(plain(
            "Add one on the hosts tab (1), then give it a ProxyJump to see a row here.".into(),
            st.text,
        ));
    } else {
        lines.push(plain("No jump edges yet.".into(), st.bright));
        lines.push(plain(
            if n == 1 {
                "The only host is reached directly, so there is no hop to hang it behind."
                    .to_string()
            } else {
                format!(
                    "All {n} hosts are reached directly, so there is no hop to hang them behind."
                )
            },
            st.text,
        ));
    }
    lines.push(plain(String::new(), st.text));
    lines.push(plain(
        "A row appears when a host, its group, or ~/.ssh/config sets a ProxyJump.".into(),
        st.muted,
    ));
    lines.push(plain("Try one:".into(), st.muted));
    lines.push(plain(String::new(), st.text));
    let action = |key: String, text: String| {
        let at = 2 + key.chars().count();
        (format!("  {key}  {text}"), at, st.text)
    };
    lines.push(action(
        k(crate::config::KeyAction::MapUseAsJump),
        "on a host: route other hosts through it (fastest first edge)".into(),
    ));
    lines.push(action(
        k(crate::config::KeyAction::Edit),
        "edit a host \u{2192} ProxyJump \u{2192} a add hop".into(),
    ));
    lines.push(action(
        k(crate::config::KeyAction::GroupsManage),
        "give a group a default ProxyJump".into(),
    ));
    lines.push(action(
        k(crate::config::KeyAction::MapFoldAll),
        if n == 1 {
            "show the direct host as a per-group health board".to_string()
        } else {
            format!("show the {n} direct hosts as a per-group health board")
        },
    ));
    lines.push(plain(String::new(), st.text));
    lines.push(plain(
        "Groups and tags never create edges.".into(),
        st.muted,
    ));

    // Fit the block: the preview first goes when there is no room, then the
    // blank lines, then the tail.
    let h = inner.height as usize;
    let with_preview = h >= lines.len() + preview.len() + 2;
    if !with_preview {
        while lines.len() > h {
            match lines.iter().rposition(|(t, _, _)| t.is_empty()) {
                Some(i) => {
                    lines.remove(i);
                }
                None => {
                    lines.pop();
                }
            }
        }
    }
    let used = lines.len() + if with_preview { preview.len() + 2 } else { 0 };
    let top = inner.y + (h.saturating_sub(used) / 2).min(6) as u16;
    let left = inner.x + (inner.width.saturating_sub(80) / 2).max(1);
    let mut y = top;
    if with_preview {
        for p in &preview {
            put(buf, left, y, p, st.dim, right);
            y += 1;
        }
        y += 2;
    }
    for (text, key_len, style) in &lines {
        if y >= inner.y + inner.height {
            break;
        }
        if *key_len > 0 {
            let (key, rest) = text.split_at(
                text.char_indices()
                    .nth(*key_len)
                    .map_or(text.len(), |(i, _)| i),
            );
            let x = put(buf, left, y, key, st.accent, right);
            put(buf, x, y, rest, *style, right);
        } else {
            put(buf, left, y, text, *style, right);
        }
        y += 1;
    }
}

/// `#` tag picker, centred over the map.
fn draw_tag_picker(
    buf: &mut Buffer,
    inner: Rect,
    p: &crate::app::network_map::TagPicker,
    st: &Styles,
) {
    let visible = p.visible();
    let rows = visible
        .len()
        .min(inner.height.saturating_sub(5) as usize)
        .max(1);
    let w = 36.min(inner.width.saturating_sub(2));
    let area = Rect::new(
        inner.x + (inner.width - w) / 2,
        inner.y + 1,
        w,
        rows as u16 + 3,
    );
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            buf.set_string(x, y, " ", st.text);
        }
    }
    let body = panel(buf, area, "tag filter", st.accent, st.bright);
    let max_x = body.x + body.width;
    put(
        buf,
        body.x,
        body.y,
        &format!(" \u{203a} {}\u{258f}", p.query),
        st.text,
        max_x,
    );
    if visible.is_empty() {
        put(
            buf,
            body.x,
            body.y + 1,
            "   no matching tags",
            st.dim,
            max_x,
        );
        return;
    }
    let first = p.selected.saturating_sub(rows - 1);
    for (i, &t) in visible.iter().skip(first).take(rows).enumerate() {
        let style = if first + i == p.selected {
            st.selected
        } else {
            st.text
        };
        let text = format!(" #{}", p.items[t]);
        put(
            buf,
            body.x,
            body.y + 1 + i as u16,
            &format!("{text:<w$}", w = body.width as usize),
            style,
            max_x,
        );
    }
}

/// `u` picker: which host takes the hop as its jump.
fn draw_use_picker(
    buf: &mut Buffer,
    inner: Rect,
    view: &MapView,
    p: &crate::app::network_map::UsePicker,
    st: &Styles,
) {
    let visible = p.visible(&view.map);
    let rows = visible
        .len()
        .min(inner.height.saturating_sub(6) as usize)
        .max(1);
    let w = 56.min(inner.width.saturating_sub(2));
    let area = Rect::new(
        inner.x + (inner.width - w) / 2,
        inner.y + 1,
        w,
        rows as u16 + 4,
    );
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            buf.set_string(x, y, " ", st.text);
        }
    }
    let title = match p.purpose {
        crate::app::network_map::UsePurpose::Apply => format!(
            "use {} as jump \u{b7} Tab: {}",
            p.token,
            if p.append { "append" } else { "replace" }
        ),
        crate::app::network_map::UsePurpose::Edit => "edit which host".to_string(),
        crate::app::network_map::UsePurpose::PickHop => "use which host as the jump".to_string(),
    };
    let body = panel(buf, area, &title, st.accent, st.bright);
    let max_x = body.x + body.width;
    put(
        buf,
        body.x,
        body.y,
        &format!(" \u{203a} {}\u{258f}", p.query),
        st.text,
        max_x,
    );
    let first = p.selected.saturating_sub(rows - 1);
    for (i, &(h, loops)) in visible.iter().skip(first).take(rows).enumerate() {
        let name = &view.map.hosts[h].name;
        let style = if first + i == p.selected {
            st.selected
        } else if loops {
            st.dim
        } else {
            st.text
        };
        let text = format!(
            "   {name}{}",
            if loops { "   \u{21bb} would loop" } else { "" }
        );
        put(
            buf,
            body.x,
            body.y + 1 + i as u16,
            &format!("{text:<w$}", w = body.width as usize),
            style,
            max_x,
        );
    }
}

// ---------------------------------------------------------------- the tree

/// What one hop node shows.
struct NodeLook {
    text: String,
    style: Style,
    /// Edge fill: `\u{2500}` for a real hop, `\u{2504}` for a ProxyCommand.
    dashed: bool,
}

fn node_look(ctx: &Ctx, cols: &Cols, node: usize, shared: &HashSet<HopKey>) -> NodeLook {
    let (app, view, st) = (ctx.app, ctx.view, ctx.st);
    let n = &view.map.nodes[node];
    // Room between the node column and the next trunk, minus the glyph.
    let room = cols.hop2.saturating_sub(cols.hop1 + 4) as usize;
    match &n.key {
        HopKey::Saved(h) => {
            let s = host_status(app, st, *h);
            let glyph = ctx.test_glyph(*h).map_or(s.glyph, |(g, _)| g);
            let name =
                crate::tui::text::ellipsize(&view.map.hosts[*h].name, room.saturating_sub(2));
            let mut text = if cols.latency && cols.ms_w > 0 {
                // Names padded to one column and latencies right-aligned, so
                // the ms of every visible hop line up.
                format!(
                    "{glyph} {name:<nw$}{ms:>mw$}",
                    nw = cols.name_w,
                    ms = ms_text(s.ms),
                    mw = cols.ms_w
                )
            } else {
                format!("{glyph} {name}")
            };
            if shared.contains(&n.key) {
                text.push_str(" \u{25c7}");
            }
            if matches!(view.lens, Lens::Tunnel) {
                for t in ctx.tunnels.get(h).into_iter().flatten() {
                    text.push_str(&format!(" \u{21c4}{}", tunnel_letter(t.kind)));
                }
            }
            NodeLook {
                text,
                style: st.text,
                dashed: false,
            }
        }
        HopKey::Raw(r) => NodeLook {
            text: format!(
                "\u{2506}{}\u{2506}",
                crate::tui::text::ellipsize(&r.label(), room.saturating_sub(2))
            ),
            style: st.muted,
            dashed: false,
        },
        HopKey::ProxyCmd(_) => NodeLook {
            text: format!("\u{2039}{PROXY_CMD_LABEL}\u{203a}"),
            style: st.info,
            dashed: true,
        },
        HopKey::Invalid(t) => NodeLook {
            text: format!(
                "\u{2506}{}\u{2506}",
                crate::tui::text::ellipsize(t, room.saturating_sub(2))
            ),
            style: st.err,
            dashed: false,
        },
    }
}

/// Group strip for `hosts` by group: `group \u{25cf}\u{25cf}\u{25cb}   group2 \u{25cf}`.
/// Returns the x after it and the x range of the selected host's glyph.
#[allow(clippy::too_many_arguments)]
/// Record host `h`'s glyph at (`x`, `y`) as a click target.
fn record_glyph(ctx: &Ctx, x: u16, y: u16, row: usize, h: usize) {
    let view = ctx.view;
    let Some(&r) = view.rows.get(row) else {
        return;
    };
    if let Some(k) = view.map.row_hosts(r).iter().position(|&v| v == h) {
        ctx.app.record_click(
            Rect::new(x, y, 1, 1),
            ClickHit::MapCell {
                row,
                pos: view.hop_count(row) + k,
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_strip(
    buf: &mut Buffer,
    mut x: u16,
    y: u16,
    max_x: u16,
    ctx: &Ctx,
    tint: Tint,
    groups: &[(String, Vec<usize>)],
    selected: Option<usize>,
    row: usize,
    cap: usize,
) -> u16 {
    let st = ctx.st;
    for (gi, (group, hosts)) in groups.iter().enumerate() {
        if gi > 0 {
            x += 3;
        }
        if x >= max_x {
            break;
        }
        if !group.is_empty() {
            // The group label follows the lens: dim when the row is dimmed.
            let label = if tint == Tint::Dim { st.dim } else { st.muted };
            x = put(buf, x, y, group, label, max_x);
            x = put(buf, x, y, " ", label, max_x);
        }
        // Keep the selected glyph on screen when the group is longer than the cap.
        let start = selected
            .and_then(|s| hosts.iter().position(|&h| h == s))
            .map_or(0, |p| (p + 1).saturating_sub(cap));
        let shown = hosts.iter().skip(start).take(cap);
        for &h in shown {
            let (glyph, gstyle) = ctx.glyph(h, tint);
            let style = if selected == Some(h) {
                st.selected
            } else {
                gstyle
            };
            if x < max_x {
                record_glyph(ctx, x, y, row, h);
            }
            x = put(buf, x, y, &glyph.to_string(), style, max_x);
        }
        let hidden = hosts.len().saturating_sub(cap);
        if hidden > 0 {
            x = put(buf, x, y, &format!("+{hidden}"), st.muted, max_x);
        }
    }
    x
}

/// `1 host`, `2 hosts`.
fn hosts_n(n: usize) -> String {
    format!("{n} host{}", if n == 1 { "" } else { "s" })
}

/// The folded hops right above `n` (nearest first): they have no row, `n`'s row
/// speaks for them.
fn folded_chain(map: &Map, n: usize) -> Vec<usize> {
    let mut chain = Vec::new();
    let mut at = map.nodes[n].parent;
    while let Some(p) = at.filter(|&p| map.folded[p]) {
        chain.push(p);
        at = map.nodes[p].parent;
    }
    chain
}

/// The root hop whose own row `n` took over (it is folded and `n` is where its
/// chain ends): `n`'s row then starts at the left trunk.
fn folded_top(map: &Map, n: usize) -> Option<usize> {
    folded_chain(map, n)
        .last()
        .copied()
        .filter(|&t| map.nodes[t].depth == 0)
}

/// Every host behind the hops of cluster `c`, at any depth.
fn cluster_hosts(map: &Map, c: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack: Vec<usize> = map.clusters[c].roots.clone();
    while let Some(n) = stack.pop() {
        out.extend(
            map.nodes[n]
                .cargo
                .iter()
                .flat_map(|(_, h)| h.iter().copied()),
        );
        stack.extend(map.nodes[n].children.iter().copied());
    }
    out.sort_unstable();
    out
}

fn draw_map(buf: &mut Buffer, inner: Rect, ctx: &Ctx) {
    let (view, st) = (ctx.view, ctx.st);
    if inner.height < 4 {
        return;
    }
    let mut cols = Cols::for_width(inner.width + 2);
    // Hops under a cluster sit one step in, so the later columns move over.
    if view.clustered() {
        cols.hop2 += 8;
        cols.cargo += 8;
    }
    let right = inner.x + inner.width;
    let sigma_x = right.saturating_sub(5);
    let cx = |c: u16| inner.x + c;

    // Header: the columns, or the filter line while a filter is set.
    let hy = inner.y;
    if view.editing || view.filter_active() {
        put(
            buf,
            cx(1),
            hy,
            &format!(
                "/ {}{}",
                view.query,
                if view.editing { "\u{258f}" } else { "" }
            ),
            st.text,
            cx(cols.cargo),
        );
        // A bare `tag:` has no value yet and filters nothing: every host stays.
        let n = view
            .matched
            .as_ref()
            .map_or(view.map.hosts.len(), HashSet::len);
        put(
            buf,
            cx(cols.cargo),
            hy,
            &format!(
                "{} {} \u{b7} rows without a match are hidden, hops above kept",
                hosts_n(n),
                if n == 1 { "matches" } else { "match" }
            ),
            st.muted,
            sigma_x,
        );
    } else if !view.edgeless() {
        put(buf, cx(1), hy, "you", st.muted, right);
        let clustered = view.clustered();
        put(
            buf,
            cx(cols.hop1),
            hy,
            if clustered {
                "hop 1 \u{b7} clusters"
            } else {
                "hop 1"
            },
            st.muted,
            right,
        );
        put(buf, cx(cols.hop2), hy, "hop 2", st.muted, right);
        put(
            buf,
            cx(cols.cargo),
            hy,
            if clustered {
                "hosts behind \u{b7} one glyph each"
            } else {
                "hosts behind this hop \u{b7} one glyph each"
            },
            st.muted,
            sigma_x,
        );
    }
    put(buf, sigma_x + 3, hy, "\u{3a3}", st.muted, right);

    let map = &view.map;
    // Align the names and latencies of the hop nodes on screen.
    {
        let (mut name_w, mut ms_w) = (0usize, 0usize);
        for r in &view.rows {
            if let Row::Node(n) = r {
                if let HopKey::Saved(h) = map.nodes[*n].key {
                    name_w = name_w.max(map.hosts[h].name.chars().count());
                    ms_w = ms_w.max(ms_text(host_status(ctx.app, st, h).ms).len());
                }
            }
        }
        cols.name_w = (name_w + 2).min(cols.step.saturating_sub(12) as usize);
        cols.ms_w = ms_w;
    }
    let sel = view.selection();
    let sel_host = match sel {
        Some(MapSel::Host(h)) => Some(h),
        _ => None,
    };
    let path_nodes: HashSet<usize> = match sel {
        Some(MapSel::Hop(n)) => map.node_path(n).into_iter().collect(),
        Some(MapSel::Host(h)) => map.host_node[h]
            .map(|n| map.node_path(n).into_iter().collect())
            .unwrap_or_default(),
        _ => HashSet::new(),
    };
    let mut counts: HashMap<&HopKey, usize> = HashMap::new();
    for n in &map.nodes {
        *counts.entry(&n.key).or_default() += 1;
    }
    let shared: HashSet<HopKey> = counts
        .into_iter()
        .filter(|(_, c)| *c > 1)
        .map(|(k, _)| k.clone())
        .collect();

    // Top-level items own the left trunk: root hops, clusters and loops. A root
    // hop inside a cluster hangs off the cluster's trunk instead.
    let in_cluster: HashSet<usize> = map
        .clusters
        .iter()
        .flat_map(|c| c.roots.iter().copied())
        .collect();
    let top: Vec<usize> = view
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| match r {
            Row::Node(n) => {
                (map.nodes[*n].depth == 0 || folded_top(map, *n).is_some())
                    && !in_cluster.contains(n)
            }
            Row::Cycle(_) | Row::Cluster(_) => true,
            Row::Direct | Row::More(_) | Row::DirectGroup(_) => false,
        })
        .map(|(i, _)| i)
        .collect();
    let top_set: HashSet<usize> = top.iter().copied().collect();
    let clustered = view.clustered();
    let last_top = top.last().copied();

    // One blank row between top-level items when everything fits. The
    // collapsed direct-only item is two rows: the summary and its hint.
    let visible = inner.height.saturating_sub(2) as usize;
    let height = |r: &Row| usize::from(matches!(r, Row::Direct) && !view.direct_open) + 1;
    let blanks = top.len().saturating_sub(1) + usize::from(view.rows.contains(&Row::Direct));
    let spacious = view.rows.iter().map(height).sum::<usize>() + blanks <= visible;
    let mut ys: Vec<usize> = Vec::with_capacity(view.rows.len());
    let mut y = 0usize;
    for (i, r) in view.rows.iter().enumerate() {
        let starts_item = (top_set.contains(&i) && !clustered) || matches!(r, Row::Direct);
        if spacious && starts_item && i > 0 {
            y += 1;
        }
        ys.push(y);
        y += height(r);
    }
    let sel_y = ys.get(view.row).copied().unwrap_or(0);
    let offset = (sel_y + 1).saturating_sub(visible);
    let row_y = |i: usize| -> Option<u16> {
        let v = ys[i].checked_sub(offset)?;
        (v < visible).then(|| inner.y + 2 + v as u16)
    };

    // Vertical trunks first, so a connector glyph drawn later wins.
    if let (Some(&first), Some(&last)) = (top.first(), top.last()) {
        let lo = ys[first].saturating_sub(offset);
        let hi = ys[last].saturating_sub(offset);
        for v in lo..=hi.min(visible.saturating_sub(1)) {
            put(
                buf,
                cx(cols.trunk1()),
                inner.y + 2 + v as u16,
                "\u{2502}",
                st.dim,
                right,
            );
        }
    }

    for (i, &row) in view.rows.iter().enumerate() {
        let Some(y) = row_y(i) else {
            continue;
        };
        let row_selected = i == view.row;
        // The whole row selects its own hop; glyphs record themselves on top.
        ctx.app.record_click(
            Rect::new(inner.x, y, inner.width, 1),
            ClickHit::MapCell {
                row: i,
                pos: view.hop_count(i) - 1,
            },
        );
        match row {
            Row::Node(n) => {
                let node = &map.nodes[n];
                let look = node_look(ctx, &cols, n, &shared);
                let tint = view.tint(i);
                let on_path = path_nodes.contains(&n) && tint == Tint::Normal;
                let edge_style = match tint {
                    Tint::Dim => st.dim,
                    Tint::Warn => st.warn,
                    Tint::Normal if on_path => st.accent,
                    Tint::Normal => st.dim,
                };
                let fill = if look.dashed { "\u{2504}" } else { "\u{2500}" };
                let under = node.depth == 0 && in_cluster.contains(&n);
                let merged = folded_top(map, n);
                let is_top = node.depth == 0 || merged.is_some();
                let col = if under {
                    cols.hop1 + 4
                } else if is_top {
                    cols.hop1
                } else {
                    cols.hop2
                };
                // Connector into the node.
                if under {
                    put(buf, cx(cols.hop1 + 1), y, "\u{251c}", edge_style, right);
                    put(buf, cx(cols.hop1 + 2), y, fill, edge_style, right);
                    put(buf, cx(cols.hop1 + 3), y, "\u{25b8}", edge_style, right);
                } else if is_top {
                    let first = top.first() == Some(&i);
                    let last = last_top == Some(i);
                    let joint = match (first, last) {
                        (true, true) => "\u{2500}",
                        (true, false) => "\u{252c}",
                        (false, true) => "\u{2514}",
                        _ => "\u{251c}",
                    };
                    if first {
                        put(
                            buf,
                            cx(1),
                            y,
                            "\u{25c9} you",
                            st.accent.add_modifier(Modifier::BOLD),
                            right,
                        );
                        put(buf, cx(6), y, "\u{2500}\u{2500}", edge_style, right);
                    }
                    put(buf, cx(cols.trunk1()), y, joint, edge_style, right);
                    put(buf, cx(cols.trunk1() + 1), y, fill, edge_style, right);
                    put(buf, cx(cols.trunk1() + 2), y, "\u{25b8}", edge_style, right);
                } else {
                    let last_child = node
                        .parent
                        .map(|p| map.nodes[p].children.last() == Some(&n))
                        .unwrap_or(true);
                    let joint = if last_child { "\u{2514}" } else { "\u{251c}" };
                    put(buf, cx(cols.trunk2()), y, joint, edge_style, right);
                    put(buf, cx(cols.trunk2() + 1), y, fill, edge_style, right);
                    put(buf, cx(cols.trunk2() + 2), y, "\u{25b8}", edge_style, right);
                }
                let mut text = look.text.clone();
                let mut style = match tint {
                    Tint::Dim => st.dim,
                    Tint::Warn => st.warn,
                    Tint::Normal if on_path => st.accent,
                    Tint::Normal => look.style,
                };
                let selected_hop = if row_selected {
                    match sel {
                        Some(MapSel::Hop(h)) => Some(h),
                        _ => None,
                    }
                } else {
                    None
                };
                if selected_hop == Some(n) {
                    style = st.selected;
                }
                let mut at = cx(col);
                // Hops folded away above this row, counted the same way on every
                // row: the pass-through hops between the last hop drawn and this
                // one. A root hop with folded hops after it shares the row with
                // them (`edge-gw ─▸ ‹ +4 hops › ─▸ vault-core`); a deeper row
                // starts with the marker. The chain is `merged`'s root + folded.
                let chain = folded_chain(map, n);
                let hidden = if merged.is_some() {
                    chain.len().saturating_sub(1)
                } else {
                    chain.len()
                };
                let limit = cx(cols.cargo).saturating_sub(1);
                if merged.is_some() || hidden > 0 {
                    let first = merged.map(|t| (t, node_look(ctx, &cols, t, &shared)));
                    let mid_selected = selected_hop.is_some_and(|h| chain.contains(&h));
                    // The columns of a plain row do not apply here: collapse the
                    // padding, and keep the chain inside the hop columns (a blank
                    // before the cargo), shortening the names before anything
                    // runs into the group label.
                    let squeeze = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
                    let mut first_t = first.as_ref().map(|(_, l)| squeeze(&l.text));
                    let mut last_t = squeeze(&text);
                    let mid = (hidden > 0).then(|| format!("\u{2039} +{hidden} hops \u{203a}"));
                    let budget = limit.saturating_sub(cx(col)) as usize;
                    // Arrows: ` ─▸` after the first hop, `─▸` after the marker.
                    let glue = first_t.as_ref().map_or(0, |_| 3)
                        + mid.as_ref().map_or(0, |m| m.chars().count() + 3);
                    let room = budget.saturating_sub(glue);
                    let fw = first_t.as_ref().map_or(0, |f| f.chars().count());
                    let lw = last_t.chars().count();
                    if fw + lw > room {
                        let first_keep = fw.min((room / 2).max(room.saturating_sub(lw)));
                        if let Some(f) = first_t.as_mut() {
                            *f = crate::tui::text::ellipsize(f, first_keep.max(4));
                        }
                        let used = first_t.as_ref().map_or(0, |f| f.chars().count());
                        last_t =
                            crate::tui::text::ellipsize(&last_t, room.saturating_sub(used).max(4));
                    }
                    if let (Some(f), Some((top_hop, look))) = (first_t, first) {
                        let first_style = if selected_hop == Some(top_hop) {
                            st.selected
                        } else if tint == Tint::Normal {
                            look.style
                        } else {
                            style
                        };
                        at = put(buf, at, y, &f, first_style, limit);
                        at = put(buf, at, y, " \u{2500}\u{25b8}", edge_style, limit);
                    }
                    if let Some(mid) = mid {
                        if merged.is_some() {
                            at = put(buf, at, y, " ", edge_style, limit);
                        }
                        let mid_style = if mid_selected { st.selected } else { st.muted };
                        at = put(buf, at, y, &mid, mid_style, limit);
                        at = put(buf, at, y, " \u{2500}\u{25b8}", edge_style, limit);
                    }
                    // `─▸●` reads as one arrow into the hop; a blank only when
                    // the previous piece was a name.
                    text = last_t;
                }
                let end = put(
                    buf,
                    at,
                    y,
                    &text,
                    style,
                    if merged.is_some() || hidden > 0 {
                        limit
                    } else {
                        sigma_x
                    },
                );
                // Edge from the node to the cargo column.
                if !node.children.is_empty() {
                    for x in end + 1..cx(cols.trunk2()) {
                        put(buf, x, y, fill, edge_style, right);
                    }
                    // A merged row can run past the trunk: no joint then.
                    if end <= cx(cols.trunk2()) {
                        put(buf, cx(cols.trunk2()), y, "\u{252c}", edge_style, right);
                    }
                    // Never over a name that runs past the trunk.
                    for x in (cx(cols.trunk2()) + 1).max(end + 1)..cx(cols.cargo).saturating_sub(1)
                    {
                        put(buf, x, y, "\u{2508}", st.dim, right);
                    }
                } else {
                    for x in end + 1..cx(cols.cargo).saturating_sub(1) {
                        put(
                            buf,
                            x,
                            y,
                            if look.dashed { "\u{2504}" } else { "\u{2508}" },
                            st.dim,
                            right,
                        );
                    }
                }
                let end = draw_strip(
                    buf,
                    cx(cols.cargo),
                    y,
                    sigma_x,
                    ctx,
                    tint,
                    &node.cargo,
                    sel_host.filter(|_| row_selected),
                    i,
                    ctx.max_glyphs,
                );
                if let (true, Some(h)) = (row_selected, sel_host) {
                    put(
                        buf,
                        end + 1,
                        y,
                        &format!("\u{25c2} {}", map.hosts[h].name),
                        st.accent,
                        sigma_x,
                    );
                }
                let sigma = format!("{:>4}", node.sigma);
                let sigma_style = match tint {
                    Tint::Dim => st.dim,
                    Tint::Warn => st.warn,
                    Tint::Normal => st.bright,
                };
                put(buf, sigma_x, y, &sigma, sigma_style, right);
                // Child trunks: a vertical line from this row to its last child.
                if !node.children.is_empty() {
                    draw_child_trunk(buf, inner, &cols, view, &ys, offset, visible, i, n, st);
                }
            }
            Row::Cycle(ci) => {
                let c = &map.cycles[ci];
                let tint = view.tint(i);
                // A lens that greys the rest of the map greys the loop too.
                let err = if tint == Tint::Dim { st.dim } else { st.err };
                let last = last_top == Some(i);
                let first = top.first() == Some(&i);
                if first {
                    put(
                        buf,
                        cx(1),
                        y,
                        "\u{25c9} you",
                        st.accent.add_modifier(Modifier::BOLD),
                        right,
                    );
                    put(buf, cx(6), y, "\u{2500}\u{2500}", st.dim, right);
                }
                put(
                    buf,
                    cx(cols.trunk1()),
                    y,
                    if last { "\u{2558}" } else { "\u{255e}" },
                    err,
                    right,
                );
                put(
                    buf,
                    cx(cols.trunk1() + 1),
                    y,
                    "\u{2550}\u{2550}",
                    err,
                    right,
                );
                let style = if row_selected && matches!(sel, Some(MapSel::Cycle(_))) {
                    st.selected
                } else if tint == Tint::Dim {
                    st.dim
                } else {
                    err
                };
                // `loop-a` at the first hop column, `loop-b` at the second, and
                // the rest of the loop (ending where it began) after it.
                let names: Vec<&str> = c.path.iter().map(|&h| map.hosts[h].name.as_str()).collect();
                let head = format!("\u{21bb} {}", names[0]);
                let mut end = put(buf, cx(cols.hop1), y, &head, style, cx(cols.hop2));
                if names.len() > 1 {
                    let to = cx(cols.hop2).saturating_sub(1);
                    for x in end + 1..to {
                        put(buf, x, y, "\u{2550}", err, right);
                    }
                    put(buf, to, y, "\u{25b8}", err, right);
                    let mut rest = format!("\u{21bb} {}", names[1]);
                    for n in &names[2..] {
                        rest.push_str(&format!(" \u{2550}\u{2550}\u{25b8} {n}"));
                    }
                    end = put(buf, cx(cols.hop2), y, &rest, style, cx(cols.cargo));
                }
                for x in end + 1..cx(cols.cargo).saturating_sub(1) {
                    put(buf, x, y, "\u{2508}", st.dim, right);
                }
                let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
                for &h in &c.hosts {
                    groups
                        .entry(map.hosts[h].group.clone())
                        .or_default()
                        .push(h);
                }
                let groups: Vec<(String, Vec<usize>)> = groups.into_iter().collect();
                let end = draw_strip(
                    buf,
                    cx(cols.cargo),
                    y,
                    sigma_x,
                    ctx,
                    tint,
                    &groups,
                    sel_host.filter(|_| row_selected),
                    i,
                    ctx.max_glyphs,
                );
                put(
                    buf,
                    end + 3,
                    y,
                    "cycle \u{b7} not connectable",
                    err,
                    sigma_x,
                );
                put(
                    buf,
                    sigma_x,
                    y,
                    &format!("{:>4}", c.hosts.len()),
                    st.bright,
                    right,
                );
            }
            Row::Cluster(c) => {
                let cl = &map.clusters[c];
                let open = view.open_clusters.contains(&cl.name);
                let tint = view.tint(i);
                let first = top.first() == Some(&i);
                let last = last_top == Some(i);
                let edge = if tint == Tint::Warn { st.warn } else { st.dim };
                if first {
                    put(
                        buf,
                        cx(1),
                        y,
                        "\u{25c9} you",
                        st.accent.add_modifier(Modifier::BOLD),
                        right,
                    );
                    put(buf, cx(6), y, "\u{2500}\u{2500}", edge, right);
                }
                let joint = match (first, last) {
                    (true, true) => "\u{2500}",
                    (true, false) => "\u{252c}",
                    (false, true) => "\u{2514}",
                    _ => "\u{251c}",
                };
                put(buf, cx(cols.trunk1()), y, joint, edge, right);
                put(
                    buf,
                    cx(cols.trunk1() + 1),
                    y,
                    "\u{2500}\u{25b8}",
                    edge,
                    right,
                );
                let style = match (
                    row_selected && matches!(sel, Some(MapSel::Cluster(_))),
                    tint,
                ) {
                    (true, _) => st.selected,
                    (_, Tint::Dim) => st.dim,
                    (_, Tint::Warn) => st.warn,
                    _ => st.bright,
                };
                let head = format!(
                    "{} {}  {} hops ",
                    if open { "\u{25be}" } else { "\u{25b8}" },
                    crate::tui::text::ellipsize(&cl.name, 20),
                    cl.roots.len()
                );
                let mut x = put(buf, cx(cols.hop1), y, &head, style, cx(cols.cargo));
                // The strip of the cluster is its hops themselves, one glyph each.
                let hops = map.row_hosts(row);
                let selected = sel_host.filter(|_| row_selected);
                let cap = 12.min(ctx.max_glyphs + 2);
                for &h in hops.iter().take(cap) {
                    let (glyph, gstyle) = ctx.glyph(h, tint);
                    let gs = if selected == Some(h) {
                        st.selected
                    } else {
                        gstyle
                    };
                    if x < cx(cols.cargo) {
                        record_glyph(ctx, x, y, i, h);
                    }
                    x = put(buf, x, y, &glyph.to_string(), gs, cx(cols.cargo));
                }
                if hops.len() > cap {
                    x = put(
                        buf,
                        x,
                        y,
                        &format!(" +{}", hops.len() - cap),
                        st.muted,
                        cx(cols.cargo),
                    );
                }
                for xx in x + 1..cx(cols.cargo).saturating_sub(1) {
                    put(buf, xx, y, "\u{2508}", st.dim, right);
                }
                // What is behind it: a note while open, a host summary when shut.
                if open {
                    put(
                        buf,
                        cx(cols.cargo),
                        y,
                        "expanded below \u{b7} sorted by \u{3a3}, largest radius first",
                        st.muted,
                        sigma_x,
                    );
                } else {
                    let behind = cluster_hosts(map, c);
                    let groups: HashSet<&str> = behind
                        .iter()
                        .map(|&h| map.hosts[h].group.as_str())
                        .collect();
                    let mut x = put(
                        buf,
                        cx(cols.cargo),
                        y,
                        &format!(
                            "{} \u{b7} {} group{} ",
                            hosts_n(behind.len()),
                            groups.len(),
                            if groups.len() == 1 { "" } else { "s" }
                        ),
                        st.muted,
                        sigma_x,
                    );
                    for &h in behind.iter().take(cap) {
                        let (glyph, gstyle) = ctx.glyph(h, tint);
                        x = put(buf, x, y, &glyph.to_string(), gstyle, sigma_x);
                    }
                    if behind.len() > cap {
                        put(
                            buf,
                            x,
                            y,
                            &format!(" +{}", behind.len() - cap),
                            st.muted,
                            sigma_x,
                        );
                    }
                }
                let sigma_style = match tint {
                    Tint::Dim => st.dim,
                    Tint::Warn => st.warn,
                    Tint::Normal => st.bright,
                };
                put(
                    buf,
                    sigma_x,
                    y,
                    &format!("{:>4}", cl.sigma),
                    sigma_style,
                    right,
                );
            }
            Row::More(c) => {
                let cl = &map.clusters[c];
                let shown = cl
                    .roots
                    .iter()
                    .filter(|&&r| view.rows.contains(&Row::Node(r)))
                    .count();
                let hidden = cl.roots.len() - shown;
                let tail_max = cl.roots[shown.min(cl.roots.len() - 1)..]
                    .iter()
                    .map(|&r| map.nodes[r].sigma)
                    .max()
                    .unwrap_or(0);
                put(
                    buf,
                    cx(cols.hop1 + 1),
                    y,
                    "\u{2514}\u{2500}\u{25b8}",
                    st.dim,
                    right,
                );
                let style = if row_selected { st.selected } else { st.muted };
                let key = key_label(
                    ctx.app
                        .config
                        .keybinds
                        .primary(crate::config::KeyAction::MapFold),
                );
                put(
                    buf,
                    cx(cols.hop1 + 4),
                    y,
                    &format!(
                        "\u{2026} {hidden} more hops  \u{3a3} \u{2264} {tail_max} each \u{b7} {key} show {REVEAL_STEP} more"
                    ),
                    style,
                    sigma_x,
                );
            }
            Row::DirectGroup(g) => {
                let (name, hosts) = &map.direct_groups[g];
                let style = if row_selected && matches!(sel, Some(MapSel::Direct)) {
                    st.selected
                } else {
                    st.muted
                };
                put(
                    buf,
                    cx(3),
                    y,
                    &format!(
                        "{:<12}{:>2}",
                        crate::tui::text::ellipsize(name, 12),
                        hosts.len()
                    ),
                    style,
                    cx(cols.cargo),
                );
                // One glyph per host: as many as the row holds, then `+n`.
                let start = cx(3 + 12 + 2 + 3);
                let room = sigma_x.saturating_sub(start + 6) as usize;
                let end = draw_strip(
                    buf,
                    start,
                    y,
                    sigma_x,
                    ctx,
                    Tint::Normal,
                    &[(String::new(), hosts.clone())],
                    sel_host.filter(|_| row_selected),
                    i,
                    room.max(1),
                );
                if let (true, Some(h)) = (row_selected, sel_host) {
                    put(
                        buf,
                        end + 1,
                        y,
                        &format!("\u{25c2} {}", map.hosts[h].name),
                        st.accent,
                        sigma_x,
                    );
                }
            }
            Row::Direct if view.direct_open => {
                put(buf, cx(1), y, "\u{25be}", st.muted, right);
                let style = if row_selected { st.selected } else { st.text };
                put(buf, cx(3), y, "direct-only", style, cx(cols.hop1 + 6));
                put(
                    buf,
                    cx(3 + 11 + 3),
                    y,
                    &format!(
                        "{} hosts with no jump \u{b7} one glyph per host \u{b7} \u{2190}\u{2192} walks hosts, \u{2191}\u{2193} groups",
                        map.direct.len()
                    ),
                    st.muted,
                    sigma_x,
                );
                put(
                    buf,
                    sigma_x,
                    y,
                    &format!("{:>4}", map.direct.len()),
                    st.bright,
                    right,
                );
            }
            Row::Direct => {
                put(buf, cx(1), y, "\u{22ef}", st.muted, right);
                let style = if row_selected { st.selected } else { st.muted };
                // The long label when the column holds it, else just the name.
                let label = if cols.cargo.saturating_sub(cols.hop1) >= 44 {
                    "direct-only \u{b7} no jump, nothing behind them"
                } else {
                    "direct-only"
                };
                put(buf, cx(cols.hop1), y, label, style, cx(cols.cargo));
                let mut x = cx(cols.cargo);
                let limit = sigma_x.saturating_sub(2);
                for (gi, (g, hosts)) in map.direct_groups.iter().enumerate() {
                    let down = hosts
                        .iter()
                        .filter(|&&h| host_status(ctx.app, st, h).glyph == '\u{d7}')
                        .count();
                    let text = format!("{g} {}", hosts.len());
                    let down_text = if down > 0 {
                        format!(" {down}\u{d7}")
                    } else {
                        String::new()
                    };
                    let gap = if gi > 0 { 3 } else { 0 };
                    let need = gap + cells(&text) + cells(&down_text);
                    let left = map.direct_groups.len() - gi;
                    // Keep room for `+n` when more groups follow.
                    if x + need + if left > 1 { 4 } else { 0 } > limit {
                        put(buf, x + gap, y, &format!("+{left}"), st.muted, limit);
                        break;
                    }
                    x += gap;
                    x = put(buf, x, y, &text, st.muted, limit);
                    x = put(buf, x, y, &down_text, st.err, limit);
                }
                put(
                    buf,
                    sigma_x,
                    y,
                    &format!("{:>4}", map.direct.len()),
                    st.bright,
                    right,
                );
                // The hint under it, when the item has its second row.
                if y + 1 < inner.y + inner.height.saturating_sub(1) {
                    let key = key_label(
                        ctx.app
                            .config
                            .keybinds
                            .primary(crate::config::KeyAction::MapFoldAll),
                    );
                    put(
                        buf,
                        cx(cols.hop1),
                        y + 1,
                        &format!("{key} expand into one glyph strip per group (a health board)"),
                        st.dim,
                        sigma_x,
                    );
                }
            }
        }
    }
}

/// Vertical trunk under a node that has children, down to its last child row.
#[allow(clippy::too_many_arguments)]
fn draw_child_trunk(
    buf: &mut Buffer,
    inner: Rect,
    cols: &Cols,
    view: &MapView,
    ys: &[usize],
    offset: usize,
    visible: usize,
    row: usize,
    node: usize,
    st: &Styles,
) {
    let map = &view.map;
    // Last descendant row of `node`: rows are depth-first, so it is the last
    // consecutive row whose node sits below `node`.
    let mut last = row;
    for (j, r) in view.rows.iter().enumerate().skip(row + 1) {
        match r {
            Row::Node(m) if map.node_path(*m).contains(&node) => last = j,
            _ => break,
        }
    }
    let x = inner.x + cols.trunk2();
    // Under a filter the children may all be hidden: nothing to draw then.
    for &yj in ys.get(row + 1..last).unwrap_or_default() {
        let Some(v) = yj.checked_sub(offset) else {
            continue;
        };
        if v < visible {
            let y = inner.y + 2 + v as u16;
            let cell = buf.cell((x, y)).map(|c| c.symbol().to_string());
            if cell.as_deref() == Some(" ") {
                buf.set_string(x, y, "\u{2502}", st.dim);
            }
        }
    }
}

// ---------------------------------------------------------- ribbon, facts

/// One card of the path ribbon.
struct Card {
    title: String,
    status: Option<usize>,
    name: String,
    addr: String,
    line3: String,
    border: Style,
}

fn source_text(source: &JumpSource) -> String {
    match source {
        JumpSource::Own => "own ProxyJump".into(),
        JumpSource::Inherited { group } => format!("inherited \u{b7} group \"{group}\""),
        JumpSource::SshConfig => "ssh_config".into(),
    }
}

fn host_addr(map: &Map, h: usize) -> String {
    let host = &map.hosts[h];
    let user = host
        .user
        .as_deref()
        .map(|u| format!("{u}@"))
        .unwrap_or_default();
    format!("{user}{}:{}", host.address, host.port)
}

fn hop_card(map: &Map, hop: &Hop, idx: usize, st: &Styles) -> Card {
    let title = format!("hop {}", idx + 1);
    match &hop.kind {
        HopKind::Saved(h) => Card {
            title,
            status: Some(*h),
            name: map.hosts[*h].name.clone(),
            addr: host_addr(map, *h),
            line3: source_text(&hop.source),
            border: st.dim,
        },
        HopKind::Raw(r) => Card {
            title,
            status: None,
            name: format!("\u{2506}{}\u{2506}", r.label()),
            addr: String::new(),
            line3: "raw \u{b7} not a saved host".into(),
            border: st.dim,
        },
        HopKind::ProxyCommand(_) => Card {
            title,
            status: None,
            name: format!("\u{2039}{PROXY_CMD_LABEL}\u{203a}"),
            addr: String::new(),
            line3: "ssh_config, opaque".into(),
            border: st.dim,
        },
        HopKind::Invalid(t) => Card {
            title,
            status: None,
            name: t.clone(),
            addr: String::new(),
            line3: "invalid hop".into(),
            border: st.err,
        },
    }
}

fn target_card(map: &Map, h: usize, st: &Styles, border_focus: Style) -> Card {
    let _ = st;
    Card {
        title: "target".into(),
        status: Some(h),
        name: map.hosts[h].name.clone(),
        addr: host_addr(map, h),
        line3: "target".into(),
        border: border_focus,
    }
}

/// Cards for the cursor: hops from the root, then the target.
fn cards_for(view: &MapView, sel: Option<MapSel>, st: &Styles) -> (Vec<Card>, bool) {
    let map = &view.map;
    let chain_cards = |hops: &[Hop]| -> Vec<Card> {
        hops.iter()
            .enumerate()
            .map(|(i, h)| hop_card(map, h, i, st))
            .collect()
    };
    match sel {
        Some(MapSel::Hop(n)) => {
            let mut cards = Vec::new();
            for (i, p) in map.node_path(n).into_iter().enumerate() {
                let last = p == n;
                let key = &map.nodes[p].key;
                let mut c = match key {
                    HopKey::Saved(h) if last => target_card(map, *h, st, st.accent),
                    HopKey::Saved(h) => Card {
                        title: format!("hop {}", i + 1),
                        status: Some(*h),
                        name: map.hosts[*h].name.clone(),
                        addr: host_addr(map, *h),
                        line3: String::new(),
                        border: st.dim,
                    },
                    other => Card {
                        title: if last {
                            "target".into()
                        } else {
                            format!("hop {}", i + 1)
                        },
                        status: None,
                        name: map.key_label(other),
                        addr: String::new(),
                        line3: String::new(),
                        border: if last { st.accent } else { st.dim },
                    },
                };
                if let HopKey::Saved(h) = key {
                    if !last {
                        c.line3 = hop_source_in_path(map, n, i, *h);
                    }
                }
                cards.push(c);
            }
            (cards, false)
        }
        Some(MapSel::Host(h)) => match &map.chains[h] {
            Chain::Hops(hops) => {
                let mut cards = chain_cards(hops);
                cards.push(target_card(map, h, st, st.accent));
                (cards, false)
            }
            Chain::Cycle(path) => (cycle_cards(map, path, st), true),
        },
        Some(MapSel::Cycle(i)) => (cycle_cards(map, &map.cycles[i].path, st), true),
        _ => (Vec::new(), false),
    }
}

/// Where the hop at `idx` of `node`'s path comes from, read from the host
/// that routes through it: any host whose chain runs through that prefix.
fn hop_source_in_path(map: &Map, node: usize, idx: usize, hop_host: usize) -> String {
    let path = map.node_path(node);
    let prefix: Vec<HopKey> = path[..=idx]
        .iter()
        .map(|&p| map.nodes[p].key.clone())
        .collect();
    for (h, chain) in map.chains.iter().enumerate() {
        if let Chain::Hops(hops) = chain {
            if hops.len() > idx && hops[..=idx].iter().map(Hop::key).eq(prefix.iter().cloned()) {
                return source_text(&hops[idx].source);
            }
        }
        let _ = h;
    }
    let _ = hop_host;
    String::new()
}

/// A loop as the ribbon draws it: the chain of the loop's first host, which is
/// every other host of the loop in order, ending on the first one again, then
/// the first hop repeats (`...`).
fn cycle_cards(map: &Map, path: &[usize], st: &Styles) -> Vec<Card> {
    let mut order: Vec<usize> = path.iter().skip(1).copied().collect();
    if let Some(&again) = path.get(1) {
        order.push(again);
    }
    let last = order.len().saturating_sub(1);
    order
        .iter()
        .enumerate()
        .map(|(i, &h)| Card {
            title: if i == last && i > 0 {
                "\u{2026}".into()
            } else {
                format!("hop {}", i + 1)
            },
            status: Some(h),
            name: map.hosts[h].name.clone(),
            addr: host_addr(map, h),
            line3: match &map.hosts[h].jump {
                Some(j) => format!("own ProxyJump = {j}"),
                None => String::new(),
            },
            border: st.err,
        })
        .collect()
}

const CARD_W: u16 = 30;
const CARD_H: u16 = 5;
const ARROW_W: u16 = 6;

fn draw_ribbon_full(buf: &mut Buffer, inner: Rect, ctx: &Ctx, sel: Option<MapSel>) {
    let (app, view, st) = (ctx.app, ctx.view, ctx.st);
    let right = inner.x + inner.width;
    let Some(sel) = sel else {
        return;
    };
    let (mut cards, is_cycle) = cards_for(view, Some(sel), st);
    if let Some(run) = map_test(app, Some(sel)) {
        apply_test(&mut cards, run, st);
    }
    if cards.is_empty() {
        return;
    }
    let avail = inner.width.saturating_sub(12);
    let fit = ((avail + ARROW_W) / (CARD_W + ARROW_W)).max(1) as usize;
    let hidden = cards.len().saturating_sub(fit);
    if hidden > 0 {
        cards.drain(..hidden);
    }
    let arrow_style = if is_cycle { st.err } else { st.dim };
    let (bar, head) = if is_cycle {
        ("\u{2550}", "\u{25b6}")
    } else {
        ("\u{2501}", "\u{25b6}")
    };
    // The cards start on the panel's first row, as in the mockups.
    let top = inner.y;
    let mid = top + 2;
    let mut x = inner.x + 1;
    x = put(
        buf,
        x,
        mid,
        "\u{25c9} you",
        st.accent.add_modifier(Modifier::BOLD),
        right,
    );
    x = put(
        buf,
        x,
        mid,
        &format!(" {bar}{bar}{bar}{head} "),
        arrow_style,
        right,
    );
    if hidden > 0 {
        put(
            buf,
            inner.x + 1,
            top + CARD_H,
            &format!("\u{2039} +{hidden} hops \u{203a}"),
            st.info.add_modifier(Modifier::BOLD),
            right,
        );
    }
    let count = cards.len();
    for (i, c) in cards.iter().enumerate() {
        draw_card(buf, x, top, c, app, st, right);
        x += CARD_W;
        if i + 1 < count {
            x = put(
                buf,
                x,
                mid,
                &format!(" {bar}{bar}{bar}{head} "),
                arrow_style,
                right,
            );
        }
    }
    if is_cycle {
        put(buf, x + 1, mid, "\u{21bb} repeats", st.err, right);
    }
    if let Some(hint) = ribbon_hint(ctx, sel) {
        put(buf, inner.x + 1, top + CARD_H + 1, &hint, st.muted, right);
    }
}

/// The line under the ribbon cards. None while a lens or a test owns the panel.
fn ribbon_hint(ctx: &Ctx, sel: MapSel) -> Option<String> {
    let view = ctx.view;
    if !matches!(view.lens, Lens::None) || ctx.test.is_some() {
        return None;
    }
    let kb = &ctx.app.config.keybinds;
    let k = |a: crate::config::KeyAction| key_label(kb.primary(a));
    Some(match sel {
        MapSel::Hop(n) => format!(
            "\u{2192} walks right along the row: hop \u{2192} its glyphs, one host per step.  \u{21b5} connects to {}.",
            view.map.key_label(&view.map.nodes[n].key)
        ),
        MapSel::Host(_) => format!(
            "\u{21b5} connect through this path   {} test   {} copy ssh   {} edit hops",
            k(crate::config::KeyAction::MapTest),
            k(crate::config::KeyAction::MapCopyCmd),
            k(crate::config::KeyAction::Edit)
        ),
        MapSel::Cycle(i) => {
            let p = &view.map.cycles[i].path;
            let name = |j: usize| view.map.hosts[p[j.min(p.len() - 1)]].name.clone();
            format!(
                "Not connectable. Clear ProxyJump on {} or {}, then {} test. \u{21b5} opens the hop editor.",
                name(0),
                name(1),
                k(crate::config::KeyAction::MapTest)
            )
        }
        MapSel::Cluster(_) => format!(
            "\u{21b5} or {} opens the cluster. {} folds all.",
            k(crate::config::KeyAction::MapFold),
            k(crate::config::KeyAction::MapFoldAll)
        ),
        MapSel::More(_) => format!("{} reveals more hops.", k(crate::config::KeyAction::MapFold)),
        MapSel::Direct => format!(
            "{} expands one strip per group.",
            k(crate::config::KeyAction::MapFoldAll)
        ),
    })
}

/// A key as the footers and hints print it: Space is `\u{2423}`, the rest as
/// bound.
pub fn key_label(spec: &str) -> String {
    match spec {
        "Space" => "\u{2423}".to_string(),
        // `Shift+G` reads `G`; `Shift+Z` stays spelled out beside the zoom `z`.
        _ => match spec.strip_prefix("Shift+") {
            Some(rest) if rest.chars().count() == 1 && rest != "Z" => rest.to_string(),
            _ => spec.to_string(),
        },
    }
}

fn draw_card(buf: &mut Buffer, x: u16, y: u16, c: &Card, app: &App, st: &Styles, max_x: u16) {
    let w = CARD_W;
    let b = c.border;
    let title = format!("\u{2500} {} ", c.title);
    put(buf, x, y, "\u{250c}", b, max_x);
    put(buf, x + 1, y, &title, b, max_x);
    for xx in x + 1 + cells(&title)..x + w - 1 {
        put(buf, xx, y, "\u{2500}", b, max_x);
    }
    put(buf, x + w - 1, y, "\u{2510}", b, max_x);
    for yy in y + 1..y + CARD_H - 1 {
        put(buf, x, yy, "\u{2502}", b, max_x);
        put(buf, x + w - 1, yy, "\u{2502}", b, max_x);
    }
    put(buf, x, y + CARD_H - 1, "\u{2514}", b, max_x);
    for xx in x + 1..x + w - 1 {
        put(buf, xx, y + CARD_H - 1, "\u{2500}", b, max_x);
    }
    put(buf, x + w - 1, y + CARD_H - 1, "\u{2518}", b, max_x);

    let inner_w = (w - 4) as usize;
    let mut name_x = x + 2;
    let mut ms = String::new();
    if let Some(h) = c.status {
        let s = host_status(app, st, h);
        name_x = put(buf, name_x, y + 1, &format!("{} ", s.glyph), s.style, max_x);
        ms = ms_text(s.ms);
    }
    let name_room = inner_w
        .saturating_sub(if c.status.is_some() { 2 } else { 0 })
        .saturating_sub(ms.len() + 1);
    put(
        buf,
        name_x,
        y + 1,
        &crate::tui::text::ellipsize(&c.name, name_room),
        st.bright,
        max_x,
    );
    if !ms.is_empty() {
        put(buf, x + w - 2 - cells(&ms), y + 1, &ms, st.muted, max_x);
    }
    put(
        buf,
        x + 2,
        y + 2,
        &crate::tui::text::ellipsize(&c.addr, inner_w),
        st.text,
        max_x,
    );
    put(
        buf,
        x + 2,
        y + 3,
        &crate::tui::text::ellipsize(&c.line3, inner_w),
        st.muted,
        max_x,
    );
}

/// Compact ribbon: one line `you > a > b > target` plus the source line.
fn draw_ribbon_line(
    buf: &mut Buffer,
    inner: Rect,
    app: &App,
    view: &MapView,
    sel: Option<MapSel>,
    st: &Styles,
) {
    let right = inner.x + inner.width;
    let Some(sel) = sel else {
        return;
    };
    let (cards, is_cycle) = cards_for(view, Some(sel), st);
    let arrow = if is_cycle {
        " \u{2550}\u{25b6} "
    } else {
        " \u{2501}\u{25b6} "
    };
    let arrow_style = if is_cycle { st.err } else { st.dim };
    let mut x = put(
        buf,
        inner.x + 1,
        inner.y,
        "\u{25c9} you",
        st.accent.add_modifier(Modifier::BOLD),
        right,
    );
    for c in &cards {
        x = put(buf, x, inner.y, arrow, arrow_style, right);
        if let Some(h) = c.status {
            let s = host_status(app, st, h);
            x = put(buf, x, inner.y, &format!("{} ", s.glyph), s.style, right);
        }
        x = put(buf, x, inner.y, &c.name, st.bright, right);
    }
    if let Some(last) = cards.last() {
        if let Some(h) = last.status {
            let s = host_status(app, st, h);
            if let Some(ms) = s.ms {
                x = put(buf, x, inner.y, &format!("  {}ms", ms), st.muted, right);
            }
            if s.glyph == '\u{25b6}' {
                put(buf, x, inner.y, " \u{25b6}", st.accent, right);
            }
        }
    }
    if let (true, Some(run)) = (inner.height > 1, map_test(app, Some(sel))) {
        put(
            buf,
            inner.x + 1,
            inner.y + 1,
            &format!("test: {}", run.headline()),
            st.muted,
            right,
        );
    } else if inner.height > 1 {
        let note = match sel {
            MapSel::Host(h) | MapSel::Cycle(h) if matches!(sel, MapSel::Host(_)) => {
                format!(
                    "{}  \u{b7}  {}",
                    jump_note(&view.map, h, false),
                    view.map.ssh_command(h)
                )
            }
            MapSel::Hop(n) => match view.map.nodes[n].key {
                HopKey::Saved(h) => {
                    format!(
                        "{}  \u{b7}  {}",
                        jump_note(&view.map, h, false),
                        view.map.node_command(n)
                    )
                }
                _ => String::new(),
            },
            MapSel::Cycle(_) => "jump loop: not connectable".into(),
            _ => String::new(),
        };
        put(buf, inner.x + 1, inner.y + 1, &note, st.muted, right);
    }
}

/// How a host's jump is set: short for the one-line ribbon, long for the facts.
fn jump_note(map: &Map, h: usize, long: bool) -> String {
    let host = &map.hosts[h];
    let Some(jump) = host
        .jump
        .as_deref()
        .filter(|j| !crate::map::is_jump_none(j))
    else {
        return if long {
            "none (reached directly)".into()
        } else {
            "reached directly, no jump".into()
        };
    };
    match (&host.jump_source, long) {
        (Some(JumpSource::Inherited { group }), false) => {
            format!("jump inherited from group \"{group}\"")
        }
        (Some(JumpSource::Inherited { group }), true) => {
            format!(
                "inherited from \"{group}\" = {jump}{}",
                first_hop_chain(map, jump)
            )
        }
        (Some(JumpSource::SshConfig), false) => "jump from ssh_config".into(),
        (Some(JumpSource::SshConfig), true) => {
            format!(
                "ssh_config ProxyJump = {jump}{}",
                first_hop_chain(map, jump)
            )
        }
        (_, false) => "jump set on this host".into(),
        (_, true) => format!("own ProxyJump = {jump}{}", first_hop_chain(map, jump)),
    }
}

/// `; jump-2's own = bastion-eu` when the first hop is an ssh_config host that
/// brings a jump of its own (ssh follows it).
fn first_hop_chain(map: &Map, jump: &str) -> String {
    let first = jump.split(',').next().unwrap_or("").trim();
    map.hosts
        .iter()
        .find(|h| h.name == first && h.ssh_config)
        .and_then(|h| {
            h.jump
                .as_deref()
                .map(|j| format!("; {}'s own = {j}", h.name))
        })
        .unwrap_or_default()
}

fn draw_legend(buf: &mut Buffer, inner: Rect, st: &Styles) {
    let right = inner.x + inner.width;
    let line = |buf: &mut Buffer, y: u16, parts: &[(&str, Style)]| {
        let mut x = inner.x + 1;
        for (s, style) in parts {
            x = put(buf, x, inner.y + y, s, *style, right);
        }
    };
    line(
        buf,
        0,
        &[
            ("\u{25cf} up ", st.ok),
            ("\u{25cf} slow ", st.warn),
            ("\u{d7} down ", st.err),
            ("\u{25cb} unknown ", st.unknown),
            ("\u{25b6} session", st.accent),
            ("   ping is from here", st.dim),
        ],
    );
    line(
        buf,
        1,
        &[(
            "\u{2500}\u{25b8} jump edge   \u{2508}\u{2508} \"hosts behind\" leader   \u{3a3} all hosts behind, any depth",
            st.muted,
        )],
    );
    line(
        buf,
        2,
        &[
            ("\u{2506}raw\u{2506}  ", st.muted),
            ("\u{2039}proxy cmd\u{203a} \u{2504}\u{2504}  ", st.info),
            ("\u{21bb} \u{2550}\u{2550} cycle  ", st.err),
            ("\u{25c7} reused hop", st.info),
        ],
    );
    line(
        buf,
        3,
        &[(
            "lenses: o tunnels  B blast  t test, they re-skin glyphs and edges",
            st.muted,
        )],
    );
}

fn facts_title(ctx: &Ctx, sel: Option<MapSel>) -> String {
    let view = ctx.view;
    if ctx.test.is_some() {
        return "steps".into();
    }
    match &view.lens {
        Lens::Blast(b) => format!("blast radius \u{2500} {}", view.map.key_label(&b.key)),
        Lens::Tunnel => format!(
            "tunnels \u{2500} {}",
            ctx.tunnels.values().map(Vec::len).sum::<usize>()
        ),
        Lens::None => match sel {
            Some(MapSel::Host(h)) => format!("host \u{2500} {}", view.map.hosts[h].name),
            _ => format!("node \u{2500} {}", sel_title(view, sel)),
        },
    }
}

/// `\u{25b6} open 2h` for a host with a live session.
fn session_text(app: &App, name: &str) -> Option<String> {
    let s = app
        .sessions
        .iter()
        .find(|s| !s.local_shell && s.config.host_name == name)?;
    Some(match s.phase {
        crate::session::SessionPhase::Running { started_at } => {
            let secs = started_at.elapsed().as_secs();
            let age = if secs >= 3600 {
                format!("{}h", secs / 3600)
            } else if secs >= 60 {
                format!("{}m", secs / 60)
            } else {
                format!("{secs}s")
            };
            format!("\u{25b6} open {age}")
        }
        _ => "\u{25b6} connecting".to_string(),
    })
}

fn tunnel_state_word(t: TunnelState) -> &'static str {
    match t {
        TunnelState::Running => "running",
        TunnelState::Reconnecting => "reconnecting",
        TunnelState::Stopped => "stopped",
    }
}

/// Lines of the blast-radius panel: split of the radius, what rides on it, and
/// the hosts a shared hop keeps out of it.
fn blast_lines(ctx: &Ctx, b: &crate::app::network_map::BlastLens) -> Vec<String> {
    let (app, map) = (ctx.app, &ctx.view.map);
    let n = b.hosts.len();
    // Hosts whose last hop is the key sit directly behind it; the rest sit
    // behind a hop that comes after it.
    let mut direct = 0usize;
    let mut next: HashMap<String, usize> = HashMap::new();
    for &h in &b.hosts {
        let keys = map.chain_keys(h);
        match keys.iter().position(|k| *k == b.key) {
            Some(i) if i + 1 == keys.len() => direct += 1,
            Some(i) => *next.entry(map.key_label(&keys[i + 1])).or_default() += 1,
            None => {}
        }
    }
    let behind = n - direct.min(n);
    let via = match next.len() {
        0 => String::new(),
        1 => format!(" {}", next.keys().next().expect("one")),
        k => format!(" {k} other hops"),
    };
    let split = if behind == 0 {
        format!("{}, all directly behind it", hosts_n(n))
    } else {
        format!(
            "{}: {direct} directly behind, {behind} behind{via}",
            hosts_n(n)
        )
    };
    let sessions = b
        .hosts
        .iter()
        .filter(|&&h| session_text(app, &map.hosts[h].name).is_some())
        .count();
    let tunnels: usize = b
        .hosts
        .iter()
        .chain(match &b.key {
            HopKey::Saved(h) => Some(h),
            _ => None,
        })
        .filter_map(|h| ctx.tunnels.get(h))
        .map(Vec::len)
        .sum();
    // A hop under this one that other chains reuse: the hosts behind those
    // other chains are not hit.
    let below: HashSet<HopKey> = (0..map.nodes.len())
        .filter(|&i| map.nodes[i].key == b.key)
        .flat_map(|i| {
            let mut out = Vec::new();
            let mut stack: Vec<usize> = map.nodes[i].children.clone();
            while let Some(c) = stack.pop() {
                out.push(map.nodes[c].key.clone());
                stack.extend(map.nodes[c].children.iter().copied());
            }
            out
        })
        .collect();
    let mut not_hit: Vec<String> = Vec::new();
    for h in 0..map.hosts.len() {
        if b.hosts.contains(&h) {
            continue;
        }
        let keys = map.chain_keys(h);
        if let Some(i) = keys.iter().position(|k| below.contains(k)) {
            let hop = map.key_label(&keys[i]);
            let by = i
                .checked_sub(1)
                .map(|j| map.key_label(&keys[j]))
                .map(|by| format!("via {by}"))
                .unwrap_or_else(|| "directly".into());
            not_hit.push(format!("{} ({hop} {by})", map.hosts[h].name));
        }
    }
    not_hit.sort();
    let mut lines = vec![
        split,
        format!("{sessions} sessions open \u{b7} {tunnels} tunnels ride on it"),
    ];
    if let Some(first) = not_hit.first() {
        let more = if not_hit.len() > 1 {
            format!(" +{} more", not_hit.len() - 1)
        } else {
            String::new()
        };
        lines.push(format!("not hit: {first}{more}"));
    }
    lines.push("\u{3a3} is the radius of every row, before you press B.".into());
    lines
}

fn draw_facts(buf: &mut Buffer, inner: Rect, ctx: &Ctx, sel: Option<MapSel>) {
    let (view, st) = (ctx.view, ctx.st);
    let right = inner.x + inner.width;
    let map = &view.map;
    if let Some(run) = ctx.test {
        for (i, (step, state)) in run.steps.iter().zip(&run.states).enumerate() {
            if i >= inner.height.saturating_sub(1) as usize {
                break;
            }
            let (glyph, text) = crate::app::hop_editor::step_text(state);
            let style = if state.is_failure() { st.err } else { st.text };
            let cmd = crate::tui::text::ellipsize(
                if step.shown.is_empty() {
                    &step.label
                } else {
                    &step.shown
                },
                40,
            );
            put(
                buf,
                inner.x + 1,
                inner.y + i as u16,
                &format!("{}  {cmd:<40} {glyph} {text}", i + 1),
                style,
                right,
            );
        }
        let last = inner.y + inner.height.saturating_sub(1);
        let note = match run.failure() {
            Some((i, reason, detail)) => format!(
                "\u{2717} hop {} ({}): {reason} {detail}   H known hosts  P push key  l ssh log  t again",
                i + 1,
                run.steps[i].label
            ),
            None if run.running() => "Stops at the first failure. Esc cancels.".to_string(),
            None => "Esc closes. t tests again.".to_string(),
        };
        put(buf, inner.x + 1, last, &note, st.muted, right);
        return;
    }
    // (label, value): an empty label is a plain line.
    let mut rows: Vec<(&str, String)> = Vec::new();
    match &view.lens {
        Lens::Blast(b) => {
            rows.extend(blast_lines(ctx, b).into_iter().map(|l| ("", l)));
        }
        Lens::Tunnel => {
            let mut all: Vec<(&usize, &HostTunnel)> = ctx
                .tunnels
                .iter()
                .flat_map(|(h, ts)| ts.iter().map(move |t| (h, t)))
                .collect();
            all.sort_by_key(|(h, t)| (map.hosts[**h].name.clone(), t.id));
            for (h, t) in all {
                rows.push((
                    "",
                    format!(
                        "\u{21c4}{} {:<12} {:<18} {}",
                        tunnel_letter(t.kind),
                        crate::tui::text::ellipsize(&map.hosts[*h].name, 12),
                        t.summary,
                        tunnel_state_word(t.state)
                    ),
                ));
            }
            let at = inner.height as usize;
            let mut note = String::from("hosts without a tunnel show \u{b7}");
            if rows.len() >= at && at > 0 {
                let more = rows.len() - (at - 1);
                rows.truncate(at - 1);
                note = format!("+{more} more tunnels \u{b7} {note}");
            }
            rows.push(("", note));
        }
        Lens::None => facts_rows(ctx, sel, &mut rows),
    }
    for (i, (k, v)) in rows.iter().take(inner.height as usize).enumerate() {
        let y = inner.y + i as u16;
        if k.is_empty() {
            put(buf, inner.x + 1, y, v, st.text, right);
        } else {
            let x = put(buf, inner.x + 1, y, &format!("{k:<10}"), st.muted, right);
            put(buf, x, y, v, st.text, right);
        }
    }
}

/// The default facts rows for what the cursor is on.
fn facts_rows(ctx: &Ctx, sel: Option<MapSel>, rows: &mut Vec<(&'static str, String)>) {
    let (app, view) = (ctx.app, ctx.view);
    let map = &view.map;
    let addr_row = |h: usize, hop: bool, rows: &mut Vec<(&'static str, String)>| {
        let host = &map.hosts[h];
        let tags: String = host
            .tags
            .iter()
            .map(|t| format!("{}#{t}", if t == &host.tags[0] { "  " } else { " " }))
            .collect();
        // `group prod  #bastion` on a hop, `prod/db  #db #pci` on a host.
        let group = if hop {
            format!("group {}", host.group)
        } else {
            host.group.clone()
        };
        rows.push((
            "address",
            format!("{}  \u{b7}  {group}{tags}", host_addr(map, h)),
        ));
        rows.push(("jump", jump_note(map, h, true)));
    };
    match sel {
        Some(MapSel::Hop(n)) => {
            let node = &map.nodes[n];
            match &node.key {
                HopKey::Saved(h) => {
                    addr_row(*h, true, rows);
                    let direct: usize = node.cargo.iter().map(|(_, hs)| hs.len()).sum();
                    let behind = node.sigma - direct;
                    let via = match node.children.len() {
                        0 => String::new(),
                        1 => format!(" {}", map.key_label(&map.nodes[node.children[0]].key)),
                        k => format!(" {k} hops"),
                    };
                    let split = if behind == 0 {
                        hosts_n(node.sigma)
                    } else {
                        format!(
                            "{} ({direct} direct + {behind} behind{via})",
                            hosts_n(node.sigma)
                        )
                    };
                    rows.push((
                        "carries",
                        format!(
                            "{split}  {} blast",
                            key_label(
                                ctx.app
                                    .config
                                    .keybinds
                                    .primary(crate::config::KeyAction::MapBlast)
                            )
                        ),
                    ));
                    tunnel_rows(ctx, *h, rows);
                }
                other => {
                    rows.push(("hop", map.key_label(other)));
                    rows.push((
                        "carries",
                        format!("{} behind this hop", hosts_n(node.sigma)),
                    ));
                }
            }
        }
        Some(MapSel::Host(h)) => {
            addr_row(h, false, rows);
            let mut live = session_text(app, &map.hosts[h].name).unwrap_or_default();
            for t in ctx.tunnels.get(&h).into_iter().flatten() {
                if !live.is_empty() {
                    live.push_str("  ");
                }
                live.push_str(&format!(
                    "\u{21c4}{} {} {}",
                    tunnel_letter(t.kind),
                    t.summary,
                    tunnel_state_word(t.state)
                ));
            }
            if !live.is_empty() {
                rows.push(("session", live));
            }
            rows.push(("ssh", map.ssh_command(h)));
        }
        Some(MapSel::Cycle(i)) => {
            let names: Vec<&str> = map.cycles[i]
                .path
                .iter()
                .map(|&h| map.hosts[h].name.as_str())
                .collect();
            rows.push(("loop", names.join(" \u{2192} ")));
            rows.push((
                "hosts",
                format!("{} stuck behind it", map.cycles[i].hosts.len()),
            ));
        }
        Some(MapSel::Direct) => {
            rows.push(("direct", format!("{}, no jump", hosts_n(map.direct.len()))));
        }
        Some(MapSel::Cluster(c)) => {
            let cl = &map.clusters[c];
            rows.push(("cluster", cl.name.clone()));
            rows.push((
                "carries",
                format!("{} hops, {} behind them", cl.roots.len(), hosts_n(cl.sigma)),
            ));
        }
        Some(MapSel::More(c)) => {
            rows.push(("more", format!("hops folded in {}", map.clusters[c].name)));
        }
        None => {}
    }
}

fn tunnel_rows(ctx: &Ctx, h: usize, rows: &mut Vec<(&'static str, String)>) {
    for t in ctx.tunnels.get(&h).into_iter().flatten() {
        rows.push((
            "tunnels",
            format!(
                "\u{21c4}{} {} {}",
                tunnel_letter(t.kind),
                t.summary,
                tunnel_state_word(t.state)
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::MapHost;
    use crate::test_support::{find_text, frame_at, resolved_default, themed_app};

    fn host(name: &str, group: &str, jump: Option<&str>) -> MapHost {
        MapHost {
            name: name.into(),
            address: format!("{name}.example"),
            user: Some("ops".into()),
            port: 22,
            group: group.into(),
            jump: jump.map(String::from),
            jump_source: jump.map(|_| JumpSource::Own),
            ..Default::default()
        }
    }

    fn app_with(hosts: Vec<MapHost>) -> App {
        let mut app = themed_app(resolved_default());
        app.map_view = Some(MapView::new(Map::build(hosts)));
        app.active_tab = 5;
        app
    }

    fn fleet() -> Vec<MapHost> {
        let mut iot = host("iot-1", "iot", None);
        iot.proxy_command = true;
        iot.ssh_config = true;
        vec![
            host("bastion-eu", "prod", None),
            {
                let mut h = host("jump-2", "prod", Some("bastion-eu"));
                h.ssh_config = true;
                h
            },
            host("web-1", "prod", Some("bastion-eu")),
            host("db-1", "prod/db", Some("jump-2")),
            host("bastion-us", "us", None),
            host("db-us-2", "us", Some("bastion-us,jump-2")),
            host("legacy-1", "legacy", Some("ops@203.0.113.7:2222")),
            iot,
            MapHost {
                ssh_config: true,
                ..host("loop-a", "lab", Some("loop-b"))
            },
            MapHost {
                ssh_config: true,
                ..host("loop-b", "lab", Some("loop-a"))
            },
            host("standalone", "staging", None),
        ]
    }

    fn render(app: &App, w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        frame_at(area, |f| render_map(f, area, app))
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect::<String>()
    }

    fn all_text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| row_text(buf, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn default_screen_shows_hops_cargo_sigma_and_special_rows() {
        let app = app_with(fleet());
        let buf = render(&app, 160, 45);
        let text = all_text(&buf);
        for needle in [
            "hosts behind this hop",
            "\u{25c9} you",
            "bastion-eu",
            "bastion-us",
            "jump-2",
            "\u{25c7}", // jump-2 sits under two parents
            "\u{2506}ops@203.0.113.7:2222\u{2506}",
            "\u{2039}proxy cmd\u{203a}",
            "\u{21bb} loop-a",
            "cycle \u{b7} not connectable",
            "direct-only",
            "path ribbon",
            "legend",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
        // The ProxyCommand text is never shown.
        assert!(!text.contains("cloudflared"));
        // Σ of bastion-eu: web-1, jump-2 and db-1 behind it.
        let (_, y) = find_text(&buf, "bastion-eu");
        assert!(row_text(&buf, y)
            .trim_end_matches([' ', '\u{2502}'])
            .ends_with('3'));
    }

    #[test]
    fn cursor_walks_hop_then_glyphs_and_ribbon_follows() {
        let mut app = app_with(fleet());
        // Down to the jump-2 row under bastion-eu, right to its glyph.
        {
            let v = app.map_view.as_mut().unwrap();
            v.move_row(1);
            assert!(matches!(v.selection(), Some(MapSel::Hop(_))));
        }
        let buf = render(&app, 160, 45);
        assert!(all_text(&buf).contains("path ribbon \u{2500} "));
        let v = app.map_view.as_mut().unwrap();
        v.move_col(1);
        v.move_col(1);
        assert!(matches!(v.selection(), Some(MapSel::Host(_))));
        let buf = render(&app, 160, 45);
        let text = all_text(&buf);
        assert!(text.contains("\u{25c2} db-1"), "{text}");
        assert!(text.contains("target"));
    }

    #[test]
    fn tiny_area_shows_the_size_hint_instead_of_panicking() {
        let app = app_with(fleet());
        let buf = render(&app, 39, 11);
        assert!(all_text(&buf).contains("map needs 40x12"));
        // Compact sizes draw without a ribbon panel but do not panic.
        for (w, h) in [(40, 12), (100, 30), (80, 24), (120, 36), (200, 60)] {
            let _ = render(&app, w, h);
        }
    }

    #[test]
    fn ping_down_is_unknown_never_a_failed_hop() {
        let mut app = app_with(fleet());
        app.ping_data
            .insert("bastion-eu".into(), vec![crate::ping::PING_UNREACHABLE]);
        let buf = render(&app, 160, 45);
        let (x, y) = find_text(&buf, "bastion-eu");
        assert_eq!(buf.cell((x - 2, y)).unwrap().symbol(), "\u{25cb}");
    }

    /// 212 jump hosts in 4 groups of 53 (a chain of up to 6 hops in the first
    /// group) and 1,840 hosts in all.
    fn big_fleet() -> Vec<MapHost> {
        let mut hosts = Vec::new();
        for g in 0..4 {
            for j in 0..53 {
                let name = format!("jump-{g}-{j}");
                let jump = if g == 0 && (1..6).contains(&j) {
                    Some(format!("jump-0-{}", j - 1))
                } else {
                    None
                };
                let mut h = host(&name, &format!("hops-{g}"), jump.as_deref());
                h.ssh_config = true;
                hosts.push(h);
            }
        }
        let jumps = hosts.len();
        for k in 0..(1840 - jumps) {
            let via = format!("jump-{}-{}", k % 4, (k / 4) % 53);
            hosts.push(host(
                &format!("srv-{k}"),
                &format!("fleet-{}", k % 9),
                Some(&via),
            ));
        }
        hosts
    }

    #[test]
    fn big_map_is_clustered_sorted_and_tail_folded() {
        let app = app_with(big_fleet());
        let v = app.map_view.as_ref().unwrap();
        assert_eq!(v.map.hosts.len(), 1840);
        assert_eq!(v.map.clusters.len(), 4);
        assert!(v.clustered());
        // Only the biggest cluster starts open; its tail folds after 12 hops.
        let clusters = v
            .rows
            .iter()
            .filter(|r| matches!(r, Row::Cluster(_)))
            .count();
        assert_eq!(clusters, 4);
        assert_eq!(v.open_clusters.len(), 1);
        assert!(v.rows.iter().any(|r| matches!(r, Row::More(_))));
        let buf = render(&app, 160, 45);
        let text = all_text(&buf);
        assert!(text.contains("hops-0"), "{text}");
        assert!(text.contains("more hops"), "{text}");
        assert!(text.contains("hops-1"), "{text}");
    }

    #[test]
    fn cluster_fold_reveal_and_filter_reveal() {
        let mut app = app_with(big_fleet());
        let v = app.map_view.as_mut().unwrap();
        // Cursor on the first cluster row: space folds it, space opens it.
        assert!(matches!(v.selection(), Some(MapSel::Cluster(_))));
        let rows_open = v.rows.len();
        v.toggle_fold();
        assert!(v.rows.len() < rows_open);
        v.toggle_fold();
        assert_eq!(v.rows.len(), rows_open);
        // Walk to the More row: space reveals 20 more hops.
        let more = v
            .rows
            .iter()
            .position(|r| matches!(r, Row::More(_)))
            .unwrap();
        v.row = more;
        v.pos = 0;
        v.toggle_fold();
        assert!(v.rows.len() > rows_open);
        // A filter + Enter opens only the clusters that hold the match.
        v.query = "via:jump-2-7".into();
        v.apply_filter();
        v.reveal_matches();
        let open: Vec<_> = v.open_clusters.iter().cloned().collect();
        assert_eq!(open, ["hops-2"]);
        // Shift+Z folds all, the next press expands direct-only (none here).
        v.fold_all();
        assert!(v.open_clusters.is_empty());
    }

    #[test]
    fn direct_only_expands_into_one_strip_per_group() {
        let mut hosts = fleet();
        for k in 0..30 {
            hosts.push(host(
                &format!("lone-{k}"),
                if k % 2 == 0 { "lab" } else { "home" },
                None,
            ));
        }
        let mut app = app_with(hosts);
        let v = app.map_view.as_mut().unwrap();
        let direct = v.rows.iter().position(|r| *r == Row::Direct).unwrap();
        v.row = direct;
        v.pos = 0;
        v.toggle_fold();
        assert!(v.rows.iter().any(|r| matches!(r, Row::DirectGroup(_))));
        let text = all_text(&render(&app, 160, 45));
        assert!(text.contains("one glyph per host"), "{text}");
        assert!(text.contains("lab") && text.contains("home"), "{text}");
    }

    #[test]
    fn narrow_modes_render_without_overflow() {
        let app = app_with(fleet());
        // 100x30: compact columns, no latency, one-line ribbon.
        let t = all_text(&render(&app, 100, 30));
        assert!(
            t.contains("bastion-eu") && t.contains("reached directly"),
            "{t}"
        );
        // Under 80x24: outline with the selected path on the last line.
        let buf = render(&app, 70, 20);
        let last = row_text(&buf, 19);
        assert!(last.starts_with("you "), "{last:?}");
        let t = all_text(&buf);
        assert!(t.contains("\u{251c} "), "{t}");
        // Under 40x12: the size hint.
        assert!(all_text(&render(&app, 39, 11)).contains("map needs 40x12"));
        // Every size draws.
        for (w, h) in [(40, 12), (79, 23), (80, 24), (99, 29), (200, 60)] {
            let _ = render(&app, w, h);
        }
    }

    #[test]
    fn the_212_jump_host_fleet_renders_fully_opened() {
        let mut app = app_with(big_fleet());
        // Open everything and walk to the middle: the worst case. Only that it
        // draws: a frame-time bound would flake on a loaded runner.
        let v = app.map_view.as_mut().unwrap();
        for c in v.map.clusters.clone() {
            v.open_clusters.insert(c.name);
        }
        v.apply_filter();
        v.row = v.rows.len() / 2;
        let text = all_text(&render(&app, 160, 45));
        assert!(text.contains("jump hosts"), "{text}");
    }

    #[test]
    fn empty_map_renders() {
        let app = app_with(Vec::new());
        let buf = render(&app, 160, 45);
        let t = all_text(&buf);
        assert!(t.contains("No hosts yet."), "{t}");
    }

    #[test]
    fn a_long_chain_is_one_row_with_a_fold_marker() {
        let hops = "u@10.0.0.1,u@10.0.0.2,u@10.0.0.3,u@10.0.0.4,u@10.0.0.5,u@10.0.0.6";
        let mut hosts = vec![host("deep-1", "vault", Some(hops))];
        hosts.push(host("deep-2", "vault", Some(hops)));
        let mut app = app_with(hosts);
        let v = app.map_view.as_ref().unwrap();
        // One row for the whole chain (plus the direct-only row, if any).
        let node_rows = v.rows.iter().filter(|r| matches!(r, Row::Node(_))).count();
        assert_eq!(node_rows, 1, "{:?}", v.rows);
        let text = all_text(&render(&app, 160, 45));
        assert!(
            text.contains("\u{2039} +4 hops \u{203a}"),
            "one fold marker with the number of hidden hops:\n{text}"
        );
        // The row starts at the left trunk with the first hop, ends with the last.
        let row = text.lines().find(|l| l.contains("+4 hops")).unwrap();
        assert!(
            row.contains("10.0.0.1") && row.contains("10.0.0.6"),
            "{row}"
        );
        assert!(row.contains("\u{25c9} you"), "{row}");
        // Arrow keys walk the hidden hops without a row of their own: the
        // ribbon still follows the highlight.
        let v = app.map_view.as_mut().unwrap();
        v.move_col(-1);
        v.move_col(-1);
        assert!(matches!(v.selection(), Some(MapSel::Hop(_))));
    }

    #[test]
    fn a_filter_that_hides_every_child_row_does_not_panic() {
        // The match is the parent hop; its children are filtered out, so the
        // trunk under it has no rows to draw (this sliced `ys[row + 1..row]`).
        for query in [
            "bastion-eu",
            "bastion",
            "tag:bastion",
            "jump-2",
            "!db-1",
            "zzz",
        ] {
            let mut app = app_with(fleet());
            let v = app.map_view.as_mut().unwrap();
            v.query = query.into();
            v.apply_filter();
            for (w, h) in [(160, 45), (155, 31), (100, 30), (79, 23), (60, 20)] {
                let _ = render(&app, w, h);
            }
        }
    }

    #[test]
    fn a_folded_row_keeps_its_arrows_and_never_runs_into_the_group() {
        // Saved hops with latency, and raw hops: the whole chain stays inside
        // the hop columns, with a blank before the group label.
        let raw = "ops@127.0.0.65,u@10.0.0.2,u@10.0.0.3,ops@127.0.0.66";
        for (name, jump, width) in [("a", raw, 160u16), ("b", raw, 100), ("c", raw, 120)] {
            let mut hosts = vec![host(&format!("deep-{name}"), "prod", Some(jump))];
            hosts.push(host(&format!("deep-{name}2"), "prod", Some(jump)));
            let app = app_with(hosts);
            let text = all_text(&render(&app, width, 45));
            let row = text.lines().find(|l| l.contains("+2 hops")).unwrap_or("");
            assert!(row.contains("\u{203a} \u{2500}\u{25b8}"), "{width}: {row}");
            assert!(
                !row.contains("\u{2506}prod") && !row.contains("0prod"),
                "{width}: {row}"
            );
            let cut = row.find("prod").expect("group label");
            assert!(row[..cut].ends_with(' '), "{width}: {row}");
        }
    }

    /// The tree rows that mention `needle`, frame removed, blanks collapsed.
    fn tree_rows(app: &App, width: u16, needle: &str) -> Vec<String> {
        all_text(&render(app, width, 45))
            .lines()
            .filter(|l| l.contains(needle) && l.contains('\u{2502}'))
            .map(|l| {
                let inner = l.trim().trim_matches('\u{2502}');
                inner.split_whitespace().collect::<Vec<_>>().join(" ")
            })
            .collect()
    }

    #[test]
    fn folded_chain_rows_have_exact_text_named_and_raw() {
        let mut hosts: Vec<MapHost> = (1..=9)
            .map(|k| host(&format!("g{k}"), "ops", None))
            .collect();
        hosts.push(host("t1", "prod", Some("g1,g2,g3,g4,g5")));
        hosts.push(host("t2", "prod", Some("g1,g2,g3,g4,g5,g6")));
        hosts.push(host("t3", "prod", Some("g1,g2,g3,g4,g5,g7,g8,g9")));
        let app = app_with(hosts);
        // The root row carries the hops folded after it, the deeper row the
        // ones folded above it: each count is the pass-through hops between
        // the last hop drawn and the next, `\u{2500}\u{25b8}` on both sides.
        let rows = tree_rows(&app, 160, "hops");
        assert_eq!(
            rows[0],
            "\u{25c9} you\u{2500}\u{2500}\u{2500}\u{2500}\u{25b8}\u{25cb} g1 \u{2500}\u{25b8} \u{2039} +3 hops \u{203a} \u{2500}\u{25b8}\u{25cb} g5 \u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508} prod \u{25cb} 3"
        );
        assert_eq!(
            rows[1],
            "\u{2514}\u{2500}\u{25b8}\u{2039} +2 hops \u{203a} \u{2500}\u{25b8}\u{25cb} g9 \u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508}\u{2508} prod \u{25cb} 1"
        );

        let raw = "ops@127.0.0.61,ops@127.0.0.62,ops@127.0.0.63,ops@127.0.0.64,ops@127.0.0.65";
        let hosts = vec![
            host("r1", "prod", Some(raw)),
            host("r2", "prod", Some(&format!("{raw},ops@127.0.0.66"))),
            host(
                "r3",
                "prod",
                Some(&format!(
                    "{raw},ops@127.0.0.67,ops@127.0.0.68,ops@127.0.0.69"
                )),
            ),
        ];
        let app = app_with(hosts);
        let rows = tree_rows(&app, 160, "hops");
        assert_eq!(
            rows[0],
            "\u{25c9} you\u{2500}\u{2500}\u{2500}\u{2500}\u{25b8}\u{2506}ops@127.0.0.61\u{2506} \u{2500}\u{25b8} \u{2039} +3 hops \u{203a} \u{2500}\u{25b8}\u{2506}ops@127.0.0.65\u{2506} \u{2508}\u{2508} prod \u{25cb} 3"
        );
        assert_eq!(
            rows[1],
            "\u{2514}\u{2500}\u{25b8}\u{2039} +2 hops \u{203a} \u{2500}\u{25b8}\u{2506}ops@127.0.\u{2026} prod \u{25cb} 1"
        );
    }
}
