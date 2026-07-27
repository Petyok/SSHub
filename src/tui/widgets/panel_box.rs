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

    // ── Shared dashboard harness ─────────────────────────────
    //
    // Every panel migrated in this task is proved by *invoking its real
    // renderer* under a marker theme, not by comparing role constants. These
    // helpers are what make that cheap enough to do for all of them.

    struct NoHosts;

    impl crate::ssh::HostResolver for NoHosts {
        fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }
        fn resolve_host(&self, name: &str) -> anyhow::Result<crate::ssh::SshHost> {
            Ok(crate::ssh::SshHost::new(name))
        }
    }

    /// An app carrying one host and `theme`, built entirely in memory: no real
    /// HOME, config, keyring or on-disk database.
    pub(crate) fn themed_app(theme: crate::theme::model::ResolvedTheme) -> crate::app::App {
        use crate::app::{AppDeps, HostEntry};
        use crate::metadata::{HostMetadata, MetadataDb};
        use crate::ssh::SshHost;
        use std::sync::Arc;

        let mut app = crate::app::App::new_with_deps(
            crate::config::AppConfig::default(),
            AppDeps {
                resolver: Box::new(NoHosts),
                metadata: Arc::new(MetadataDb::default()),
                store: Arc::new(crate::store::LauncherStore::open_in_memory().unwrap()),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        let mut web = SshHost::new("web-prod");
        web.hostname = Some("10.0.0.1".into());
        web.user = Some("ubuntu".into());
        app.hosts = vec![HostEntry::Legacy {
            host: web,
            meta: HostMetadata {
                host_name: "web-prod".into(),
                tags: vec!["prod".into()],
                favorite: true,
                ..Default::default()
            },
        }];
        app.rebuild_filter();
        app.activate_resolved_theme(std::rc::Rc::new(theme));
        app
    }

    /// Render into a standalone buffer at the origin, so a test can name
    /// absolute coordinates.
    pub(crate) fn buffer_at(area: Rect, draw: impl FnOnce(&mut Buffer)) -> Buffer {
        let mut buf = Buffer::empty(area);
        draw(&mut buf);
        buf
    }

    /// Same, for the renderers that take a `Frame` rather than a `Buffer`.
    pub(crate) fn frame_at(area: Rect, draw: impl FnOnce(&mut ratatui::Frame)) -> Buffer {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal.draw(draw).unwrap();
        terminal.backend().buffer().clone()
    }

    /// The cell where `needle` starts, searched row by row.
    pub(crate) fn find_text(buf: &Buffer, needle: &str) -> (u16, u16) {
        let area = buf.area;
        (area.top()..area.bottom())
            .find_map(|y| {
                let line: String = (area.left()..area.right())
                    .map(|x| buf.cell((x, y)).unwrap().symbol())
                    .collect();
                line.find(needle)
                    .map(|b| (area.left() + line[..b].chars().count() as u16, y))
            })
            .unwrap_or_else(|| panic!("`{needle}` is not in the rendered buffer"))
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

    /// One theme giving every migrated panel a border, focused border, title,
    /// count and background nobody else uses.
    ///
    /// `border` and `border_focused` are one hex apart per family on purpose:
    /// the assertions below name the exact value, so a bundle copy-pasted from
    /// the neighbouring panel fails on the colour, not on a vague "something
    /// changed".
    fn all_panels_marker_theme() -> crate::theme::model::ResolvedTheme {
        let mut src =
            String::from("schema_version = 1\nname = \"Panels\"\nextends = \"default\"\n");
        for (i, family) in PANEL_FAMILIES.iter().enumerate() {
            let n = i as u32;
            src.push_str(&format!(
                "\n[components.{}]\n\
                 border = \"#{:02x}0000\"\n\
                 border_focused = \"#{:02x}0001\"\n\
                 title = {{ foreground = \"#{:02x}0002\" }}\n\
                 count = {{ foreground = \"#{:02x}0003\" }}\n\
                 background = \"#{:02x}0004\"\n",
                family.path,
                0x11 + n,
                0x11 + n,
                0x11 + n,
                0x11 + n,
                0x11 + n
            ));
        }
        resolved_source("panels", &src)
    }

    /// The ten role families behind the eleven call sites (broadcast is used
    /// twice), in the order their marker colours are generated.
    ///
    /// Only the TOML path is needed: what each family's five roles resolve to
    /// is proved by rendering, not by naming the constants again.
    const PANEL_FAMILIES: &[PanelFamily] = &[
        PanelFamily {
            path: "dashboard.host_list",
        },
        PanelFamily {
            path: "dashboard.details",
        },
        PanelFamily {
            path: "dashboard.ssh_log",
        },
        PanelFamily {
            path: "dashboard.agent",
        },
        PanelFamily {
            path: "dashboard.latency",
        },
        PanelFamily {
            path: "dashboard.recent",
        },
        PanelFamily {
            path: "dashboard.auth",
        },
        PanelFamily {
            path: "dashboard.ping",
        },
        PanelFamily { path: "sftp.panel" },
        PanelFamily {
            path: "broadcast.panel",
        },
    ];

    struct PanelFamily {
        path: &'static str,
    }

    fn marker(family_index: usize, slot: u8) -> Color {
        Color::Rgb(0x11 + family_index as u8, 0x00, slot)
    }

    /// Assert that whatever `render` drew into `area` carries `family`'s own
    /// border, title, count and background — at named cells, in a real buffer.
    fn assert_panel_wears(
        buf: &Buffer,
        area: Rect,
        family_index: usize,
        focused: bool,
        title: &str,
        count: Option<&str>,
        // `body` is a cell no content is drawn over. The selection bar and the
        // footer paint their own background, so which cell is free differs per
        // panel and the caller names it.
        body: (u16, u16),
    ) {
        let path = PANEL_FAMILIES[family_index].path;
        let border_slot = if focused { 1 } else { 0 };
        assert_eq!(
            buf.cell((area.x, area.y)).unwrap().fg,
            marker(family_index, border_slot),
            "{path}: top-left border corner (focused = {focused})"
        );
        let (tx, ty) = find_text(buf, title);
        assert_eq!(
            buf.cell((tx, ty)).unwrap().fg,
            marker(family_index, 2),
            "{path}: title `{title}`"
        );
        if let Some(c) = count {
            let (cx, cy) = find_text(buf, c);
            assert_eq!(
                buf.cell((cx, cy)).unwrap().fg,
                marker(family_index, 3),
                "{path}: count `{c}`"
            );
        }
        assert_eq!(
            buf.cell(body).unwrap().bg,
            marker(family_index, 4),
            "{path}: panel background at {body:?}"
        );
    }

    /// Every one of the eleven productive call sites really wears its own
    /// bundle.
    ///
    /// The predecessor of this test only compared the ten `PanelRoles`
    /// constants to each other. That can never catch a caller that passes the
    /// right bundle with the wrong *arguments* — which is exactly how the SFTP
    /// panes came to hard-code `focused = false` and pass review. Every case
    /// below therefore drives the real renderer and reads real cells.
    #[test]
    fn every_panel_call_site_wears_its_own_five_roles() {
        use crate::app::PanelId;

        let theme = all_panels_marker_theme();

        // 1 — hosts panel. The only dashboard panel with a count badge.
        let mut app = themed_app(theme.clone());
        app.focused_panel = PanelId::Hosts;
        let area = Rect::new(0, 0, 46, 12);
        let buf = frame_at(area, |frame| {
            crate::tui::widgets::hosts_panel::render_hosts_panel(frame, area, &app);
        });
        assert_panel_wears(&buf, area, 0, true, "hosts", Some("1"), (2, 6));

        // 2 — host details, unfocused so `border` (not `border_focused`) shows.
        app.focused_panel = PanelId::Recent;
        let buf = buffer_at(area, |buf| {
            crate::tui::widgets::middle_stack::render_host_panel(buf, area, &app);
        });
        assert_panel_wears(
            &buf,
            area,
            1,
            false,
            "host \u{b7}",
            None,
            (2, area.height - 2),
        );

        // 3 — SSH log.
        app.focused_panel = PanelId::SshLog;
        let buf = frame_at(area, |frame| {
            crate::tui::widgets::middle_stack::render_ssh_log_panel(frame, area, &app);
        });
        assert_panel_wears(&buf, area, 2, true, "ssh log", None, (2, area.height - 2));

        // 4 — agent.
        app.focused_panel = PanelId::Agent;
        let buf = buffer_at(area, |buf| {
            crate::tui::widgets::middle_stack::render_agent_panel(buf, area, &app);
        });
        assert_panel_wears(&buf, area, 3, true, "agent", None, (2, area.height - 2));

        // 5 — latency.
        app.focused_panel = PanelId::Latency;
        let buf = buffer_at(area, |buf| {
            crate::tui::widgets::middle_stack::render_latency_panel(buf, area, &app);
        });
        assert_panel_wears(&buf, area, 4, true, "latency", None, (2, area.height - 2));

        // 6 — recent sessions.
        app.focused_panel = PanelId::Recent;
        let buf = buffer_at(area, |buf| {
            crate::tui::widgets::right_stack::render_recent_panel(buf, area, &app);
        });
        assert_panel_wears(
            &buf,
            area,
            5,
            true,
            "recent sessions",
            None,
            (2, area.height - 2),
        );

        // 7 — auth events, unfocused.
        app.focused_panel = PanelId::Hosts;
        let buf = buffer_at(area, |buf| {
            crate::tui::widgets::right_stack::render_auth_panel(buf, area, &app);
        });
        assert_panel_wears(
            &buf,
            area,
            6,
            false,
            "auth events",
            None,
            (2, area.height - 2),
        );

        // 8 — ping all hosts.
        app.focused_panel = PanelId::Ping;
        let buf = buffer_at(area, |buf| {
            crate::tui::widgets::right_stack::render_ping_panel(buf, area, &app);
        });
        assert_panel_wears(
            &buf,
            area,
            7,
            true,
            "ping all hosts",
            None,
            (2, area.height - 2),
        );
    }

    /// The eleventh call site: both broadcast renderers, and the SFTP pane.
    ///
    /// Split out because each needs its own state on the app, not because the
    /// assertion differs.
    #[test]
    fn the_sftp_and_broadcast_call_sites_wear_their_own_five_roles() {
        use crate::sftp::model::{Focus, SftpState};

        let theme = all_panels_marker_theme();

        // 9 — the SFTP local pane, focused. `render_browser` splits the body in
        // half, so the pane is its own rect and the assertion names it.
        let area = Rect::new(0, 0, 60, 12);
        let mut state = SftpState::new("/remote", "/local");
        state.focus = Focus::Local;
        let buf = buffer_at(area, |buf| {
            crate::tui::screens::sftp::render_browser_for_test(
                buf,
                area,
                &state,
                0.0,
                1.0,
                [0, 0],
                &theme,
            );
        });
        let pane = Rect::new(area.x, area.y, area.width / 2, buf.area.height);
        assert_eq!(
            buf.cell((pane.x, pane.y)).unwrap().fg,
            marker(8, 1),
            "sftp.panel: the focused pane takes `border_focused`"
        );
        assert_eq!(
            buf.cell((pane.x + 2, pane.y + 1)).unwrap().bg,
            marker(8, 4),
            "sftp.panel: panel background"
        );
        let (tx, ty) = find_text(&buf, "local");
        assert_eq!(
            buf.cell((tx, ty)).unwrap().fg,
            marker(8, 2),
            "sftp.panel: title"
        );

        // 10 & 11 — both broadcast callers share one bundle, and both must
        // actually wear it.
        let mut app = themed_app(theme);
        app.broadcast = Some(broadcast_state());

        let area = Rect::new(0, 0, 60, 12);
        let buf = frame_at(area, |frame| {
            crate::tui::screens::broadcast::render_broadcast_panel(frame, area, &app, true);
        });
        assert_panel_wears(
            &buf,
            area,
            9,
            true,
            "cast",
            Some("0/1"),
            (2, area.height - 2),
        );

        let buf = frame_at(area, |frame| {
            crate::tui::screens::broadcast::render_broadcast_zoomed(frame, area, &app);
        });
        assert_panel_wears(
            &buf,
            area,
            9,
            true,
            "cast",
            Some("0/1"),
            (2, area.height - 2),
        );
    }

    /// A live broadcast run with one pending host, enough for the panel to draw.
    fn broadcast_state() -> crate::app::BroadcastState {
        use crate::app::BroadcastPhase;
        use crate::broadcast::BroadcastTask;
        let tasks = vec![BroadcastTask {
            host_id: 1,
            host_name: "web-prod".into(),
            argv: vec!["ssh".into(), "web-prod".into()],
            secret: None,
        }];
        let (_tx, rx) = std::sync::mpsc::channel();
        crate::app::BroadcastState {
            target_label: "group: prod".into(),
            command: "uptime".into(),
            results: crate::broadcast::seed_results(&tasks),
            rx,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            concurrency: 2,
            phase: BroadcastPhase::Running,
            anim: None,
            audit_written: false,
        }
    }
}
