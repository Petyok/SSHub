//! Reusable bordered panel box for dashboard columns.
//!
//! Draws box-drawing borders from a caller-supplied [`PanelRoles`] bundle, with
//! an optional title and count badge embedded in the top border. Every panel on
//! the dashboard — and the SFTP and broadcast panels — names its own five roles,
//! so a theme can retint one panel without touching the others.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::theme::catalog::{PaintRole, StyleRole};
use crate::theme::gradient::paint_gradient_ring;
use crate::theme::model::ResolvedTheme;
use crate::tui::text::ellipsize;

/// The five roles one panel frame is painted from.
///
/// Border and focused border are separate `Paint` roles (either may be a
/// gradient); title and count are `Style` roles so they stay independently
/// legible over a ring; the background is the `Paint` the panel body sits on.
#[derive(Clone, Copy)]
pub struct PanelRoles {
    pub border: PaintRole,
    pub border_focused: PaintRole,
    pub title: StyleRole,
    pub count: StyleRole,
    pub background: PaintRole,
}

/// Declare the role bundle of one panel from its `components.<prefix>.*` family.
macro_rules! panel_roles {
    ($name:ident, $border:ident, $focused:ident, $title:ident, $count:ident, $background:ident) => {
        pub(crate) const $name: PanelRoles = PanelRoles {
            border: PaintRole::$border,
            border_focused: PaintRole::$focused,
            title: StyleRole::$title,
            count: StyleRole::$count,
            background: PaintRole::$background,
        };
    };
}

panel_roles!(
    HOST_LIST_PANEL,
    DashboardHostListBorder,
    DashboardHostListBorderFocused,
    DashboardHostListTitle,
    DashboardHostListCount,
    DashboardHostListBackground
);
panel_roles!(
    DETAILS_PANEL,
    DashboardDetailsBorder,
    DashboardDetailsBorderFocused,
    DashboardDetailsTitle,
    DashboardDetailsCount,
    DashboardDetailsBackground
);
panel_roles!(
    SSH_LOG_PANEL,
    DashboardSshLogBorder,
    DashboardSshLogBorderFocused,
    DashboardSshLogTitle,
    DashboardSshLogCount,
    DashboardSshLogBackground
);
panel_roles!(
    AGENT_PANEL,
    DashboardAgentBorder,
    DashboardAgentBorderFocused,
    DashboardAgentTitle,
    DashboardAgentCount,
    DashboardAgentBackground
);
panel_roles!(
    LATENCY_PANEL,
    DashboardLatencyBorder,
    DashboardLatencyBorderFocused,
    DashboardLatencyTitle,
    DashboardLatencyCount,
    DashboardLatencyBackground
);
panel_roles!(
    RECENT_PANEL,
    DashboardRecentBorder,
    DashboardRecentBorderFocused,
    DashboardRecentTitle,
    DashboardRecentCount,
    DashboardRecentBackground
);
panel_roles!(
    AUTH_PANEL,
    DashboardAuthBorder,
    DashboardAuthBorderFocused,
    DashboardAuthTitle,
    DashboardAuthCount,
    DashboardAuthBackground
);
panel_roles!(
    PING_PANEL,
    DashboardPingBorder,
    DashboardPingBorderFocused,
    DashboardPingTitle,
    DashboardPingCount,
    DashboardPingBackground
);
panel_roles!(
    SFTP_PANEL,
    SftpPanelBorder,
    SftpPanelBorderFocused,
    SftpPanelTitle,
    SftpPanelCount,
    SftpPanelBackground
);
panel_roles!(
    BROADCAST_PANEL,
    BroadcastPanelBorder,
    BroadcastPanelBorderFocused,
    BroadcastPanelTitle,
    BroadcastPanelCount,
    BroadcastPanelBackground
);

/// A dashboard panel switches from its compact grid layout to the richer zoomed
/// layout once it is drawn at least this tall (#35). Deciding by the panel's
/// actual box height (not the global zoom flag) means the content only swaps
/// once the zoom morph has grown the box enough to hold it, instead of popping
/// the instant `z` is pressed. Above every compact panel height (max 9), below
/// a full-body zoom, so a grid slot always reads as compact.
pub const ZOOM_CONTENT_MIN: u16 = 13;

