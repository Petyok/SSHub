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
///
/// `count` is optional because most panels have no badge: only the host list,
/// the SFTP panes and the broadcast panel ever pass one. A family that cannot
/// show a badge publishes no `count` role at all, rather than publishing one
/// that could never reach a cell.
#[derive(Clone, Copy)]
pub struct PanelRoles {
    pub border: PaintRole,
    pub border_focused: PaintRole,
    pub title: StyleRole,
    pub count: Option<StyleRole>,
    pub background: PaintRole,
}

/// A count badge: its text and the role that styles it, inseparably.
///
/// This is one value rather than two independent options on purpose. When
/// `render_panel_box` took `count: Option<&str>` and read the role from a
/// second `Option`, the two could disagree: a badge-less family handed a text
/// reserved blank cells in the frame and truncated the title to make room for a
/// badge that was then never drawn. Pairing them makes that state
/// unrepresentable — the slot is reserved from the same value that fills it.
#[derive(Clone, Copy)]
pub struct PanelBadge<'a> {
    pub text: &'a str,
    pub role: StyleRole,
}

impl PanelRoles {
    /// The badge for `text`, or `None` when this family publishes no count
    /// role — in which case nothing is reserved and nothing is drawn.
    pub(crate) fn badge<'a>(&self, text: &'a str) -> Option<PanelBadge<'a>> {
        self.count.map(|role| PanelBadge { text, role })
    }
}

/// Declare the role bundle of a panel that carries a count badge.
macro_rules! panel_roles {
    ($name:ident, $border:ident, $focused:ident, $title:ident, $count:ident, $background:ident) => {
        pub(crate) const $name: PanelRoles = PanelRoles {
            border: PaintRole::$border,
            border_focused: PaintRole::$focused,
            title: StyleRole::$title,
            count: Some(StyleRole::$count),
            background: PaintRole::$background,
        };
    };
}

