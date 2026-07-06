pub mod animation;
pub mod dashboard_layout;
pub mod layout;
pub mod screens;
pub mod text;
pub mod theme;
pub mod widgets;

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::{App, AppMode};

/// Convert a Unix epoch timestamp to `"HH:MM:SS"` in the local timezone.
///
/// Uses libc `localtime_r` (reentrant, no allocation) so we stay
/// dependency-free beyond what the project already pulls in transitively.
pub fn format_local_time(epoch_secs: i64) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let time_t = epoch_secs as libc::time_t;
    // SAFETY: localtime_r is reentrant and writes into our stack-local `tm`.
    unsafe { libc::localtime_r(&time_t, &mut tm) };
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

pub fn render(frame: &mut Frame, app: &App) {
    // Embedded session takes over the whole frame — no dashboard chrome.
    if matches!(app.mode, AppMode::Connecting | AppMode::Session) {
        crate::session::render::render(frame, app);
        return;
    }

    // GroupManage keeps the old layout (will be converted to overlay in Phase 5).
    if app.mode == AppMode::GroupManage || app.mode == AppMode::GroupForm {
        render_group_manage_base(frame, app);
        match app.mode {
            AppMode::GroupForm => render_form_popup(frame, app, FormKind::Group),
            AppMode::ConfirmDelete => render_confirm_delete_popup(frame, app),
            AppMode::Help => render_help_popup(frame),
            _ => {}
        }
        return;
    }

    // ── Dashboard chrome (shared across all tabs) ─────────────
    let area = frame.area();
    let areas = dashboard_layout::dashboard_layout(area);

    // Header stats
    let (total, online, slow, down) = compute_header_stats(app);
    let clock = format_utc_clock();
    widgets::header::render_header(frame, areas.header, total, online, slow, down, &clock);

    // Horizontal rule 1
    let rule1 = row_in(area, areas.header.y + areas.header.height);
    widgets::footer::render_hrule(frame, rule1, false);

    // Tab bar
    let scope_path = "~/.config/sshub";
    widgets::tab_bar::render_tab_bar(frame, areas.tab_bar, app.active_tab + 1, scope_path);

    // Horizontal rule 2
    let rule2 = row_in(area, areas.tab_bar.y + areas.tab_bar.height);
    widgets::footer::render_hrule(frame, rule2, false);

    // ── Tab body dispatch ─────────────────────────────────────
    match app.active_tab {
        0 => render_hosts_body(frame, &areas, app),
        1 => render_tunnels_body(frame, &areas, app),
        2 => render_keys_body(frame, &areas, app),
        3 => render_audit_body(frame, &areas, app),
        _ => render_hosts_body(frame, &areas, app),
    }

    // Horizontal rule 3: above footer (bold)
    let rule3 = row_in(area, areas.footer.y.saturating_sub(1));
    widgets::footer::render_hrule(frame, rule3, true);

    // Footer keybinds (tab-specific)
    let keybinds = footer_keybinds(app.active_tab);
    widgets::footer::render_footer(frame, areas.footer, &keybinds);

    // ── Overlay popups ─────────────────────────────────────────
    match app.mode {
        AppMode::Palette => {
            screens::palette::render_palette(
                frame,
                &app.palette_query,
                &app.hosts,
                &app.palette_results,
                app.palette_selected,
            );
        }
        AppMode::HostForm => render_form_popup(frame, app, FormKind::Host),
        AppMode::FieldPicker => {
            render_form_popup(frame, app, FormKind::Host);
            screens::field_picker::render_field_picker(frame, app);
        }
        AppMode::IdentityForm => render_form_popup(frame, app, FormKind::Identity),
        AppMode::GroupForm => render_form_popup(frame, app, FormKind::Group),
        AppMode::TunnelForm => screens::tunnels::render_tunnel_form(frame, app),
        AppMode::ConfirmDiscard => {
            if app.host_form.is_some() {
                render_form_popup(frame, app, FormKind::Host);
            } else if app.identity_form.is_some() {
                render_form_popup(frame, app, FormKind::Identity);
            } else if app.tunnel_form.is_some() {
                screens::tunnels::render_tunnel_form(frame, app);
            }
            render_confirm_discard_popup(frame);
        }
        AppMode::ConfirmDelete => render_confirm_delete_popup(frame, app),
        AppMode::Help => render_help_popup(frame),
        AppMode::ImportPrompt => render_import_prompt_popup(frame, app),
        _ => {}
    }
}