/// Write `s` at (`x`,`y`), truncated with `…` so it never exceeds `max_w`
/// display columns — keeps dashboard text inside its panel border even when
/// the column is narrow (e.g. after a zoom). Returns the columns written.
pub fn put_clamped(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style, max_w: usize) -> u16 {
    if max_w == 0 {
        return 0;
    }
    let text = ellipsize(s, max_w);
    buf.set_string(x, y, &text, style);
    text.chars().count() as u16
}

/// Draw a bordered panel box into `buf`.
///
/// Top line: `┌── title ── count ──...──┐`
/// Sides:    `│`
/// Bottom:   `└──...──┘`
///
/// If `count` is `None`, the title fills the top bar alone.
///
/// The four passes run in a fixed order and the order is the point:
///
/// 1. the panel background, so the body sits on its own `Paint`;
/// 2. the *complete* solid fallback frame, including blank slots where the
///    title and count will go — a solid role is finished here;
/// 3. the perimeter gradient ring, which recolours whatever the frame drew;
/// 4. the title and count cells, written last so a ring never bleeds into them
///    and both keep their own independently themed `Style`.
///
/// The ring is handed exactly `area`. It takes no exclusions, so a caller must
/// never pass a rect that could overlap the remote PTY viewport.
pub fn render_panel_box(
    buf: &mut Buffer,
    area: Rect,
    title: &str,
    count: Option<&str>,
    focused: bool,
    theme: &ResolvedTheme,
    roles: PanelRoles,
) {
    if area.width < 4 || area.height < 2 {
        return;
    }

    let x = area.x;
    let y = area.y;
    let w = area.width as usize;
    let bottom = area.y + area.height - 1;
    let right_edge = x + area.width - 1;
    // A focused dashboard panel (issue #18) gets the accent border role.
    let border_role = if focused {
        roles.border_focused
    } else {
        roles.border
    };

    // ── 1. Panel background ─────────────────────────────
    crate::tui::blit::fill_paint(buf, area, theme, roles.background);

    // The title slot is measured before anything is drawn, so pass 2 can leave
    // exactly the cells pass 4 fills — the frame is complete either way.
    let reserved = 1 + count.map(|c| c.len() + 4).unwrap_or(0); // "┐" + "── c "
    let title_budget = (right_edge.saturating_sub(x + 4) as usize).saturating_sub(reserved);
    let title_text = if title_budget == 0 {
        String::new()
    } else {
        ellipsize(title, title_budget)
    };
    let title_w = title_text.chars().count() as u16;

    // ── 2. Complete solid fallback frame ────────────────
    // Build: ┌── title ── count ──...──┐
    let bstyle = Style::default().fg(crate::tui::blit::line_color(theme, border_role, area));
    buf.set_string(x, y, "┌── ", bstyle);
    let mut col = x + 4;
    let title_x = col;
    if title_w > 0 {
        buf.set_string(col, y, " ".repeat(title_w as usize), bstyle);
        col += title_w;
    }
    buf.set_string(col, y, " ", bstyle);
    col += 1;

    let mut count_x = None;
    if let Some(c) = count {
        buf.set_string(col, y, "── ", bstyle);
        col += 3;
        count_x = Some(col);
        buf.set_string(col, y, " ".repeat(c.len()), bstyle);
        col += c.len() as u16;
        buf.set_string(col, y, " ", bstyle);
        col += 1;
    }

    // Fill remaining top with ─ and close with ┐
    while col < right_edge {
        buf.set_string(col, y, "─", bstyle);
        col += 1;
    }
    buf.set_string(right_edge, y, "┐", bstyle);

    for row in (y + 1)..bottom {
        buf.set_string(x, row, "│", bstyle);
        buf.set_string(right_edge, row, "│", bstyle);
    }

    buf.set_string(x, bottom, "└", bstyle);
    for col in 1..(w - 1) {
        buf.set_string(x + col as u16, bottom, "─", bstyle);
    }
    buf.set_string(right_edge, bottom, "┘", bstyle);

    // ── 3. Gradient ring over the finished frame ────────
    if let Some(gradient) = theme.paint_gradient(border_role) {
        paint_gradient_ring(buf, area, gradient);
    }

    // ── 4. Title and count, over the ring ───────────────
    if title_w > 0 {
        buf.set_string(title_x, y, &title_text, theme.style(roles.title));
    }
    if let (Some(cx), Some(c)) = (count_x, count) {
        buf.set_string(cx, y, c, theme.style(roles.count));
    }
}

