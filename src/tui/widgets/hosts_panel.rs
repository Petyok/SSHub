//! Hosts panel — grouped host tree for the dashboard left column.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::Frame;

use crate::app::{App, HostEntry};
use crate::theme::catalog::{ColorRole, PaintRole, StyleRole};
use crate::theme::model::ResolvedTheme;
use crate::tui::tween;
use crate::tui::widgets::panel_box;

/// Fraction of the selected row the highlight has filled, left to right (#35).
/// `1.0` at rest, so a settled cursor draws its full bar. The wipe runs on the
/// background only: the text under it is already styled for the selection, so a
/// moved cursor lands on the right row instantly and the bar catches up.
fn highlight_fill(app: &App) -> f32 {
    if !app.motion_enabled() {
        return 1.0;
    }
    match app.selection_at {
        Some(at) => tween::ease_out(tween::progress(
            at,
            crate::tui::SELECT_ANIM,
            std::time::Instant::now(),
        )),
        None => 1.0,
    }
}

/// Restore the background of the selected row past the point the wipe has
/// reached, leaving the bar filling in from the left edge of the row.
///
/// What it restores is the panel's own resolved background, sampled per cell so
/// a gradient survives the wipe — not a blind `Color::Reset`, which would punch
/// a transparent hole through a painted panel.
fn clip_highlight(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    w: usize,
    fill: f32,
    ground: PanelGround<'_>,
) {
    if fill >= 1.0 {
        return;
    }
    let filled = (w as f32 * fill).round() as u16;
    for cx in (x + filled)..(x + w as u16) {
        let bg = ground.at(cx, y);
        if let Some(c) = buf.cell_mut((cx, y)) {
            c.bg = bg;
        }
    }
}

/// The panel background a wipe restores, sampled against the panel's own rect.
#[derive(Clone, Copy)]
struct PanelGround<'a> {
    theme: &'a ResolvedTheme,
    role: PaintRole,
    /// The panel rectangle the paint role is sampled against — a gradient must
    /// not restart inside the row being wiped.
    area: Rect,
}

impl PanelGround<'_> {
    fn at(&self, x: u16, y: u16) -> ratatui::style::Color {
        self.theme.paint_color_at(self.role, self.area, x, y)
    }
}