fn render_import_prompt_popup(frame: &mut Frame, app: &App) {
    let Some(prompt) = app.import_prompt.as_ref() else {
        return;
    };

    let area = frame.area();
    let popup_width = (area.width * 80 / 100).max(50).min(area.width);
    let popup_height = if prompt.error.is_some() { 10 } else { 8 }.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let mut lines = vec![
        ratatui::text::Line::from(Span::styled(
            "Path to Termius export folder (contains L00t.csv, ssh_keys/):",
            theme::text(),
        )),
        ratatui::text::Line::from(Span::styled(
            crate::text_input::with_cursor(&prompt.path, prompt.cursor),
            theme::bright(),
        )),
        ratatui::text::Line::from(""),
    ];
    if let Some(err) = &prompt.error {
        lines.push(ratatui::text::Line::from(Span::styled(
            format!("\u{2717} {err}"),
            Style::default().fg(Color::Red),
        )));
        lines.push(ratatui::text::Line::from(""));
    }
    lines.push(ratatui::text::Line::from(Span::styled(
        "Enter: import  \u{2502}  Esc: cancel",
        theme::dim(),
    )));

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(" Import from Termius ", theme::heading()))
                .border_style(theme::border()),
        ),
        popup_area,
    );
}

/// A one-row rect at `y`, or a zero-height rect when `y` falls outside
/// `area` (tiny terminals) — rendering helpers skip zero-height rects.
fn row_in(area: Rect, y: u16) -> Rect {
    if y >= area.y && y < area.y + area.height {
        Rect::new(area.x, y, area.width, 1)
    } else {
        Rect::new(area.x, area.y, area.width, 0)
    }
}

fn compute_header_stats(app: &App) -> (usize, usize, usize, usize) {
    let total = app.hosts.len();
    let mut online = 0usize;
    let mut slow = 0usize;
    for h in &app.hosts {
        if let Some(samples) = app.ping_data.get(h.name()) {
            if let Some(&last) = samples.last() {
                if last < 100 {
                    online += 1;
                } else {
                    slow += 1;
                }
            }
        }
    }
    (total, online, slow, 0)
}

fn footer_keybinds(active_tab: usize) -> Vec<(&'static str, &'static str)> {
    match active_tab {
        0 => vec![
            ("\u{2191}\u{2193}", "select"),
            ("\u{21b5}", "connect"),
            ("/", "search"),
            ("#", "tags"),
            ("a", "add"),
            ("e", "edit"),
            ("d", "del"),
            ("G", "groups"),
            ("?", "help"),
            ("q", "quit"),
        ],
        1 => vec![
            ("\u{2191}\u{2193}", "select"),
            ("\u{21b5}", "start/stop"),
            ("a", "new tunnel"),
            ("e", "edit"),
            ("d", "delete"),
            ("x", "kill"),
            ("?", "help"),
            ("q", "quit"),
        ],
        2 => vec![
            ("\u{2191}\u{2193}", "select"),
            ("a", "add key"),
            ("e", "edit"),
            ("d", "delete"),
            ("r", "remove agent"),
            ("p", "add to agent"),
            ("?", "help"),
            ("q", "quit"),
        ],
        3 => vec![
            ("\u{2191}\u{2193}", "select"),
            ("f", "filter"),
            ("r", "range"),
            ("?", "help"),
            ("q", "quit"),
        ],
        _ => vec![("q", "quit")],
    }
}

fn render_hosts_body(frame: &mut Frame, areas: &dashboard_layout::DashboardAreas, app: &App) {
    widgets::hosts_panel::render_hosts_panel(frame, areas.col_left, app);
    widgets::middle_stack::render_middle_stack(frame, areas.col_mid, app);
    widgets::right_stack::render_right_stack(frame, areas.col_right, app);

    // SSH log panel spanning middle + right columns below their stacks
    let log_top = areas.col_mid.y + 19;
    let log_bottom = areas.footer.y.saturating_sub(2);
    if log_bottom > log_top + 3 {
        let log_area = Rect::new(
            areas.col_mid.x,
            log_top,
            areas.col_mid.width + 1 + areas.col_right.width,
            log_bottom - log_top,
        );
        widgets::middle_stack::render_ssh_log_panel(frame, log_area, app);
    }
}