/// Selection window for a zoomed *selectable* list panel (issue #18):
/// `panel_scroll` is the selected row index. Clamp it to `[0, len)`, write it
/// back, and return `(first_visible, selected)` so the render draws
/// `items[first .. first + visible]` with `selected` highlighted and always on
/// screen (the view follows the selection).
pub(crate) fn zoom_window(app: &crate::app::App, len: usize, visible: usize) -> (usize, usize) {
    if len == 0 {
        app.panel_scroll.set(0);
        return (0, 0);
    }
    let sel = (app.panel_scroll.get() as usize).min(len - 1);
    app.panel_scroll.set(sel as u16);
    let visible = visible.max(1);
    let first = sel
        .saturating_sub(visible - 1)
        .min(len.saturating_sub(visible));
    (first, sel)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    /// Resolve a theme source in memory: parse it beside the embedded
    /// `default` it extends and run the real resolver. No filesystem, no HOME,
    /// no registry — a marker theme is just a string in the test that needs it.
    pub(crate) fn resolved_source(id: &str, source: &str) -> crate::theme::model::ResolvedTheme {
        use crate::theme::model::{ThemeDefinition, ThemeId, ThemeOrigin};
        use crate::theme::parse::parse_theme;
        use crate::theme::resolve::resolve_theme;
        use std::collections::BTreeMap;

        let root = ThemeId::parse("default").unwrap();
        let root_def = parse_theme(
            root.clone(),
            ThemeOrigin::BuiltIn,
            crate::theme::builtins::source("default").unwrap(),
        )
        .definition
        .expect("`default` parses");
        let this = ThemeId::parse(id).unwrap();
        let def = parse_theme(this.clone(), ThemeOrigin::BuiltIn, source)
            .definition
            .expect("the test theme parses");

        let mut defs: BTreeMap<ThemeId, &ThemeDefinition> = BTreeMap::new();
        defs.insert(root, &root_def);
        defs.insert(this.clone(), &def);
        let outcome = resolve_theme(&this, &defs);
        outcome
            .theme
            .unwrap_or_else(|| panic!("the test theme resolves: {:#?}", outcome.diagnostics))
    }

    /// The built-in `default`, resolved in memory.
    pub(crate) fn resolved_default() -> crate::theme::model::ResolvedTheme {
        resolved_source(
            "default",
            crate::theme::builtins::source("default").unwrap(),
        )
    }

    /// A theme whose host-list border is a perimeter ring running red → green →
    /// red, so the ring carries a spread of colours while title and count do not.
    fn resolved_gradient_theme() -> crate::theme::model::ResolvedTheme {
        resolved_source(
            "ringed",
            "schema_version = 1\nname = \"Ringed\"\nextends = \"default\"\n\n\
             [gradients.ring]\ndirection = \"perimeter\"\n\
             stops = [ { at = 0.0, color = \"#ff0000\" }, \
             { at = 0.5, color = \"#00ff00\" }, \
             { at = 1.0, color = \"#ff0000\" } ]\n\n\
             [components.dashboard.host_list]\nborder = { gradient = \"gradients.ring\" }\n",
        )
    }

    fn default_panel_roles() -> PanelRoles {
        HOST_LIST_PANEL
    }

    fn gradient_panel_roles() -> PanelRoles {
        HOST_LIST_PANEL
    }

    /// Draw one panel titled "Hosts" with the count badge "12" into a 24×6
    /// buffer whose origin is (0, 0), so a test can name absolute coordinates.
    fn render_panel_for_test(
        theme: &crate::theme::model::ResolvedTheme,
        roles: PanelRoles,
        focused: bool,
    ) -> Buffer {
        let area = Rect::new(0, 0, 24, 6);
        let mut buf = Buffer::empty(area);
        render_panel_box(&mut buf, area, "Hosts", Some("12"), focused, theme, roles);
        buf
    }

    /// What a `Style` role is *expected* to look like once written onto the
    /// panel's own background.
    ///
    /// A role's `Style` carries no background of its own under `default`
    /// (`background = "auto"`), and `Buffer::set_string` leaves a cell's
    /// existing background untouched in that case — so the cell the assertion
    /// reads back is the role composed over whatever pass 1 painted. Composing
    /// the expectation the same way makes the comparison exact instead of
    /// dropping the background from it.
    fn style_over_panel_background(
        theme: &crate::theme::model::ResolvedTheme,
        role: StyleRole,
        background: PaintRole,
        area: Rect,
    ) -> Style {
        Style::default()
            .bg(theme.paint_color_at(background, area, area.x, area.y))
            .patch(theme.style(role))
    }

    /// The style of `needle` where it sits on the panel's top border.
    ///
    /// Geometry, not a substring sweep over the frame: the top row is the only
    /// row a title or count can occupy, and the glyphs are matched cell by cell
    /// along it.
    fn style_at_text(buf: &Buffer, needle: &str) -> Style {
        for x in buf.area.left()..buf.area.right() {
            let matches = needle.chars().enumerate().all(|(i, ch)| {
                buf.cell((x + i as u16, 0))
                    .is_some_and(|c| c.symbol() == ch.to_string())
            });
            if matches {
                let cell = buf.cell((x, 0)).unwrap();
                return Style::default()
                    .fg(cell.fg)
                    .bg(cell.bg)
                    .add_modifier(cell.modifier);
            }
        }
        panic!("`{needle}` is not on the panel's top border");
    }

    /// The ring carries a spread of colours, i.e. the gradient actually ran.
    fn assert_ring_has_multiple_colors(buf: &Buffer) {
        let area = buf.area;
        let mut colors: Vec<Color> = Vec::new();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let on_ring = x == area.left()
                    || x == area.right() - 1
                    || y == area.top()
                    || y == area.bottom() - 1;
                if !on_ring {
                    continue;
                }
                let fg = buf.cell((x, y)).unwrap().fg;
                if !colors.contains(&fg) {
                    colors.push(fg);
                }
            }
        }
        assert!(
            colors.len() > 2,
            "the perimeter ring should carry a spread of colours, got {colors:?}"
        );
    }

    #[test]
    fn solid_panel_roles_preserve_title_count_and_border_styles() {
        let theme = resolved_default();
        let roles = default_panel_roles();
        let buffer = render_panel_for_test(&theme, roles, false);
        assert_eq!(
            buffer[(0, 0)].fg,
            theme.paint_color_at(roles.border, buffer.area, 0, 0)
        );
        assert_eq!(
            style_at_text(&buffer, "Hosts"),
            style_over_panel_background(&theme, roles.title, roles.background, buffer.area)
        );
        assert_eq!(
            style_at_text(&buffer, "12"),
            style_over_panel_background(&theme, roles.count, roles.background, buffer.area)
        );
    }

    #[test]
    fn perimeter_gradient_recolors_only_the_ring_not_title_or_count() {
        let theme = resolved_gradient_theme();
        let roles = gradient_panel_roles();
        let buffer = render_panel_for_test(&theme, roles, false);
        assert_ring_has_multiple_colors(&buffer);
        assert_eq!(
            style_at_text(&buffer, "Hosts"),
            style_over_panel_background(&theme, roles.title, roles.background, buffer.area)
        );
        assert_eq!(
            style_at_text(&buffer, "12"),
            style_over_panel_background(&theme, roles.count, roles.background, buffer.area)
        );
    }

    /// Default parity alone can never catch an unbound role: an unbound role
    /// and a correctly bound one both render the legacy colour under
    /// `default`. Give each role under test its own marker value instead.
    #[test]
    fn the_panel_title_count_and_background_take_their_own_marker_roles() {
        let theme = resolved_source(
            "markers",
            "schema_version = 1\nname = \"Markers\"\nextends = \"default\"\n\n\
             [components.dashboard.host_list]\n\
             title = { foreground = \"#ff00ff\" }\n\
             count = { foreground = \"#00ffff\" }\n\
             background = \"#123456\"\n",
        );
        let buffer = render_panel_for_test(&theme, HOST_LIST_PANEL, false);
        assert_eq!(
            style_at_text(&buffer, "Hosts").fg,
            Some(Color::Rgb(0xff, 0x00, 0xff))
        );
        assert_eq!(
            style_at_text(&buffer, "12").fg,
            Some(Color::Rgb(0x00, 0xff, 0xff))
        );
        assert_eq!(
            buffer[(4, 3)].bg,
            Color::Rgb(0x12, 0x34, 0x56),
            "the panel body sits on `background`"
        );
    }

    #[test]
    fn a_focused_panel_takes_the_focused_border_role() {
        let theme = resolved_source(
            "markers",
            "schema_version = 1\nname = \"Markers\"\nextends = \"default\"\n\n\
             [components.dashboard.host_list]\n\
             border = \"#111111\"\n\
             border_focused = \"#ee00ee\"\n",
        );
        let unfocused = render_panel_for_test(&theme, HOST_LIST_PANEL, false);
        let focused = render_panel_for_test(&theme, HOST_LIST_PANEL, true);
        assert_eq!(unfocused[(0, 0)].fg, Color::Rgb(0x11, 0x11, 0x11));
        assert_eq!(focused[(0, 0)].fg, Color::Rgb(0xee, 0x00, 0xee));
    }

    /// The eleven call sites must carry *eleven* bundles, not the same one
    /// pasted eleven times.
    ///
    /// Nothing about wiring `render_panel_box` forces the bundles apart: a
    /// copy-pasted `HOST_LIST_PANEL` at the latency panel compiles, renders,
    /// and is invisible under `default` — every dashboard border resolves to
    /// the same `border` semantic there. This asserts on the role identities
    /// instead, where the mistake is actually visible.
    #[test]
    fn every_panel_call_site_names_its_own_five_roles() {
        let bundles = [
            ("dashboard.host_list", HOST_LIST_PANEL),
            ("dashboard.details", DETAILS_PANEL),
            ("dashboard.ssh_log", SSH_LOG_PANEL),
            ("dashboard.agent", AGENT_PANEL),
            ("dashboard.latency", LATENCY_PANEL),
            ("dashboard.recent", RECENT_PANEL),
            ("dashboard.auth", AUTH_PANEL),
            ("dashboard.ping", PING_PANEL),
            ("sftp.panel", SFTP_PANEL),
            ("broadcast.panel", BROADCAST_PANEL),
        ];

        for (i, (a_name, a)) in bundles.iter().enumerate() {
            // Within one bundle the five roles are five different roles.
            assert_ne!(a.border, a.border_focused, "{a_name}: border == focused");
            assert_ne!(a.border, a.background, "{a_name}: border == background");
            assert_ne!(a.title, a.count, "{a_name}: title == count");

            for (b_name, b) in bundles.iter().skip(i + 1) {
                assert_ne!(a.border, b.border, "{a_name} and {b_name} share a border");
                assert_ne!(
                    a.border_focused, b.border_focused,
                    "{a_name} and {b_name} share a focused border"
                );
                assert_ne!(a.title, b.title, "{a_name} and {b_name} share a title");
                assert_ne!(a.count, b.count, "{a_name} and {b_name} share a count");
                assert_ne!(
                    a.background, b.background,
                    "{a_name} and {b_name} share a background"
                );
            }
        }
    }
}