/// Render the hosts panel into the left column of the dashboard bento grid.
///
/// Draws a panel box with title "hosts" and the total host count, then renders
/// group headers and host rows inside the bordered area.
pub fn render_hosts_panel(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    // The background the selected row is filled with, taken from the
    // `selection.active` **role** so the per-span backgrounds on that row (the
    // dot, the star, the dim address column) follow a theme that retunes the
    // role rather than only the semantic token behind it.
    let selection_bg = theme
        .style(StyleRole::SelectionActive)
        .bg
        .unwrap_or(theme.semantic().selection_bg);
    let buf = frame.buffer_mut();

    // Total host count for the panel badge.
    let count_str = app.filtered_indices.len().to_string();
    // Surface active tag filters in the panel title so they stay visible once
    // the picker popup closes.
    let raw_title = if app.tag_filters.is_empty() {
        "hosts".to_string()
    } else {
        // Spell out the AND operator so an empty result set is self-explaining.
        let joined = app
            .tag_filters
            .iter()
            .map(|t| format!("#{t}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        format!("hosts · {joined}")
    };
    let max_title = (area.width as usize).saturating_sub(12).max(5);
    let title = crate::tui::text::ellipsize(&raw_title, max_title);
    panel_box::render_panel_box(
        buf,
        area,
        &title,
        panel_box::HOST_LIST_PANEL.with_badge(&count_str),
        app.focused_panel == crate::app::PanelId::Hosts,
        theme,
    );

    // Content area inside the panel borders: x+2, y+1, width-4, height-2
    if area.width < 6 || area.height < 4 {
        return;
    }
    let cx = area.x + 2;
    let cy = area.y + 1;
    let cw = (area.width - 4) as usize;
    let ch = (area.height - 2) as usize;

    // Reserve 2 lines at the bottom for footer (divider + add action).
    let body_h = if ch > 2 { ch - 2 } else { ch };

    // The host tree is taller than the panel once enough hosts are imported,
    // so scroll to keep the selection on screen (roughly centered). `vrow` is
    // the flattened visual-row index, counting group headers and blank
    // separators alongside host rows. The offset math lives on `App` so click
    // mapping stays in sync with what is drawn.
    // Empty state: tell a first-time user how to get hosts in.
    if app.hosts.is_empty() {
        let lines = [
            "No hosts yet.",
            "",
            "a        add a host",
            "Shift+I  import ~/.ssh/config",
            "Shift+T  import Termius export",
        ];
        for (i, line) in lines.iter().enumerate() {
            let y = cy + 1 + i as u16;
            if (y as usize) < cy as usize + ch {
                buf.set_string(
                    cx,
                    y,
                    crate::tui::text::ellipsize(line, cw),
                    theme.style(StyleRole::TextMuted),
                );
            }
        }
        return;
    }

    let offset = app.host_scroll_advance(body_h);
    let window_end = offset + body_h;

    use crate::app::VisualRow;
    let visual = app.host_visual_rows();
    let fill = highlight_fill(app);
    let ground = PanelGround {
        theme,
        role: PaintRole::DashboardHostListBackground,
        area,
    };
    let match_style = theme.style(StyleRole::DashboardHostListMatch);

    for (vrow, row) in visual.iter().enumerate() {
        if vrow < offset || vrow >= window_end {
            continue;
        }
        let y = cy + (vrow - offset) as u16;
        let selected_row = matches!(
            *row,
            VisualRow::Header { selected: true, .. } | VisualRow::Host { selected: true, .. }
        );

        match *row {
            VisualRow::Blank => {}
            VisualRow::Header {
                section,
                collapsed,
                selected,
                depth,
            } => {
                let section = &app.group_sections[section];
                let arrow = if collapsed { "\u{25b8}" } else { "\u{25be}" }; // ▸ / ▾
                let host_count = section.host_indices.len();
                let count_suffix = format!("({})", host_count);
                let label = &section.label;
                // Indent nested groups by two columns per level.
                let mut col = cx + (depth as u16) * 2;

                if selected {
                    let blank = " ".repeat(cw);
                    buf.set_string(cx, y, &blank, theme.style(StyleRole::SelectionActive));
                }
                // The group label is `host_list.group`, on the selection
                // background when the row is picked. `default` pins the role to
                // the frozen legacy colour, so parity lives in the theme rather
                // than in a bypass here.
                let group = theme.style(StyleRole::DashboardHostListGroup);
                let (arrow_style, label_style) = if selected {
                    (group.bg(selection_bg), group.bg(selection_bg))
                } else {
                    (theme.style(StyleRole::TextMuted), group)
                };
                let muted = theme.style(StyleRole::TextMuted);
                let mute_bg = if selected {
                    muted.bg(selection_bg)
                } else {
                    muted
                };

                buf.set_string(col, y, arrow, arrow_style);
                col += 2; // arrow + space

                // Right-align the count first and reserve its slot. Filling the
                // dotted leader up to (but not into) that slot means the count
                // always renders, regardless of the label length's parity.
                let count_x = cx + (cw as u16).saturating_sub(count_suffix.len() as u16);

                // Label, truncated so it can never run into the count slot
                // (leave at least one cell of gap before the count).
                let name_max = (count_x.saturating_sub(col) as usize).saturating_sub(1);
                let truncated_label: String = label.chars().take(name_max).collect();
                buf.set_string(col, y, &truncated_label, label_style);
                col += truncated_label.chars().count() as u16;

                // Dotted leader fills the gap [col, count_x).
                if count_x > col {
                    let gap = (count_x - col) as usize;
                    let dots: String = " \u{00b7}".repeat(gap.div_ceil(2));
                    let dots_trimmed: String = dots.chars().take(gap).collect();
                    buf.set_string(col, y, &dots_trimmed, mute_bg);
                }
                buf.set_string(count_x, y, &count_suffix, mute_bg);
            }
            VisualRow::Host {
                host_idx,
                selected: is_selected,
                depth,
            } => {
                let entry = &app.hosts[host_idx];

                // If selected, fill the entire row with SEL_BG.
                if is_selected {
                    let blank = " ".repeat(cw);
                    buf.set_string(cx, y, &blank, theme.style(StyleRole::SelectionActive));
                }

                // Indent hosts under their group; two cols per nesting level
                // (min one col so a flat ssh_config list still indents slightly).
                let mut col = cx + ((depth as u16) * 2).max(1);

                // Status dot — reflects ping latency.
                let host_name_for_dot = entry.name();
                let ping_samples = app.ping_data.get(host_name_for_dot).map(|v| v.as_slice());
                let (dot_char, dot_color) = match crate::ping::classify_ping(ping_samples) {
                    crate::ping::PingClass::Online => {
                        ("\u{25cf}", theme.color(ColorRole::StatusSuccess))
                    }
                    crate::ping::PingClass::Slow => {
                        let ms = ping_samples.and_then(|s| s.last().copied()).unwrap_or(0);
                        if ms <= 200 {
                            ("\u{25cf}", theme.color(ColorRole::StatusWarning))
                        } else {
                            ("\u{25cf}", theme.color(ColorRole::StatusError))
                        }
                    }
                    crate::ping::PingClass::Unreachable => {
                        ("\u{25cf}", theme.color(ColorRole::StatusError))
                    }
                    crate::ping::PingClass::Unknown => {
                        ("\u{25cb}", theme.color(ColorRole::StatusUnknown))
                    }
                };
                // Flash the dot through white when the class just changed (#35).
                let dot_color = app.ping_flash_color(host_name_for_dot, dot_color);
                let dot_style = if is_selected {
                    Style::default().fg(dot_color).bg(selection_bg)
                } else {
                    Style::default().fg(dot_color)
                };
                buf.set_string(col, y, dot_char, dot_style);
                col += 2; // dot + space

                // Base style for text on this row. The *name* is what
                // `host_list.host_selected` names, so it takes that role when
                // picked; the row fill behind it stays `selection.active`.
                let name_style = if is_selected {
                    theme.style(StyleRole::DashboardHostListHostSelected)
                } else {
                    theme.style(StyleRole::DashboardHostListHost)
                };
                let dim = theme.style(StyleRole::TextDim);
                let dim_style = if is_selected {
                    dim.bg(selection_bg)
                } else {
                    dim
                };

                let inner_right = cx + cw as u16;

                // Name — width driven by the zoom level (+/-), clamped to the panel
                // width so narrow terminals don't bleed into the border/neighbour.
                let name = entry.display_name();
                let name_w = (inner_right.saturating_sub(col) as usize).min(app.name_col_width());
                if name_w > 0 {
                    let name_display = crate::tui::text::pad_ellipsize(name, name_w);
                    buf.set_string(col, y, &name_display, name_style);
                    // Mark the characters the live search actually matched.
                    // Written over the name, so the row keeps one style and
                    // only the hits carry `host_list.match`.
                    if let Some(hits) = app.search_matches.get(&host_idx) {
                        let matched = match_style.patch(if is_selected {
                            Style::default().bg(selection_bg)
                        } else {
                            Style::default()
                        });
                        for (i, ch) in name_display.chars().enumerate() {
                            if !hits.contains(&(i as u32)) {
                                continue;
                            }
                            let x = col + i as u16;
                            if x >= inner_right {
                                break;
                            }
                            buf.set_string(x, y, ch.to_string(), matched);
                        }
                    }
                    col += name_w as u16 + 1; // + gap
                }

                // Favorite star. A fixed 2-col slot is reserved on every row so
                // addresses stay aligned whether or not the host is a favorite.
                if col < inner_right {
                    if entry.favorite() {
                        let star_style = if is_selected {
                            Style::default()
                                .fg(theme.color(ColorRole::StatusWarning))
                                .bg(selection_bg)
                        } else {
                            Style::default().fg(theme.color(ColorRole::StatusWarning))
                        };
                        buf.set_string(col, y, "\u{2605}", star_style);
                    }
                    col += 2;
                }

                // Address — up to 14 chars, only if it still fits.
                let addr = host_address(entry);
                let addr_w = (inner_right.saturating_sub(col) as usize).min(14);
                if addr_w >= 4 {
                    let addr_display = crate::tui::text::pad_ellipsize(&addr, addr_w);
                    buf.set_string(col, y, &addr_display, dim_style);
                    col += addr_w as u16 + 1; // + gap
                }

                // Ping value — right-aligned in 6 chars at the right edge.
                let ping_width: u16 = 6;
                let right_edge = cx + cw as u16;
                if right_edge >= col + ping_width {
                    let ping_x = right_edge - ping_width;
                    let host_name = entry.name();
                    let ping_samples = app.ping_data.get(host_name).map(|v| v.as_slice());
                    let (ping_str, ping_style) = match crate::ping::classify_ping(ping_samples) {
                        crate::ping::PingClass::Online | crate::ping::PingClass::Slow => {
                            let latest = ping_samples.and_then(|s| s.last().copied()).unwrap_or(0);
                            let s = format!(
                                "{:>width$}",
                                format!("{}ms", latest),
                                width = ping_width as usize
                            );
                            let style = if latest < 100 {
                                dim_style
                            } else {
                                let color = if latest <= 200 {
                                    theme.color(ColorRole::StatusWarning)
                                } else {
                                    theme.color(ColorRole::StatusError)
                                };
                                if is_selected {
                                    Style::default().fg(color).bg(selection_bg)
                                } else {
                                    Style::default().fg(color)
                                }
                            };
                            (s, style)
                        }
                        crate::ping::PingClass::Unreachable | crate::ping::PingClass::Unknown => (
                            format!("{:>width$}", "\u{2014}", width = ping_width as usize),
                            dim_style,
                        ),
                    };
                    buf.set_string(ping_x, y, &ping_str, ping_style);
                }
            }
        }

        // The highlight bar fills in from the left under a moved cursor; done
        // after the row is drawn so it clips the row's own styling too.
        if selected_row {
            clip_highlight(buf, cx, y, cw, fill, ground);
        }
    }

    // ── Footer ───────────────────────────────────────────
    if ch >= 2 {
        let footer_y = cy + (ch - 2) as u16;

        // Dotted divider line.
        let dots: String = "\u{00b7} ".repeat(cw / 2);
        let dots_trimmed: String = dots.chars().take(cw).collect();
        buf.set_string(
            cx,
            footer_y,
            &dots_trimmed,
            theme.style(StyleRole::TextMuted),
        );

        // "+ add a new host" action.
        let action = "+ add a new host";
        buf.set_string(cx, footer_y + 1, action, theme.style(StyleRole::TextDim));
    }
}

/// Extract a display address from a host entry.
fn host_address(entry: &HostEntry) -> String {
    match entry {
        HostEntry::Managed(m) => {
            if m.address.is_empty() {
                m.name.clone()
            } else {
                m.address.clone()
            }
        }
        HostEntry::Legacy { host, .. } => {
            host.hostname.clone().unwrap_or_else(|| host.name.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        assert_panel_wears, find_text, frame_at, panel_marker_theme, resolved_default,
        resolved_source, themed_app, PanelFamily, PanelProof,
    };
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    /// A wiped row over `theme`'s host-list background, returning the row's
    /// backgrounds left to right. `sel_bg` is what the bar itself is filled
    /// with, so the two halves are told apart by value.
    fn filled_row_with(theme: &ResolvedTheme, fill: f32) -> Vec<Color> {
        let area = Rect::new(0, 0, 8, 1);
        let mut buf = Buffer::empty(area);
        let sel_bg = theme.semantic().selection_bg;
        for x in 0..8 {
            buf.cell_mut((x, 0)).unwrap().bg = sel_bg;
        }
        clip_highlight(
            &mut buf,
            0,
            0,
            8,
            fill,
            PanelGround {
                theme,
                role: PaintRole::DashboardHostListBackground,
                area,
            },
        );
        (0..8).map(|x| buf.cell((x, 0)).unwrap().bg).collect()
    }

    fn filled_row(fill: f32) -> Vec<Color> {
        filled_row_with(&resolved_default(), fill)
    }

    #[test]
    fn highlight_at_rest_keeps_the_whole_bar() {
        let theme = resolved_default();
        let sel = theme.semantic().selection_bg;
        assert!(filled_row(1.0).iter().all(|c| *c == sel));
    }

    #[test]
    fn highlight_fills_from_the_left() {
        let theme = resolved_default();
        let sel = theme.semantic().selection_bg;
        let bgs = filled_row(0.5);
        assert!(
            bgs[..4].iter().all(|c| *c == sel),
            "left half stays filled: {bgs:?}"
        );
        // `default`'s `surface` is "terminal", so the wiped half goes back to
        // the unpainted ground exactly as it always did.
        assert!(
            bgs[4..].iter().all(|c| *c == Color::Reset),
            "right half is cleared: {bgs:?}"
        );
    }

    #[test]
    fn highlight_at_zero_clears_the_row() {
        assert!(filled_row(0.0).iter().all(|c| *c == Color::Reset));
    }

    /// The wipe restores the *panel's* background role, not a blind reset —
    /// only a marker theme can tell the two apart.
    #[test]
    fn the_wipe_restores_the_panel_background_role() {
        let theme = resolved_source(
            "markers",
            "schema_version = 1\nname = \"Markers\"\nextends = \"default\"\n\n\
             [components.dashboard.host_list]\nbackground = \"#123456\"\n",
        );
        let bgs = filled_row_with(&theme, 0.5);
        assert!(
            bgs[4..].iter().all(|c| *c == Color::Rgb(0x12, 0x34, 0x56)),
            "the wiped half falls back to `host_list.background`: {bgs:?}"
        );
    }

    /// Every host-list content role, each with a colour nobody else uses.
    fn host_list_marker_theme() -> ResolvedTheme {
        resolved_source(
            "markers",
            "schema_version = 1\nname = \"Markers\"\nextends = \"default\"\n\n\
             [components.dashboard.host_list]\n\
             host = { foreground = \"#ff0000\" }\n\
             host_selected = { foreground = \"#00ff00\", background = \"#004400\" }\n\
             group = { foreground = \"#0000ff\" }\n\
             match = { foreground = \"#ffff00\" }\n\n\
             [components.selection]\nactive = { foreground = \"#888888\", background = \"#222222\" }\n",
        )
    }

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 46,
        height: 14,
    };

    /// The selected host *name* takes `host_list.host_selected`.
    ///
    /// It used to take `selection.active`, which is the row fill — so an
    /// override of the role whose name promises the selected host did nothing.
    #[test]
    fn the_selected_host_name_takes_the_host_selected_role() {
        let app = themed_app(host_list_marker_theme());
        let buf = frame_at(AREA, |frame| render_hosts_panel(frame, AREA, &app));

        let (x, y) = find_text(&buf, "web-prod");
        assert_eq!(
            buf.cell((x, y)).unwrap().fg,
            Color::Rgb(0x00, 0xff, 0x00),
            "the selected host name takes `host_selected`, not `selection.active`"
        );
        assert_eq!(
            buf.cell((x, y)).unwrap().bg,
            Color::Rgb(0x00, 0x44, 0x00),
            "the name carries `host_selected`'s own background"
        );
        // The bar the name sits on is still `selection.active`: a cell at the
        // far end of the row, past everything the row writes.
        assert_eq!(
            buf.cell((AREA.width - 3, y)).unwrap().bg,
            Color::Rgb(0x22, 0x22, 0x22),
            "the row fill behind the name stays `selection.active`"
        );
    }

    /// An unselected host name takes `host_list.host`.
    #[test]
    fn an_unselected_host_name_takes_the_host_role() {
        let mut app = themed_app(host_list_marker_theme());
        // Move the cursor off the only host so it renders unselected.
        app.selected = usize::MAX;
        let buf = frame_at(AREA, |frame| render_hosts_panel(frame, AREA, &app));

        let (x, y) = find_text(&buf, "web-prod");
        assert_eq!(
            buf.cell((x, y)).unwrap().fg,
            Color::Rgb(0xff, 0x00, 0x00),
            "an unselected host name takes `host_list.host`"
        );
    }

    /// The group label takes `host_list.group`.
    ///
    /// It used to be written straight from `semantic.text_highlight`, so the
    /// published role controlled nothing. Parity now lives in `default`'s own
    /// override of the role instead.
    #[test]
    fn a_group_label_takes_the_group_role() {
        let group = crate::store::HostGroup {
            id: 1,
            name: "production".into(),
            sort_order: 0,
            default_identity_id: None,
            parent_id: None,
            reserved: false,
        };
        let mut app = themed_app(host_list_marker_theme());
        app.groups = vec![group.clone()];
        // Group membership lives on a managed host, so the fixture host is
        // swapped for one that carries the group.
        app.hosts = vec![crate::app::HostEntry::from_managed(
            crate::store::ManagedHost {
                id: 1,
                name: "web-prod".into(),
                label: None,
                address: "10.0.0.1".into(),
                port: 22,
                group_id: Some(1),
                identity_id: None,
                group: Some(group.clone()),
                groups: vec![group],
                identity: None,
                os_icon: None,
                tags: Vec::new(),
                notes: None,
                proxy_jump: None,
                forward_agent: false,
                remote_command: None,
                environment: None,
                sort_order: 0,
                favorite: false,
                last_connected: None,
                source: crate::store::HostSource::Launcher,
                ssh_config_hash: None,
                has_password: false,
                username: None,
                session_logging: crate::session_log::SessionLoggingOverride::Inherit,
                transport: Default::default(),
                created_at: 0,
                updated_at: 0,
            },
        )];
        app.rebuild_filter();

        let buf = frame_at(AREA, |frame| render_hosts_panel(frame, AREA, &app));
        let (x, y) = find_text(&buf, "production");
        assert_eq!(
            buf.cell((x, y)).unwrap().fg,
            Color::Rgb(0x00, 0x00, 0xff),
            "the group label takes `host_list.group`"
        );
    }

    /// The characters the live search matched take `host_list.match`.
    ///
    /// This role had no productive caller at all before: the list drew the name
    /// in one style and never marked what the query hit.
    #[test]
    fn the_searched_characters_take_the_match_role() {
        let mut app = themed_app(host_list_marker_theme());
        app.search_query = "prod".into();
        app.rebuild_filter();
        assert!(
            !app.search_matches.is_empty(),
            "the query must actually match, or this test proves nothing"
        );

        let buf = frame_at(AREA, |frame| render_hosts_panel(frame, AREA, &app));
        let (x, y) = find_text(&buf, "web-prod");

        // "web-prod": the query hits columns 4..8, and only those.
        for (i, ch) in "web-prod".chars().enumerate() {
            let fg = buf.cell((x + i as u16, y)).unwrap().fg;
            if i >= 4 {
                assert_eq!(
                    fg,
                    Color::Rgb(0xff, 0xff, 0x00),
                    "`{ch}` at offset {i} is a match and takes `host_list.match`"
                );
            } else {
                assert_ne!(
                    fg,
                    Color::Rgb(0xff, 0xff, 0x00),
                    "`{ch}` at offset {i} did not match and must keep the row style"
                );
            }
        }
    }

    /// The hosts panel wears `dashboard.host_list` in **both** focus states,
    /// and is the one dashboard panel whose caller really passes a badge.
    #[test]
    fn the_hosts_panel_wears_its_own_five_roles_in_both_focus_states() {
        let mut app = themed_app(panel_marker_theme());
        for focused in [false, true] {
            app.focused_panel = if focused {
                crate::app::PanelId::Hosts
            } else {
                crate::app::PanelId::Recent
            };
            let buf = frame_at(AREA, |frame| render_hosts_panel(frame, AREA, &app));
            assert_panel_wears(
                &buf,
                AREA,
                PanelProof {
                    family: PanelFamily::HostList,
                    focused,
                    title: "hosts",
                    count: Some("1"),
                    // Below the single host row and above the footer.
                    body: (2, 6),
                },
            );
        }
    }
}