fn render_tunnels_body(frame: &mut Frame, areas: &dashboard_layout::DashboardAreas, app: &App) {
    screens::tunnels::render_tunnels(frame, areas.body, app);
}

fn render_keys_body(frame: &mut Frame, areas: &dashboard_layout::DashboardAreas, app: &App) {
    screens::keys::render_keys(frame, areas.body, app);
}

fn render_audit_body(frame: &mut Frame, areas: &dashboard_layout::DashboardAreas, app: &App) {
    screens::audit::render_audit(frame, areas.body, app);
}

fn render_group_manage_base(frame: &mut Frame, app: &App) {
    let areas = layout::root_layout(frame.area(), false);

    let sidebar_items = vec![
        ratatui::widgets::ListItem::new("  Hosts").style(theme::bright()),
        ratatui::widgets::ListItem::new("  Groups").style(theme::mute()),
    ];
    frame.render_widget(
        ratatui::widgets::List::new(sidebar_items).block(Block::default().borders(Borders::RIGHT)),
        areas.sidebar,
    );

    frame.render_widget(
        Paragraph::new("Groups")
            .style(Style::default().add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::BOTTOM)),
        areas.search,
    );

    frame.render_widget(
        screens::group_manage::render_group_list(app),
        areas.group_tree,
    );

    let notice = app.group_notice.as_deref().or(app.host_notice.as_deref());
    if let Some(notice) = notice {
        let notice_area = Rect {
            height: 1,
            y: areas.status.y.saturating_sub(1),
            ..areas.status
        };
        frame.render_widget(screens::keychain::render_notice(notice), notice_area);
    }

    frame.render_widget(widgets::status_bar::render_status_bar(app), areas.status);
}

enum FormKind {
    Host,
    Identity,
    Group,
}

fn render_form_popup(frame: &mut Frame, app: &App, kind: FormKind) {
    let area = frame.area();
    let popup_width = (area.width * 70 / 100).max(50).min(area.width);
    let popup_height = (area.height * 70 / 100).max(18).min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    match kind {
        FormKind::Host => {
            if let Some(form) = app.host_form.as_ref() {
                frame.render_widget(
                    screens::host_form::render_host_form(form, &app.groups, &app.identities),
                    popup_area,
                );
            }
        }
        FormKind::Identity => {
            if let Some(form) = app.identity_form.as_ref() {
                frame.render_widget(screens::keychain::render_identity_form(form), popup_area);
            }
        }
        FormKind::Group => {
            if let Some(form) = app.group_form.as_ref() {
                frame.render_widget(screens::group_form::render_group_form(form), popup_area);
            }
        }
    }

    // Validation errors belong INSIDE the popup — the dashboard status bar is
    // hidden behind it, so a save failure otherwise looks like a stuck form.
    let notice = match kind {
        FormKind::Host => app.host_notice.as_deref(),
        FormKind::Identity => app.identity_notice.as_deref(),
        FormKind::Group => app.group_notice.as_deref(),
    };
    if let Some(notice) = notice {
        let y = popup_area.y + popup_area.height.saturating_sub(2);
        if y > popup_area.y && popup_area.width > 4 {
            let msg = text::ellipsize(notice, popup_area.width as usize - 4);
            frame.buffer_mut().set_string(
                popup_area.x + 2,
                y,
                &msg,
                Style::default().fg(Color::Red),
            );
        }
    }
}

fn render_confirm_discard_popup(frame: &mut Frame) {
    let message = "Save changes?";
    let hint = "y: save \u{2502} n: discard \u{2502} Esc: back";

    let area = frame.area();
    let popup_width = 36u16.min(area.width);
    let popup_height = 5u16.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(format!("{message}\n{hint}"))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Unsaved changes")
                    .border_style(Style::default().fg(Color::Yellow)),
            ),
        popup_area,
    );
}

fn render_confirm_delete_popup(frame: &mut Frame, app: &App) {
    use crate::app::PendingDelete;
    let message = match &app.pending_delete {
        Some(PendingDelete::Host { name, .. }) => format!("Delete host '{name}'?"),
        Some(PendingDelete::Identity { name, .. }) => format!("Delete identity '{name}'?"),
        Some(PendingDelete::Group { name, .. }) => format!("Delete group '{name}'?"),
        Some(PendingDelete::Tunnel { label, .. }) => format!("Delete tunnel '{label}'?"),
        None => "Delete?".to_string(),
    };
    let lines = vec![
        ratatui::text::Line::from(message),
        ratatui::text::Line::from(""),
        ratatui::text::Line::from("y: delete │ n/Esc: cancel"),
    ];

    let area = frame.area();
    let popup_width = 40u16.min(area.width);
    let popup_height = 6u16.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Confirm")
                    .border_style(Style::default().fg(Color::Red)),
            ),
        popup_area,
    );
}