/// Declare the role bundle of a panel whose caller never supplies a badge.
macro_rules! panel_roles_no_count {
    ($name:ident, $border:ident, $focused:ident, $title:ident, $background:ident) => {
        pub(crate) const $name: PanelRoles = PanelRoles {
            border: PaintRole::$border,
            border_focused: PaintRole::$focused,
            title: StyleRole::$title,
            count: None,
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
panel_roles_no_count!(
    DETAILS_PANEL,
    DashboardDetailsBorder,
    DashboardDetailsBorderFocused,
    DashboardDetailsTitle,
    DashboardDetailsBackground
);
panel_roles_no_count!(
    SSH_LOG_PANEL,
    DashboardSshLogBorder,
    DashboardSshLogBorderFocused,
    DashboardSshLogTitle,
    DashboardSshLogBackground
);
panel_roles_no_count!(
    AGENT_PANEL,
    DashboardAgentBorder,
    DashboardAgentBorderFocused,
    DashboardAgentTitle,
    DashboardAgentBackground
);
panel_roles_no_count!(
    LATENCY_PANEL,
    DashboardLatencyBorder,
    DashboardLatencyBorderFocused,
    DashboardLatencyTitle,
    DashboardLatencyBackground
);
panel_roles_no_count!(
    RECENT_PANEL,
    DashboardRecentBorder,
    DashboardRecentBorderFocused,
    DashboardRecentTitle,
    DashboardRecentBackground
);
panel_roles_no_count!(
    AUTH_PANEL,
    DashboardAuthBorder,
    DashboardAuthBorderFocused,
    DashboardAuthTitle,
    DashboardAuthBackground
);
panel_roles_no_count!(
    PING_PANEL,
    DashboardPingBorder,
    DashboardPingBorderFocused,
    DashboardPingTitle,
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
    badge: Option<PanelBadge<'_>>,
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
    let reserved = 1 + badge.map(|b| b.text.len() + 4).unwrap_or(0); // "┐" + "── c "
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

    let mut badge_x = None;
    if let Some(b) = badge {
        buf.set_string(col, y, "── ", bstyle);
        col += 3;
        badge_x = Some(col);
        buf.set_string(col, y, " ".repeat(b.text.len()), bstyle);
        col += b.text.len() as u16;
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
    if let (Some(bx), Some(b)) = (badge_x, badge) {
        buf.set_string(bx, y, b.text, theme.style(b.role));
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
mod tests {
    use super::*;
    use crate::test_support::{resolved_default, resolved_source};
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

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
        render_panel_box(
            &mut buf,
            area,
            "Hosts",
            roles.badge("12"),
            focused,
            theme,
            roles,
        );
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
            style_over_panel_background(
                &theme,
                roles
                    .count
                    .expect("the host-list panel publishes a count role"),
                roles.background,
                buffer.area,
            )
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
            style_over_panel_background(
                &theme,
                roles
                    .count
                    .expect("the host-list panel publishes a count role"),
                roles.background,
                buffer.area,
            )
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

    /// A family that publishes no count role cannot be given a badge at all.
    ///
    /// The regression this pins: `render_panel_box` used to take the badge
    /// *text* and the badge *role* as two independent options. Handed a text
    /// for a badge-less family it reserved the slot, drew blank cells into the
    /// frame and truncated the title to make room — then drew no badge, because
    /// there was no role. Pairing text and role into one `PanelBadge` makes that
    /// half-state unrepresentable, and the proof is that the frame comes out
    /// byte-identical to passing `None`.
    #[test]
    fn a_badge_less_family_renders_exactly_as_if_no_badge_was_passed() {
        assert!(
            HOST_LIST_PANEL.count.is_some(),
            "the host list carries a badge"
        );
        for (name, roles) in [
            ("details", DETAILS_PANEL),
            ("ssh_log", SSH_LOG_PANEL),
            ("agent", AGENT_PANEL),
            ("latency", LATENCY_PANEL),
            ("recent", RECENT_PANEL),
            ("auth", AUTH_PANEL),
            ("ping", PING_PANEL),
        ] {
            assert!(
                roles.count.is_none(),
                "`{name}` never passes a badge, so it must publish no count role"
            );
            assert!(
                roles.badge("12").is_none(),
                "`{name}` cannot build a badge at all"
            );

            let theme = resolved_default();
            // Narrow enough that a wrongly reserved four-cell slot would have to
            // eat into the title.
            let area = Rect::new(0, 0, 20, 4);
            let render = |badge| {
                let mut buf = Buffer::empty(area);
                render_panel_box(
                    &mut buf,
                    area,
                    "A rather long title",
                    badge,
                    false,
                    &theme,
                    roles,
                );
                buf
            };

            let with_badge = render(roles.badge("12"));
            let without = render(None);
            assert_eq!(
                with_badge, without,
                "`{name}` must render identically with and without a badge argument"
            );

            let top: String = (area.left()..area.right())
                .map(|x| with_badge.cell((x, 0)).unwrap().symbol())
                .collect();
            assert!(
                !top.contains("12"),
                "`{name}` must not draw a badge: {top:?}"
            );
        }
    }

    /// And a family that *does* publish a count role still draws one.
    #[test]
    fn a_badge_carrying_family_draws_its_badge_from_its_own_role() {
        let theme = resolved_source(
            "markers",
            "schema_version = 1\nname = \"Markers\"\nextends = \"default\"\n\n\
             [components.dashboard.host_list]\ncount = { foreground = \"#00ffff\" }\n",
        );
        let area = Rect::new(0, 0, 24, 6);
        let mut buf = Buffer::empty(area);
        render_panel_box(
            &mut buf,
            area,
            "Hosts",
            HOST_LIST_PANEL.badge("12"),
            false,
            &theme,
            HOST_LIST_PANEL,
        );
        assert_eq!(
            style_at_text(&buf, "12").fg,
            Some(Color::Rgb(0x00, 0xff, 0xff))
        );
    }
}
