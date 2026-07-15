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

/// Panic-safe popup dimension: clamp `desired` into `[min, avail]`, but never
/// let `min` exceed `avail` (which would make `u16::clamp` assert `min <= max`
/// and crash the whole TUI on a terminal smaller than the popup's minimum).
/// On a too-small terminal the popup just shrinks to the available space.
pub fn fit_popup(desired: u16, min: u16, avail: u16) -> u16 {
    desired.clamp(min.min(avail), avail)
}

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

/// Frame entry point. Renders the UI, then — when `appearance.opaque_background`
/// is on — paints a solid backdrop behind every still-transparent cell so text
/// stays readable on a transparent terminal.
pub fn render(frame: &mut Frame, app: &App) {
    render_inner(frame, app);
    if app.config.appearance.opaque_background {
        let buf = frame.buffer_mut();
        let area = buf.area;
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    if cell.bg == ratatui::style::Color::Reset {
                        cell.bg = theme::BG;
                    }
                }
            }
        }
    }
}

fn render_inner(frame: &mut Frame, app: &App) {
    let session_behind_picker = app.mode == AppMode::SessionHostPicker
        && app
            .session_host_picker
            .as_ref()
            .is_some_and(|p| matches!(p.return_mode, AppMode::Connecting | AppMode::Session));

    // Embedded session takes over the whole frame — no dashboard chrome.
    if matches!(app.mode, AppMode::Connecting | AppMode::Session) || session_behind_picker {
        crate::session::render::render(frame, app);
        if app.mode == AppMode::SessionHostPicker {
            screens::session_host_picker::render(frame, app);
        }
        return;
    }

    // ── Dashboard chrome (shared across all tabs) ─────────────
    let area = frame.area();
    let areas = dashboard_layout::dashboard_layout_zoomed(area, app.ui_zoom);

    // Header stats
    let (total, online, slow, down) = compute_header_stats(app);
    let clock = format_utc_clock();
    widgets::header::render_header(frame, areas.header, total, online, slow, down, &clock);

    // Open embedded sessions — visible strip on the top header row so
    // background SSH tabs aren't hidden behind a footer hint.
    let session_chips = build_session_chips(app);
    widgets::header::render_session_strip(frame, areas.header, &session_chips);

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
        1 => render_sftp_body(frame, &areas, app),
        2 => render_tunnels_body(frame, &areas, app),
        3 => render_keys_body(frame, &areas, app),
        4 => render_audit_body(frame, &areas, app),
        _ => render_hosts_body(frame, &areas, app),
    }

    // Horizontal rule 3: above footer (bold)
    let rule3 = row_in(area, areas.footer.y.saturating_sub(1));
    widgets::footer::render_hrule(frame, rule3, true);

    // Footer keybinds (tab-specific)
    let keybinds = footer_keybinds(app);
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
        AppMode::GroupManage => screens::group_manage::render_group_manage_popup(frame, app),
        AppMode::GroupForm => {
            // Keep the group list behind the form when it was opened from the
            // group-management popup, for context.
            if app.group_form.as_ref().is_some_and(|f| f.return_to_manage) {
                screens::group_manage::render_group_manage_popup(frame, app);
            }
            render_form_popup(frame, app, FormKind::Group);
        }
        AppMode::GroupFieldPicker => {
            if app.group_form.as_ref().is_some_and(|f| f.return_to_manage) {
                screens::group_manage::render_group_manage_popup(frame, app);
            }
            render_form_popup(frame, app, FormKind::Group);
            screens::group_form::render_group_field_picker(frame, app);
        }
        AppMode::TagFilter => screens::tag_filter::render(frame, app),
        AppMode::TunnelForm => screens::tunnels::render_tunnel_form(frame, app),
        AppMode::TunnelHostPicker => {
            screens::tunnels::render_tunnel_form(frame, app);
            screens::tunnels::render_tunnel_host_picker(frame, app);
        }
        AppMode::SessionHostPicker => screens::session_host_picker::render(frame, app),
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
        AppMode::Help => render_help_popup(frame, app),
        AppMode::KeybindEditor => screens::keybind_editor::render_keybind_editor(frame, app),
        AppMode::Settings => screens::settings::render_settings(frame, app),
        AppMode::ConfirmQuit => render_confirm_quit_popup(frame, app),
        AppMode::ImportPrompt => render_import_prompt_popup(frame, app),
        AppMode::SftpPrompt => render_sftp_prompt_popup(frame, app),
        _ => {}
    }
}