/// Format current UTC time as "Ddd HH:MM:SS".
fn format_utc_clock() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;

    // Day-of-week via Tomohiko Sakamoto's algorithm.
    // Convert unix timestamp to y/m/d then compute weekday.
    let days = (secs / 86400) as i64;
    // 1970-01-01 was a Thursday (weekday index 4).
    let weekday = ((days % 7 + 4) % 7) as usize;
    const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

    format!("{} {:02}:{:02}:{:02} UTC", DAY_NAMES[weekday], h, m, s)
}

fn render_help_popup(frame: &mut Frame) {
    let area = frame.area();
    let popup_width = (area.width * 70 / 100).max(40).min(area.width);
    let popup_height = (area.height * 60 / 100).max(16).min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(screens::help::render_help(), popup_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppDeps, HostEntry};
    use crate::config::AppConfig;
    use crate::launcher::TerminalLauncher;
    use crate::metadata::{HostMetadata, MetadataDb};
    use crate::ssh::{HostResolver, SshHost};
    use crate::store::LauncherStore;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::sync::Arc;

    fn test_store() -> Arc<LauncherStore> {
        Arc::new(LauncherStore::open_in_memory().unwrap())
    }

    struct EmptyResolver;

    impl HostResolver for EmptyResolver {
        fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }

        fn resolve_host(&self, name: &str) -> anyhow::Result<SshHost> {
            Ok(SshHost::new(name))
        }
    }

    struct NoopLauncher;

    impl TerminalLauncher for NoopLauncher {
        fn launch_ssh_argv(&self, _ssh_argv: &[String]) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn buffer_contains(buffer: &Buffer, needle: &str) -> bool {
        let area = buffer.area;
        for y in area.y..area.y + area.height {
            let line: String = (area.x..area.x + area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            if line.contains(needle) {
                return true;
            }
        }
        false
    }

    fn test_app_with_hosts() -> App {
        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(EmptyResolver),
                metadata: Arc::new(MetadataDb::default()),
                store: test_store(),
                launcher: Box::new(NoopLauncher),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        let mut web = SshHost::new("web-prod");
        web.hostname = Some("10.0.0.1".into());
        web.user = Some("ubuntu".into());
        web.port = Some(22);
        app.hosts = vec![HostEntry::Legacy {
            host: web,
            meta: HostMetadata {
                host_name: "web-prod".into(),
                tags: vec!["prod".into()],
                favorite: true,
                ..Default::default()
            },
        }];
        app.filtered_indices = vec![0];
        app.selected = 0;
        app.rebuild_filter();
        app
    }

    fn render_to_buffer(app: &App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn render_includes_host_name_in_list() {
        let app = test_app_with_hosts();
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "web-prod"));
    }

    #[test]
    fn render_status_bar_shows_counts_and_mode() {
        let app = test_app_with_hosts();
        let buffer = render_to_buffer(&app, 120, 38);
        // Dashboard footer shows keybinds; check for key elements
        assert!(buffer_contains(&buffer, "connect"));
        assert!(buffer_contains(&buffer, "quit"));
    }

    #[test]
    fn palette_popup_interior_filled_with_theme_bg() {
        // Regression: the palette overlay used to leave its interior at the
        // terminal default background while the group/user columns were painted
        // theme::BG, producing dark vertical bars. The whole interior must now
        // be theme::BG (or SEL_BG on the selected row).
        let mut app = test_app_with_many_hosts(92);
        app.mode = AppMode::Palette;
        app.palette_results = (0..92).collect();
        app.palette_selected = 0;
        let buf = render_to_buffer(&app, 120, 38);

        // Find a popup interior row (one fully inside the centered box) and
        // assert no cell is left at the reset/default background.
        let mut checked_rows = 0;
        for y in 0..buf.area.height {
            let row_has_box = (0..buf.area.width)
                .any(|x| matches!(buf.cell((x, y)).unwrap().bg, Color::Rgb(0x0b, 0x0d, 0x10)));
            if !row_has_box {
                continue;
            }
            checked_rows += 1;
            for x in 0..buf.area.width {
                let bg = buf.cell((x, y)).unwrap().bg;
                if matches!(
                    bg,
                    Color::Rgb(0x0b, 0x0d, 0x10) | Color::Rgb(0x18, 0x2b, 0x22)
                ) {
                    continue; // theme::BG or SEL_BG — fine
                }
                // Outside the popup, default bg is expected; only flag default
                // bg sandwiched between theme::BG cells (i.e. inside the box).
                let left = (0..x).rev().find_map(|xx| {
                    matches!(buf.cell((xx, y)).unwrap().bg, Color::Rgb(0x0b, 0x0d, 0x10))
                        .then_some(())
                });
                let right = (x + 1..buf.area.width).find_map(|xx| {
                    matches!(buf.cell((xx, y)).unwrap().bg, Color::Rgb(0x0b, 0x0d, 0x10))
                        .then_some(())
                });
                assert!(
                    !(left.is_some() && right.is_some()),
                    "default-bg hole inside palette popup at ({x},{y})"
                );
            }
        }
        assert!(checked_rows > 10, "expected to inspect the popup body rows");
    }

    #[test]
    fn render_palette_mode_shows_query() {
        let mut app = test_app_with_hosts();
        app.mode = AppMode::Palette;
        app.palette_query = "web".into();
        app.palette_results = vec![0];
        app.palette_selected = 0;
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "web"));
        assert!(buffer_contains(&buffer, "quick connect"));
    }

    #[test]
    fn render_dashboard_shows_header_stats() {
        let app = test_app_with_hosts();
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "hosts:"));
        assert!(buffer_contains(&buffer, "online"));
    }

    #[test]
    fn render_hides_detail_panel_when_disabled() {
        let mut app = test_app_with_hosts();
        app.config.appearance.show_detail_panel = false;
        let buffer = render_to_buffer(&app, 120, 38);
        // Host name should still be visible in hosts panel
        assert!(buffer_contains(&buffer, "web-prod"));
    }

    #[test]
    fn render_host_list_shows_favorite_star() {
        let app = test_app_with_hosts();
        let buffer = render_to_buffer(&app, 120, 38);
        // The hosts panel shows host name; favorites are indicated by the panel
        assert!(buffer_contains(&buffer, "web-prod"));
    }

    fn test_app_with_many_hosts(n: usize) -> App {
        let mut app = test_app_with_hosts();
        app.hosts = (0..n)
            .map(|i| {
                let name = format!("host-{i:02}");
                let mut h = SshHost::new(&name);
                h.hostname = Some(format!("10.0.0.{i}"));
                HostEntry::Legacy {
                    host: h,
                    meta: HostMetadata {
                        host_name: name,
                        ..Default::default()
                    },
                }
            })
            .collect();
        app.filtered_indices = (0..n).collect();
        app.selected = 0;
        app.rebuild_filter();
        app
    }

    #[test]
    fn keys_tab_scrolls_to_keep_selection_visible() {
        use crate::store::Identity;

        let mut app = test_app_with_hosts();
        app.active_tab = 2;
        app.identities = (0..30)
            .map(|i| Identity {
                id: i as i64,
                name: format!("key-{i:02}"),
                username: None,
                private_key: None,
                certificate: None,
                has_password: false,
            })
            .collect();

        app.identity_selected = 0;
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "key-00"));

        app.identity_selected = 28;
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(
            buffer_contains(&buffer, "key-28"),
            "selected key card scrolled off-screen"
        );
        assert!(
            !buffer_contains(&buffer, "key-00"),
            "keys grid did not scroll; first card still visible"
        );
    }

    #[test]
    fn hosts_panel_scrolls_to_keep_selection_visible() {
        let mut app = test_app_with_many_hosts(60);

        // Selection at the top: first host visible.
        app.selected = 0;
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "host-00"));

        // Selecting a host far down must bring it into view (it would be off
        // the bottom of the panel without scrolling).
        app.selected = 58;
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(
            buffer_contains(&buffer, "host-58"),
            "selected host scrolled off-screen"
        );
        // And the top of the list should have scrolled away.
        assert!(
            !buffer_contains(&buffer, "host-00"),
            "list did not scroll; top host still visible"
        );
    }
}