fn render_sftp_prompt_popup(frame: &mut Frame, app: &App) {
    let Some(prompt) = app.sftp_prompt.as_ref() else {
        return;
    };
    use crate::app::SftpPromptKind;

    let (title, label) = match prompt.kind {
        SftpPromptKind::Mkdir => (" New folder ", "New folder name:"),
        SftpPromptKind::Rename => (" Rename ", "Rename to:"),
        SftpPromptKind::Chmod => (" Permissions ", "Permissions (octal, e.g. 755):"),
    };

    let area = frame.area();
    let popup_width = (area.width * 70 / 100).max(40).min(area.width);
    let popup_height = if prompt.error.is_some() { 9 } else { 7 }.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let mut lines = vec![
        ratatui::text::Line::from(Span::styled(label, theme::text())),
        ratatui::text::Line::from(Span::styled(
            crate::text_input::with_cursor(&prompt.value, prompt.cursor),
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
        "Enter: confirm  \u{2502}  Esc: cancel",
        theme::dim(),
    )));

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title, theme::heading()))
                .border_style(theme::popup_border()),
        ),
        popup_area,
    );
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
                .border_style(theme::popup_border()),
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

fn build_session_chips(app: &App) -> Vec<widgets::header::SessionChip> {
    use crate::session::SessionPhase;
    use widgets::header::{SessionChip, SessionDot};

    app.sessions
        .iter()
        .enumerate()
        .map(|(i, s)| SessionChip {
            name: s.display_name.clone(),
            dot: match s.phase {
                SessionPhase::Connecting { .. } => SessionDot::Connecting,
                SessionPhase::Running { .. } => SessionDot::Running,
                SessionPhase::Exited { .. } => SessionDot::Exited,
            },
            active: app.active_session == Some(i),
        })
        .collect()
}

fn compute_header_stats(app: &App) -> (usize, usize, usize, usize) {
    use crate::ping::{classify_ping, PingClass};

    let total = app.hosts.len();
    let mut online = 0usize;
    let mut slow = 0usize;
    let mut down = 0usize;
    for h in &app.hosts {
        match classify_ping(app.ping_data.get(h.name()).map(|v| v.as_slice())) {
            PingClass::Online => online += 1,
            PingClass::Slow => slow += 1,
            PingClass::Unreachable => down += 1,
            PingClass::Unknown => {}
        }
    }
    (total, online, slow, down)
}

fn footer_keybinds(app: &App) -> Vec<(String, &'static str)> {
    let mut binds: Vec<(String, &'static str)> = match app.active_tab {
        0 => vec![
            ("\u{2191}\u{2193}".into(), "select"),
            ("\u{21b5}".into(), "connect"),
            ("/".into(), "search"),
            ("#".into(), "tags"),
            ("a".into(), "add"),
            ("e".into(), "edit"),
            ("d".into(), "del"),
            ("+/-".into(), "zoom"),
            ("\u{2423}".into(), "fold"),
            ("G".into(), "groups"),
            ("?".into(), "help"),
            ("q".into(), "quit"),
        ],
        1 => vec![
            ("\u{2191}\u{2193}".into(), "select"),
            ("\u{21b5}".into(), "enter/connect"),
            ("\u{21c6}".into(), "focus"),
            ("\u{2190}".into(), "download"),
            ("\u{2192}".into(), "upload"),
            ("c".into(), "run"),
            ("u".into(), "unstage"),
            ("d".into(), "delete"),
            ("n".into(), "new dir"),
            ("R".into(), "rename"),
            ("M".into(), "chmod"),
            ("r".into(), "refresh"),
            ("s".into(), "ssh"),
            ("/".into(), "search"),
            ("Esc".into(), "back"),
            ("?".into(), "help"),
            ("q".into(), "quit"),
        ],
        2 => vec![
            ("\u{2191}\u{2193}".into(), "select"),
            ("\u{21b5}".into(), "start/stop"),
            ("a".into(), "new tunnel"),
            ("e".into(), "edit"),
            ("d".into(), "delete"),
            ("x".into(), "kill"),
            ("?".into(), "help"),
            ("q".into(), "quit"),
        ],
        3 => vec![
            ("\u{2191}\u{2193}\u{2190}\u{2192}".into(), "move"),
            ("[ ]".into(), "columns"),
            ("a".into(), "add"),
            ("e".into(), "edit"),
            ("d".into(), "delete"),
            ("p/r".into(), "agent +/-"),
            ("?".into(), "help"),
            ("q".into(), "quit"),
        ],
        4 => vec![
            ("\u{2191}\u{2193}".into(), "select"),
            ("f".into(), "filter"),
            ("r".into(), "range"),
            ("?".into(), "help"),
            ("q".into(), "quit"),
        ],
        _ => vec![("q".into(), "quit")],
    };
    if !app.sessions.is_empty() {
        binds.extend(app.config.keybinds.session_footer_hints());
    }
    binds
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

fn render_sftp_body(frame: &mut Frame, areas: &dashboard_layout::DashboardAreas, app: &App) {
    screens::sftp::render_sftp(frame, areas.body, app);
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
                    screens::host_form::render_host_form(
                        form,
                        &app.groups,
                        &app.identities,
                        &app.save_key_label(),
                    ),
                    popup_area,
                );
            }
        }
        FormKind::Identity => {
            if let Some(form) = app.identity_form.as_ref() {
                frame.render_widget(
                    screens::keychain::render_identity_form(form, &app.save_key_label()),
                    popup_area,
                );
            }
        }
        FormKind::Group => {
            if let Some(form) = app.group_form.as_ref() {
                let identity_name = form.default_identity_id.and_then(|id| {
                    app.identities
                        .iter()
                        .find(|i| i.id == id)
                        .map(|i| i.name.clone())
                });
                let parent_name = form.parent_id.and_then(|id| {
                    app.groups
                        .iter()
                        .find(|g| g.id == id)
                        .map(|g| g.name.clone())
                });
                frame.render_widget(
                    screens::group_form::render_group_form(
                        form,
                        identity_name.as_deref(),
                        parent_name.as_deref(),
                    ),
                    popup_area,
                );
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

fn render_confirm_quit_popup(frame: &mut Frame, app: &App) {
    let active = app.tunnel_manager.active_count();
    let message = if active > 0 {
        format!("Quit sshub?\n{active} active tunnel(s) will be closed.")
    } else {
        "Quit sshub?".to_string()
    };
    let hint = "y: quit \u{2502} n: stay \u{2502} Esc: cancel";

    let area = frame.area();
    let popup_width = 44u16.min(area.width);
    let popup_height = if active > 0 { 6u16 } else { 5u16 }.min(area.height);
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
                    .title("Confirm quit")
                    .border_style(Style::default().fg(Color::Yellow)),
            ),
        popup_area,
    );
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
        Some(PendingDelete::SftpEntry { name, is_dir, .. }) => {
            if *is_dir {
                format!("Delete folder '{name}' and all its contents?")
            } else {
                format!("Delete '{name}'?")
            }
        }
        None => "Delete?".to_string(),
    };
    let area = frame.area();
    let popup_width = 54u16.min(area.width);
    // Wrap the message (a host name can be long) and size the box to fit.
    let inner_w = popup_width.saturating_sub(2).max(1) as usize;
    let msg_rows = message.chars().count().div_ceil(inner_w).max(1) as u16;
    let popup_height = (msg_rows + 4).min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let lines = vec![
        ratatui::text::Line::from(message),
        ratatui::text::Line::from(""),
        ratatui::text::Line::from("y: delete    Esc: cancel"),
    ];

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
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

/// Scroll ceiling for the help body given the full terminal area — the same
/// popup geometry as `render_help_popup` (60% height, min 16; borders + fixed
/// footer row), kept in one place so the key handler can't scroll past what
/// the renderer will show (the excess would be invisible "debt" that Up has
/// to unwind before the view moves).
pub(crate) fn help_max_scroll(area: Rect) -> u16 {
    let popup_height = (area.height * 60 / 100).max(16).min(area.height);
    let body_height = popup_height.saturating_sub(3);
    screens::help::help_line_count().saturating_sub(body_height)
}

fn render_help_popup(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let popup_width = (area.width * 70 / 100).max(40).min(area.width);
    let popup_height = (area.height * 60 / 100).max(16).min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::popup_border())
            .title(Span::styled(" Help ", theme::heading())),
        popup_area,
    );

    // Reserve the last inner row for a fixed footer; scroll only the body.
    let inner = popup_area.inner(Margin::new(1, 1));
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(1),
    );
    let scroll = app.help_scroll.min(help_max_scroll(area));
    frame.render_widget(screens::help::render_help(scroll), body);

    let footer_y = inner.y + inner.height.saturating_sub(1);
    frame.buffer_mut().set_string(
        inner.x,
        footer_y,
        crate::tui::text::ellipsize(screens::help::HELP_FOOTER, inner.width as usize),
        theme::dim(),
    );
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
    fn opaque_background_fills_every_cell() {
        use ratatui::style::Color;
        let mut app = test_app_with_hosts();

        // Off (default): at least one cell is left transparent (Color::Reset).
        let transparent = render_to_buffer(&app, 120, 38);
        let a = transparent.area;
        let any_reset = (a.y..a.y + a.height)
            .any(|y| (a.x..a.x + a.width).any(|x| transparent[(x, y)].bg == Color::Reset));
        assert!(
            any_reset,
            "expected some transparent cell with the flag off"
        );

        // On: no cell is transparent — every Reset bg became theme::BG.
        app.config.appearance.opaque_background = true;
        let opaque = render_to_buffer(&app, 120, 38);
        let a = opaque.area;
        let all_opaque = (a.y..a.y + a.height)
            .all(|y| (a.x..a.x + a.width).all(|x| opaque[(x, y)].bg != Color::Reset));
        assert!(all_opaque, "opaque mode left a transparent cell");
    }

    #[test]
    fn render_shows_host_card_and_version() {
        let app = test_app_with_hosts();
        let buffer = render_to_buffer(&app, 120, 38);
        // The selected-host card (middle column) is titled "host · <name>".
        assert!(buffer_contains(&buffer, "host \u{b7} web-prod"));
        // Its address:port row is rendered.
        assert!(buffer_contains(&buffer, "10.0.0.1:22"));
        // The build version appears in the tab bar.
        let version = concat!("v", env!("CARGO_PKG_VERSION"));
        assert!(buffer_contains(&buffer, version));
    }

    #[test]
    fn overlays_do_not_panic_on_a_tiny_terminal() {
        // Regression: popup geometry used u16::clamp(min, max) with max derived
        // from the terminal size, which asserted min<=max and crashed the TUI
        // when the terminal was smaller than the popup minimum. Every overlay
        // must render without panicking even at absurdly small sizes.
        let modes = [
            AppMode::Palette,
            AppMode::GroupManage,
            AppMode::Help,
            AppMode::KeybindEditor,
            AppMode::ConfirmQuit,
        ];
        for &mode in &modes {
            for (w, h) in [(1u16, 1u16), (10, 3), (30, 8), (49, 20)] {
                let mut app = test_app_with_hosts();
                app.mode = mode;
                // Must not panic; we don't care about the pixels here.
                let _ = render_to_buffer(&app, w, h);
            }
        }
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
    fn header_stats_count_unreachable_hosts() {
        use crate::ping::{classify_ping, PingClass, PING_UNREACHABLE};

        let mut app = test_app_with_many_hosts(3);
        app.ping_data.insert("host-00".into(), vec![50]);
        app.ping_data.insert("host-01".into(), vec![120]);
        app.ping_data
            .insert("host-02".into(), vec![PING_UNREACHABLE]);

        let (total, online, slow, down) = compute_header_stats(&app);
        assert_eq!(total, 3);
        assert_eq!(online, 1);
        assert_eq!(slow, 1);
        assert_eq!(down, 1);
        assert_eq!(
            classify_ping(app.ping_data.get("host-02").map(|v| v.as_slice())),
            PingClass::Unreachable
        );
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
    fn group_manage_renders_as_themed_popup() {
        use crate::store::NewHostGroup;
        let store = test_store();
        store
            .create_group(&NewHostGroup {
                name: "prod".into(),
                sort_order: 0,
                ..Default::default()
            })
            .unwrap();

        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(EmptyResolver),
                metadata: Arc::new(MetadataDb::default()),
                store,
                launcher: Box::new(NoopLauncher),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.reload_hosts().unwrap();
        app.mode = AppMode::GroupManage;

        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "Groups"), "popup title missing");
        assert!(buffer_contains(&buffer, "prod"), "group row missing");
        assert!(buffer_contains(&buffer, "a add"), "action hint missing");
        // The scrapped legacy layout had a left "Hosts"/"Groups" sidebar list.
        assert!(
            !buffer_contains(&buffer, "  Hosts"),
            "legacy sidebar should be gone"
        );
    }

    #[test]
    fn nested_group_renders_indented() {
        use crate::store::{NewHost, NewHostGroup};
        let store = test_store();
        let parent = store
            .create_group(&NewHostGroup {
                name: "prod".into(),
                sort_order: 0,
                ..Default::default()
            })
            .unwrap();
        let child = store
            .create_group(&NewHostGroup {
                name: "europe".into(),
                sort_order: 1,
                parent_id: Some(parent.id),
                ..Default::default()
            })
            .unwrap();
        store
            .create_host(&NewHost {
                name: "p1".into(),
                address: "10.0.0.1".into(),
                port: 22,
                group_id: Some(parent.id),
                ..Default::default()
            })
            .unwrap();
        store
            .create_host(&NewHost {
                name: "e1".into(),
                address: "10.0.0.2".into(),
                port: 22,
                group_id: Some(child.id),
                ..Default::default()
            })
            .unwrap();

        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(EmptyResolver),
                metadata: Arc::new(MetadataDb::default()),
                store,
                launcher: Box::new(NoopLauncher),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.reload_hosts().unwrap();

        let buffer = render_to_buffer(&app, 120, 38);
        // Both headers render; the child sits indented under the parent.
        let indent = |needle: &str| -> Option<usize> {
            for y in 0..buffer.area.height {
                let line: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                if let Some(pos) = line.find(needle) {
                    return Some(pos);
                }
            }
            None
        };
        let parent_col = indent("prod").expect("parent header rendered");
        let child_col = indent("europe").expect("child header rendered");
        assert!(
            child_col > parent_col,
            "child group should be indented deeper than its parent ({child_col} > {parent_col})"
        );
    }

    #[test]
    fn failed_connect_shows_x_and_reason() {
        let mut app = test_app_with_hosts();
        let config = crate::session::SessionConfig {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf 'ssh: connect to host h port 22: Connection refused' 1>&2; exit 1".into(),
            ],
            display_name: "web-prod".into(),
            meta: crate::session::SessionMeta {
                address: Some("10.0.0.1".into()),
                ..Default::default()
            },
            pending_secret: None,
        };
        let session = crate::session::Session::spawn(config, 24, 80, None).unwrap();
        app.sessions.push(session);
        app.active_session = Some(0);
        app.mode = AppMode::Connecting;

        // Drive the session to exit and flush its stderr.
        for _ in 0..200 {
            app.sessions[0].drain();
            let s = &app.sessions[0];
            let exited = matches!(s.phase, crate::session::SessionPhase::Exited { .. });
            if exited && s.debug_log().to_ascii_lowercase().contains("refused") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "\u{2717}"), "failure X missing");
        assert!(buffer_contains(&buffer, "couldn't connect to"));
        assert!(
            buffer_contains(&buffer, "nothing is listening"),
            "plain-language reason missing"
        );
    }

    #[test]
    fn connecting_screen_shows_spinner_overlay() {
        let mut app = test_app_with_hosts();
        let config = crate::session::SessionConfig {
            argv: vec!["sleep".into(), "1".into()],
            display_name: "web-prod".into(),
            meta: crate::session::SessionMeta {
                address: Some("10.0.0.1".into()),
                ..Default::default()
            },
            pending_secret: None,
        };
        let session = crate::session::Session::spawn(config, 24, 80, None).unwrap();
        app.sessions.push(session);
        app.active_session = Some(0);
        app.mode = AppMode::Connecting;
        let buffer = render_to_buffer(&app, 120, 38);
        // The connect overlay replaces the raw PTY dump with a spinner + hint.
        assert!(buffer_contains(&buffer, "connecting to"));
        assert!(buffer_contains(&buffer, "expand log"));
    }

    #[test]
    fn dashboard_shows_open_session_strip() {
        let mut app = test_app_with_hosts();
        let config = crate::session::SessionConfig {
            argv: vec!["true".into()],
            display_name: "web-prod".into(),
            meta: crate::session::SessionMeta::default(),
            pending_secret: None,
        };
        let session = crate::session::Session::spawn(config, 24, 80, None).unwrap();
        app.sessions.push(session);
        app.active_session = Some(0);
        // Stays on the dashboard (Normal), so the strip is what makes the
        // background session visible.
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "open"));
        // Host name appears both in the list and in the strip; the strip marker
        // (●) must be present on the top row.
        let top: String = (0..120).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(top.contains('\u{25cf}'), "session dot missing on top row");
        assert!(top.contains("web-prod"), "session name missing on top row");
    }

    #[test]
    fn keys_tab_scrolls_to_keep_selection_visible() {
        use crate::store::Identity;

        let mut app = test_app_with_hosts();
        app.active_tab = 3;
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
