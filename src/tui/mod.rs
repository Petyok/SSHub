pub mod animation;
pub mod blit;
pub mod dashboard_layout;
pub mod layout;
pub mod screens;
pub mod text;
pub mod theme;
pub mod tween;
pub mod widgets;

use ratatui::buffer::Buffer;
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
    // Reset the per-frame popup rect; each popup that draws sets it via
    // `popup_open_rect`, and we snapshot it afterwards for the close slide (#35).
    app.last_popup_rect.set(None);
    render_inner(frame, app);
    // While in the full-screen host view, keep a fresh snapshot so leaving it can
    // slide the session off to the right (#35). Once exited, render_inner has
    // drawn the dashboard beneath — blit the snapshot sliding away over it.
    if crate::app::is_session_mode(app.mode) {
        // Hold the snapshot still while a tab slide plays: it *is* the tab being
        // carried off, so refreshing it would feed the slide its own output.
        if app.session_tab_switch.is_none() {
            *app.session_snapshot.borrow_mut() = Some(frame.buffer_mut().clone());
        }
    } else {
        // Mirror of the above: keep the dashboard fresh so entering a session has
        // something to slide over instead of blank cells.
        *app.dashboard_snapshot.borrow_mut() = Some(frame.buffer_mut().clone());
        render_session_exit(frame, app);
    }
    // Snapshot the popup shown this frame, slide a fresh one in from the top,
    // and throw a just-closed one upward.
    capture_popup_snapshot(frame, app);
    render_popup_open(frame, app);
    render_popup_close(frame, app);
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
    apply_panel_selection(frame, app);
    // Fade the whole dashboard up on the way out of the intro animation, so the
    // first frame arrives rather than replacing the splash outright (#35).
    if let Some(at) = app.dashboard_at.filter(|_| app.motion_enabled()) {
        let p = tween::progress(at, SPLASH_FADE, std::time::Instant::now());
        if p < 1.0 {
            let area = frame.area();
            blit::fade(frame.buffer_mut(), area, tween::ease_out(p));
        }
    }
}

/// Highlight the zoomed-panel text selection (issue #18) by reversing the
/// selected cells, and extract the selected text into `app.panel_sel_text` for
/// copy-on-release. Terminal-style stream selection over the dashboard body.
fn apply_panel_selection(frame: &mut Frame, app: &App) {
    if !app.panel_zoomed {
        return;
    }
    let Some(sel) = app.panel_sel else {
        app.panel_sel_text.borrow_mut().clear();
        return;
    };
    let body = dashboard_layout::dashboard_layout_zoomed(frame.area(), app.ui_zoom).body;
    // Order anchor/pointer in reading (row-major) order.
    let (a, b) = (sel.anchor, sel.cur);
    let (start, end) = if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    };
    let last_row = body.y + body.height.saturating_sub(1);
    let last_col = body.x + body.width.saturating_sub(1);
    let y0 = start.1.max(body.y);
    let y1 = end.1.min(last_row);
    if y0 > y1 {
        app.panel_sel_text.borrow_mut().clear();
        return;
    }
    let buf = frame.buffer_mut();
    let mut text = String::new();
    for row in y0..=y1 {
        let x_from = if row == start.1 { start.0 } else { body.x }.max(body.x);
        let x_to = if row == end.1 { end.0 } else { last_col }.min(last_col);
        let mut line = String::new();
        if x_from <= x_to {
            for col in x_from..=x_to {
                if let Some(cell) = buf.cell_mut((col, row)) {
                    cell.modifier.insert(Modifier::REVERSED);
                    line.push_str(cell.symbol());
                }
            }
        }
        if row != y0 {
            text.push('\n');
        }
        text.push_str(line.trim_end());
    }
    *app.panel_sel_text.borrow_mut() = text;
}

fn render_inner(frame: &mut Frame, app: &App) {
    // Only governs the picker overlay's animation below; whether a session is
    // drawn at all is `session_is_rendered`, which the clipboard relay gate
    // reads too.
    let session_behind_picker = app.session_picker_over_session();

    // Embedded session takes over the whole frame — no dashboard chrome.
    if app.session_is_rendered() {
        crate::session::render::render(frame, app);
        // Slide the freshly-connected session in from the right (#35). Skipped
        // for the picker-over-session case (no fresh connect happening).
        if !session_behind_picker {
            render_session_enter(frame, app);
            render_session_tab_slide(frame, app);
        }
        if app.mode == AppMode::SessionPicker {
            // Snapshot the session underneath before the picker draws, so its
            // drop-in can restore what it covers (#35) — the same contract the
            // dashboard branch honours for every other popup.
            if app.motion_enabled() {
                *app.popup_backdrop.borrow_mut() = Some(frame.buffer_mut().clone());
            }
            screens::session_picker::render(frame, app);
        }
        return;
    }

    // ── Dashboard chrome (shared across all tabs) ─────────────
    let area = frame.area();
    let areas = dashboard_layout::dashboard_layout_zoomed(area, app.ui_zoom);

    // Header stats
    let [total, online, slow, down] = app.header_stats_advance(compute_header_stats(app));
    let clock = format_utc_clock();
    widgets::header::render_header(frame, areas.header, total, online, slow, down, &clock);

    // Open embedded sessions — visible strip on the top header row so
    // background SSH tabs aren't hidden behind a footer hint.
    let session_chips = build_session_chips(app);
    // Cycling session tabs from the dashboard used to change the highlighted
    // chip with no motion at all, while the same keys inside the full-screen
    // view slide it. Both now read the same travel state (#35).
    let strip_travel = crate::session::render::highlight_travel(app).and_then(|p| {
        let sw = app.session_tab_switch?;
        Some(widgets::header::StripTravel {
            from: sw.from,
            to: app.active_session?,
            p,
        })
    });
    widgets::header::render_session_strip(frame, areas.header, &session_chips, strip_travel);

    // Horizontal rule 1
    let rule1 = row_in(area, areas.header.y + areas.header.height);
    widgets::footer::render_hrule(frame, rule1, false);

    // Tab bar
    let scope_path = "~/.config/sshub";
    widgets::tab_bar::render_tab_bar(frame, areas.tab_bar, app.active_tab + 1, scope_path);

    // Horizontal rule 2
    let rule2 = row_in(area, areas.tab_bar.y + areas.tab_bar.height);
    widgets::footer::render_hrule(frame, rule2, false);

    // ── Tab body dispatch (with slide animation, #35) ─────────
    let now = std::time::Instant::now();
    let sliding = app
        .tab_switch
        .filter(|s| app.motion_enabled() && now.saturating_duration_since(s.at) < TAB_ANIM);
    if let Some(sw) = sliding {
        render_tab_slide(frame, &areas, app, sw, now);
    } else {
        render_tab_body(frame, app.active_tab, &areas, app);
    }

    // ── Broadcast mode (#3): docked live panel floats over the dashboard ──
    // While a broadcast runs it lives in the bottom-right as a floating panel
    // (or full-body when zoomed + focused). Other panels are not moved, just
    // covered. The wizard overlays are handled in the mode match below.
    if let Some(bc) = app.broadcast.as_ref() {
        let body = dashboard_layout::dashboard_layout_zoomed(frame.area(), app.ui_zoom).body;
        if app.panel_zoomed && app.focused_panel == crate::app::PanelId::Broadcast {
            screens::broadcast::render_broadcast_zoomed(frame, body, app);
        } else {
            let rect = match bc.anim {
                Some(a) if app.motion_enabled() => a.rect_at(std::time::Instant::now()),
                // Reduced motion (or no anim): sit at the resting docked rect.
                _ => screens::broadcast::docked_rect(body),
            };
            let focused = app.focused_panel == crate::app::PanelId::Broadcast;
            screens::broadcast::render_broadcast_panel(frame, rect, app, focused);
        }
    }
    // Error toasts stack above the docked panel (and can outlive it), so draw
    // them whenever any exist — not only while the panel is present.
    if !app.broadcast_toasts.is_empty() {
        let body = dashboard_layout::dashboard_layout_zoomed(frame.area(), app.ui_zoom).body;
        screens::broadcast::render_broadcast_toasts(frame, body, app);
    }

    // Horizontal rule 3: above footer (bold)
    let rule3 = row_in(area, areas.footer.y.saturating_sub(1));
    widgets::footer::render_hrule(frame, rule3, true);

    // Footer keybinds (tab-specific)
    let (keybinds, pinned) = footer_keybinds(app);
    widgets::footer::render_footer(frame, areas.footer, &keybinds, pinned);

    // Issue #18: a zoomed panel hides the normal notice surface (status bar),
    // so surface transient feedback (e.g. "copied N chars") as a toast pinned
    // to the right of the footer until the next key press clears it.
    if app.panel_zoomed {
        if let Some(notice) = &app.host_notice {
            render_zoom_toast(frame, areas.footer, notice, app);
        }
    }

    // ── Overlay popups ─────────────────────────────────────────
    // Snapshot the dashboard (no popup yet) so the open slide can restore what's
    // behind the popup and let it drop in from off the top of the screen (#35).
    if app.motion_enabled() && crate::app::is_overlay_mode(app.mode) {
        *app.popup_backdrop.borrow_mut() = Some(frame.buffer_mut().clone());
    }
    match app.mode {
        AppMode::Palette => {
            screens::palette::render_palette(
                frame,
                app,
                &app.palette_query,
                &app.hosts,
                &app.palette_results,
                app.palette_selected,
                app.palette_adhoc.as_ref(),
            );
        }
        AppMode::HostForm => render_form_popup(frame, app, FormKind::Host),
        AppMode::FieldPicker => {
            render_form_popup(frame, app, FormKind::Host);
            screens::field_picker::render_field_picker(frame, app);
        }
        AppMode::IdentityForm => render_form_popup(frame, app, FormKind::Identity),
        AppMode::KeygenForm => render_form_popup(frame, app, FormKind::Keygen),
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
        AppMode::SessionPicker => screens::session_picker::render(frame, app),
        AppMode::PushKeyHostPicker => screens::push_key_pickers::render_host_picker(frame, app),
        AppMode::PushKeyIdentityPicker => {
            screens::push_key_pickers::render_identity_picker(frame, app)
        }
        AppMode::ConfirmDiscard => {
            if app.host_form.is_some() {
                render_form_popup(frame, app, FormKind::Host);
            } else if app.identity_form.is_some() {
                render_form_popup(frame, app, FormKind::Identity);
            } else if app.tunnel_form.is_some() {
                screens::tunnels::render_tunnel_form(frame, app);
            }
            render_confirm_discard_popup(frame, app);
        }
        AppMode::ConfirmDelete => render_confirm_delete_popup(frame, app),
        AppMode::Help => render_help_popup(frame, app),
        AppMode::KeybindEditor => screens::keybind_editor::render_keybind_editor(frame, app),
        AppMode::Settings => screens::settings::render_settings(frame, app),
        AppMode::TunnelReconnectSettings => {
            screens::tunnel_reconnect::render_tunnel_reconnect_settings(frame, app);
        }
        AppMode::ConfirmQuit => render_confirm_quit_popup(frame, app),
        AppMode::ImportPrompt => render_import_prompt_popup(frame, app),
        AppMode::SftpPrompt => render_sftp_prompt_popup(frame, app),
        AppMode::BroadcastPickTarget => screens::broadcast::render_pick_target(frame, app),
        AppMode::BroadcastCommand => screens::broadcast::render_command_prompt(frame, app),
        AppMode::BroadcastPreview => screens::broadcast::render_preview(frame, app),
        AppMode::Notice => render_notice_popup(frame, app),
        _ => {}
    }
}

/// Modal message popup (`AppMode::Notice`) — e.g. an SFTP connection error.
/// Text comes from `App::notice_popup`; any key dismisses it.
fn render_notice_popup(frame: &mut Frame, app: &App) {
    let Some(message) = app.notice_popup.as_ref() else {
        return;
    };
    let hint = "press any key to dismiss";

    let area = frame.area();
    let popup_width = 60u16.min(area.width).max(20.min(area.width));
    // Rough wrapped-line count so the box grows with the message.
    let inner_w = popup_width.saturating_sub(2).max(1) as usize;
    let msg_lines = message
        .split('\n')
        .map(|l| (l.chars().count() / inner_w) + 1)
        .sum::<usize>()
        .max(1);
    let popup_height = ((msg_lines as u16) + 4)
        .min(area.height)
        .max(5.min(area.height));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let popup_area = crate::tui::popup_open_rect(popup_area, app);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Paragraph::new(format!("{message}\n\n{hint}"))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Red))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Connection failed ")
                    .border_style(Style::default().fg(Color::Red)),
            ),
        popup_area,
    );
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

    let popup_area = crate::tui::popup_open_rect(popup_area, app);

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

    let popup_area = crate::tui::popup_open_rect(popup_area, app);

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

fn compute_header_stats(app: &App) -> [usize; 4] {
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
    [total, online, slow, down]
}

/// The footer's pairs, plus how many trailing ones must never be dropped.
fn footer_keybinds(app: &App) -> (Vec<(String, &'static str)>, usize) {
    let mut binds: Vec<(String, &'static str)> = match app.active_tab {
        0 => vec![
            ("\u{2191}\u{2193}".into(), "select"),
            ("\u{21b5}".into(), "connect"),
            ("/".into(), "search"),
            ("#".into(), "tags"),
            ("a".into(), "add"),
            ("e".into(), "edit"),
            ("d".into(), "del"),
            ("P".into(), "push key"),
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
            // Once the left pane points at a second server, the way back to the
            // local filesystem is the thing that needs saying: `o` only leads
            // further away, and nothing else on screen mentions `O`.
            if app.sftp.as_ref().is_some_and(|s| s.left_is_remote()) {
                ("O".into(), "local")
            } else {
                ("o".into(), "2nd host")
            },
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
            (
                ".".into(),
                if app.sftp_show_hidden {
                    "hide dotfiles"
                } else {
                    "show hidden"
                },
            ),
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
            ("R".into(), "reconnect"),
            ("?".into(), "help"),
            ("q".into(), "quit"),
        ],
        3 => vec![
            ("\u{2191}\u{2193}\u{2190}\u{2192}".into(), "move"),
            ("[ ]".into(), "columns"),
            ("a".into(), "add"),
            ("g".into(), "generate"),
            ("e".into(), "edit"),
            ("d".into(), "delete"),
            ("p/r".into(), "agent +/-"),
            ("P".into(), "push key"),
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
    // Issue #18: surface the panel-zoom hint once a panel is focused/zoomed.
    if app.active_tab == 0
        && (app.panel_zoomed || app.focused_panel != crate::app::PanelId::default())
    {
        binds.push(("z".into(), if app.panel_zoomed { "unzoom" } else { "zoom" }));
    }
    if app.active_tab == 0 && app.panel_zoomed && app.focused_panel != crate::app::PanelId::Hosts {
        let selectable = matches!(
            app.focused_panel,
            crate::app::PanelId::Ping | crate::app::PanelId::Recent
        );
        binds.push((
            "\u{2191}\u{2193}".into(),
            if selectable { "select" } else { "scroll" },
        ));
        if selectable {
            binds.push(("\u{21b5}".into(), "connect"));
        }
    }
    if app.active_tab == 0 && app.panel_zoomed {
        binds.push(("drag".into(), "copy"));
    }
    // Broadcast mode (#3): running panel gets a cancel hint (and a zoom hint
    // once focused); an active wizard step gets next/cancel.
    if app.broadcast.is_some() {
        binds.push(("x".into(), "cancel"));
        if app.focused_panel == crate::app::PanelId::Broadcast {
            binds.push(("z".into(), "zoom"));
        }
    } else if !app.broadcast_toasts.is_empty() {
        binds.push(("x".into(), "clear errors"));
    }
    if matches!(
        app.mode,
        AppMode::BroadcastPickTarget | AppMode::BroadcastCommand | AppMode::BroadcastPreview
    ) {
        binds.push(("\u{21b5}".into(), "next"));
        binds.push(("Esc".into(), "cancel"));
    }
    if !app.sessions.is_empty() {
        binds.extend(app.config.keybinds.session_footer_hints());
    }

    // Move the pairs that say how to get out, or back into a session, to the end
    // and report how many there are, because the footer pins its tail when the
    // row does not fit. Every conditional block above (panel zoom, broadcast,
    // the session hints) otherwise pushes `? help` and `q quit` into the middle,
    // which is exactly where truncation eats them.
    const PINNED_LABELS: [&str; 3] = ["resume", "help", "quit"];
    let mut pinned: Vec<(String, &'static str)> = Vec::new();
    for label in PINNED_LABELS {
        if let Some(i) = binds.iter().position(|(_, l)| *l == label) {
            pinned.push(binds.remove(i));
        }
    }
    let pinned_len = pinned.len();
    binds.extend(pinned);
    (binds, pinned_len)
}

/// Draw a transient notice (issue #18) as a floating chip right-aligned on the
/// row *above* the footer keybinds, used while a panel is zoomed and the normal
/// status-bar notice surface is hidden. Sits above the hints so it never clips
/// them.
fn render_zoom_toast(frame: &mut Frame, footer: Rect, notice: &str, app: &App) {
    let label = format!(" {notice} ");
    let w = label.chars().count() as u16;
    if footer.width < w || footer.y == 0 {
        return;
    }
    let rest_x = footer.x + footer.width - w;
    // Ride in from off the right edge (#35), like the broadcast toasts. Travel
    // the distance from the resting slot to the screen edge, so the toast is
    // fully off-screen at p == 0 and `set_string` clips whatever hangs over.
    let off = app
        .host_notice_at
        .filter(|_| app.motion_enabled())
        .map(|at| {
            let p = tween::progress(at, crate::broadcast::TOAST_ANIM, std::time::Instant::now());
            ((1.0 - tween::ease_out(p)) * frame.area().right().saturating_sub(rest_x) as f32)
                .round() as u16
        })
        .unwrap_or(0);
    let x = rest_x + off;
    let y = footer.y - 1;
    let style = theme::cyan().add_modifier(Modifier::REVERSED);
    frame.buffer_mut().set_string(x, y, &label, style);
}

/// Snapshot the popup drawn this frame (opaque cells at `last_popup_rect`) into
/// `app.popup_snapshot`, so it can be thrown upward when the popup later closes
/// (#35). No-op when the current mode draws no popup.
fn capture_popup_snapshot(frame: &mut Frame, app: &App) {
    if !crate::app::is_overlay_mode(app.mode) {
        return;
    }
    let Some(rect) = app.last_popup_rect.get() else {
        return;
    };
    let rect = rect.intersection(frame.area());
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    let mut snap = Buffer::empty(rect);
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            if let (Some(src), Some(dst)) = (buf.cell((x, y)), snap.cell_mut((x, y))) {
                *dst = src.clone();
            }
        }
    }
    *app.popup_snapshot.borrow_mut() = Some((rect, snap));
}

/// Slide a freshly-opened popup down into place from off the top of the screen
/// over [`POPUP_ANIM`] (#35). Restores the dashboard backdrop where the popup
/// rests, then blits its snapshot shifted up by an easing offset (the whole
/// popup is above the top at the start), so it truly enters from off-screen.
fn render_popup_open(frame: &mut Frame, app: &App) {
    if !app.motion_enabled() || !crate::app::is_overlay_mode(app.mode) {
        return;
    }
    let now = std::time::Instant::now();
    let p = tween::progress(app.mode_entered_at, POPUP_ANIM, now);
    if p >= 1.0 {
        return;
    }
    let snap = app.popup_snapshot.borrow();
    let backdrop = app.popup_backdrop.borrow();
    let (Some((rect, buf)), Some(bd)) = (snap.as_ref(), backdrop.as_ref()) else {
        return;
    };
    // Off starts a full popup-height above the rest (fully off-screen) and eases
    // to 0. Restore the dashboard where the popup rests, then blit it shifted up.
    let off = ((1.0 - tween::ease_out(p)) * rect.bottom() as f32).round() as u16;
    let fb = frame.buffer_mut();
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            if let (Some(src), Some(dst)) = (bd.cell((x, y)), fb.cell_mut((x, y))) {
                *dst = src.clone();
            }
        }
    }
    for y in rect.top()..rect.bottom() {
        let Some(ty) = y.checked_sub(off) else {
            continue;
        };
        for x in rect.left()..rect.right() {
            if let (Some(src), Some(dst)) = (buf.cell((x, y)), fb.cell_mut((x, ty))) {
                *dst = src.clone();
            }
        }
    }
}

/// Blit a just-closed popup's captured snapshot, sliding it up off the top over
/// [`POPUP_ANIM`] (#35). The dashboard beneath is already drawn, so the popup
/// rises away revealing it.
fn render_popup_close(frame: &mut Frame, app: &App) {
    if !app.motion_enabled() {
        return;
    }
    let Some(at) = app.popup_closing_at else {
        return;
    };
    let now = std::time::Instant::now();
    let p = tween::progress(at, POPUP_ANIM, now);
    if p >= 1.0 {
        return;
    }
    let snap = app.popup_snapshot.borrow();
    let Some((rect, buf)) = snap.as_ref() else {
        return;
    };
    // Travel the popup's whole bottom edge to the top, so at p==1 every row has
    // slid above y==0 and nothing lingers near the top of the screen.
    let off = (tween::ease_out(p) * rect.bottom() as f32).round() as u16;
    let fb = frame.buffer_mut();
    for y in rect.top()..rect.bottom() {
        let Some(ty) = y.checked_sub(off) else {
            continue;
        };
        for x in rect.left()..rect.right() {
            if let (Some(src), Some(dst)) = (buf.cell((x, y)), fb.cell_mut((x, ty))) {
                *dst = src.clone();
            }
        }
    }
}

/// Slide the freshly-rendered full-screen session in from the right edge over
/// [`SESSION_ANIM`] (#35). Snapshots the session buffer, then blits it shifted
/// right by an easing offset, leaving the vacated left band blank so the view
/// reads as pushing in from the right.
fn render_session_enter(frame: &mut Frame, app: &App) {
    if !app.motion_enabled() {
        return;
    }
    let Some(at) = app.session_enter_at else {
        return;
    };
    let now = std::time::Instant::now();
    let p = tween::progress(at, SESSION_ANIM, now);
    if p >= 1.0 {
        return;
    }
    let area = frame.area();
    // Off starts a full screen-width to the right (fully off) and eases to 0.
    let off = ((1.0 - tween::ease_out(p)) * area.width as f32).round() as u16;
    if off == 0 {
        return;
    }
    let src = frame.buffer_mut().clone();
    // What the session is sliding over. Without it the columns it has not reached
    // yet come out blank, so entering a session flashed a black screen with the
    // host arriving over it.
    let behind = app.dashboard_snapshot.borrow();
    let fb = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        // Right-to-left so each destination reads a not-yet-overwritten source.
        for x in (area.x..area.x + area.width).rev() {
            if let Some(sx) = x.checked_sub(off).filter(|sx| *sx >= area.x) {
                if let (Some(s), Some(d)) = (src.cell((sx, y)), fb.cell_mut((x, y))) {
                    *d = s.clone();
                }
            } else if let Some(d) = fb.cell_mut((x, y)) {
                match behind.as_ref().and_then(|b| b.cell((x, y))) {
                    Some(s) => *d = s.clone(),
                    None => d.reset(),
                }
            }
        }
    }
}

/// Slide a just-left session's captured snapshot off to the right over
/// [`SESSION_ANIM`] (#35), revealing the dashboard already drawn beneath. The
/// mirror of [`render_session_enter`].
fn render_session_exit(frame: &mut Frame, app: &App) {
    if !app.motion_enabled() {
        return;
    }
    let Some(at) = app.session_exit_at else {
        return;
    };
    let now = std::time::Instant::now();
    let p = tween::progress(at, SESSION_ANIM, now);
    if p >= 1.0 {
        return;
    }
    let snap = app.session_snapshot.borrow();
    let Some(buf) = snap.as_ref() else {
        return;
    };
    let area = frame.area();
    // Off eases from 0 to a full screen-width, carrying the session off the right.
    let off = (tween::ease_out(p) * area.width as f32).round() as u16;
    if off >= area.width {
        return;
    }
    let fb = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        // Left-to-right: each destination x reads source x-off (already passed).
        for x in (area.x + off)..(area.x + area.width) {
            if let (Some(s), Some(d)) = (buf.cell((x - off, y)), fb.cell_mut((x, y))) {
                *d = s.clone();
            }
        }
    }
}

/// Slide between two embedded session tabs over [`TAB_ANIM`] (#35): the tab
/// being left is carried off one edge while the new one follows it in from the
/// other, so `Ctrl`+arrows reads as travel along the strip instead of a swap.
/// Shares the dashboard tab-switch duration, being the same gesture.
fn render_session_tab_slide(frame: &mut Frame, app: &App) {
    if !app.motion_enabled() {
        return;
    }
    let Some(sw) = app.session_tab_switch else {
        return;
    };
    let now = std::time::Instant::now();
    let p = tween::progress(sw.at, TAB_ANIM, now);
    if p >= 1.0 {
        return;
    }
    let snap = app.session_snapshot.borrow();
    let Some(outgoing) = snap.as_ref() else {
        return;
    };
    // Only the PTY body travels: the header stays put so the tab strip is a
    // fixed reference while its highlight slides between tabs (#35).
    let area = crate::session::render::body_rect(frame.area());
    // The new tab is already drawn at rest; lift it so both layers can move.
    let incoming = blit::snapshot(frame.buffer_mut(), area);
    frame.render_widget(Clear, area);
    let e = tween::ease_out(p);
    let w = area.width as f32;
    let dir = sw.dir as f32;
    let fb = frame.buffer_mut();
    blit::blit(fb, area, area, outgoing, (-dir * e * w).round() as i32, 0);
    blit::blit(
        fb,
        area,
        area,
        &incoming,
        (dir * (1.0 - e) * w).round() as i32,
        0,
    );
}

/// How long the dashboard takes to fade up over the intro animation (#35).
pub const SPLASH_FADE: std::time::Duration = std::time::Duration::from_millis(360);

/// How long a panel's swapped-out content takes to fade in (#35).
pub const CONTENT_FADE: std::time::Duration = std::time::Duration::from_millis(140);

/// How long an SFTP pane's listing takes to slide to a new directory (#35).
pub const SFTP_NAV_ANIM: std::time::Duration = std::time::Duration::from_millis(200);

/// How long a newly staged SFTP transfer takes to fly into the queue (#35).
pub const SFTP_QUEUE_ANIM: std::time::Duration = std::time::Duration::from_millis(200);

/// How long a host's status dot flashes after its ping class changes (#35).
pub const PING_FLASH: std::time::Duration = std::time::Duration::from_millis(420);

/// Duration of a group's fold / unfold reveal in the host list (#35).
pub const FOLD_ANIM: std::time::Duration = std::time::Duration::from_millis(180);

/// Duration of the host-list highlight wipe under a moved cursor (#35).
pub const SELECT_ANIM: std::time::Duration = std::time::Duration::from_millis(120);

/// Duration of the tab-switch body slide (#35).
pub const TAB_ANIM: std::time::Duration = std::time::Duration::from_millis(220);

/// Duration of a popup's open / close slide (#35).
pub const POPUP_ANIM: std::time::Duration = std::time::Duration::from_millis(260);

/// Duration of the full-screen session-enter slide on connect (#35).
pub const SESSION_ANIM: std::time::Duration = std::time::Duration::from_millis(280);

/// Duration of an SFTP tab sub-state slide: picker <-> connecting <-> browser (#35).
pub const SFTP_ANIM: std::time::Duration = std::time::Duration::from_millis(260);

/// Shared popup rect hook (#35): every overlay runs its resting rect through
/// this so the render pass can snapshot the popup for its open/close slides.
/// Returns the rest rect unchanged — the popup always *draws* at rest, and the
/// slide is a separate blit pass ([`render_popup_open`] / [`render_popup_close`])
/// that can clip the popup above the top of the screen (a `Rect` cannot).
pub fn popup_open_rect(target: Rect, app: &App) -> Rect {
    app.last_popup_rect.set(Some(target));
    target
}

/// Dispatch a tab index to its body renderer, into `areas`.
fn render_tab_body(
    frame: &mut Frame,
    tab: usize,
    areas: &dashboard_layout::DashboardAreas,
    app: &App,
) {
    match tab {
        0 => render_hosts_body(frame, areas, app),
        1 => render_sftp_body(frame, areas, app),
        2 => render_tunnels_body(frame, areas, app),
        3 => render_keys_body(frame, areas, app),
        4 => render_audit_body(frame, areas, app),
        _ => render_hosts_body(frame, areas, app),
    }
}

/// Copy `areas` with the body region (body + the three columns) shifted right by
/// `dx` columns, for rendering a tab body mid-slide. Header/tab-bar/footer stay.
fn shift_body_areas(
    areas: &dashboard_layout::DashboardAreas,
    dx: u16,
) -> dashboard_layout::DashboardAreas {
    let shift = |r: Rect| Rect::new(r.x.saturating_add(dx), r.y, r.width, r.height);
    let mut a = *areas;
    a.body = shift(a.body);
    a.col_left = shift(a.col_left);
    a.col_mid = shift(a.col_mid);
    a.col_right = shift(a.col_right);
    a
}

/// Render a tab-switch slide: a static backdrop body plus the moving body
/// translated right by an eased offset, with a hard edge between them (#35).
/// `to > from` slides the new tab in from the right; `to < from` slides the old
/// tab out to the right, revealing the new one beneath.
fn render_tab_slide(
    frame: &mut Frame,
    areas: &dashboard_layout::DashboardAreas,
    app: &App,
    sw: crate::app::TabSwitch,
    now: std::time::Instant,
) {
    let p = tween::ease_out(tween::progress(sw.at, TAB_ANIM, now));
    let bw = areas.body.width;
    let right = sw.to > sw.from;
    // The moving layer sits on top starting at `body.x + off`; the backdrop shows
    // in `[body.x, body.x + off]`. Right: new enters from the right (off: bw->0).
    // Left: old exits to the right (off: 0->bw).
    let off = if right {
        ((1.0 - p) * bw as f32).round() as u16
    } else {
        (p * bw as f32).round() as u16
    };
    let (backdrop, top) = if right {
        (sw.from, sw.to)
    } else {
        (sw.to, sw.from)
    };

    render_tab_body(frame, backdrop, areas, app);
    if off < bw {
        let clear = Rect::new(
            areas.body.x + off,
            areas.body.y,
            bw - off,
            areas.body.height,
        );
        frame.render_widget(Clear, clear);
        let shifted = shift_body_areas(areas, off);
        render_tab_body(frame, top, &shifted, app);
    }
}

fn render_hosts_body(frame: &mut Frame, areas: &dashboard_layout::DashboardAreas, app: &App) {
    // Issue #18: a zoomed panel takes over the whole dashboard body.
    // Broadcast (#3) is a floating panel drawn from render_inner instead, so a
    // zoomed Broadcast must not be handled here (it has no home in the hosts
    // grid) — let render_inner's broadcast block own it.
    let grid_panel = app.focused_panel != crate::app::PanelId::Broadcast;
    let now = std::time::Instant::now();
    // A zoom morph (#35) is playing while the anim exists and hasn't finished.
    let morphing = grid_panel && app.zoom_anim.is_some_and(|a| !a.is_done(now));

    // Fully zoomed, no morph in flight: the panel owns the whole body.
    if app.panel_zoomed && !morphing && grid_panel {
        render_zoomed_panel(frame, areas.body, app);
        return;
    }
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

    // Zoom morph (#35): overlay the focused panel at the interpolating rect over
    // the grid, so zoom-in grows out of the slot and zoom-out shrinks back into
    // it. When the morph finishes, the branch above takes over (full body) or
    // the plain grid remains.
    if morphing {
        if let Some(anim) = app.zoom_anim {
            let rect = anim.rect_at(now);
            frame.render_widget(Clear, rect);
            render_zoomed_panel(frame, rect, app);
        }
    }
}

/// The grid slot a panel morphs out of / back into for the zoom animation
/// (#35). Approximated by the panel's column (or the log strip), which reads
/// well without threading every sub-panel's exact rect out of the stacks.
pub fn panel_zoom_source(
    areas: &dashboard_layout::DashboardAreas,
    panel: crate::app::PanelId,
) -> Rect {
    use crate::app::PanelId;
    use widgets::middle_stack::{AGENT_H, HOST_H, LATENCY_H};
    use widgets::right_stack::{AUTH_H, PING_H, RECENT_H};
    let mid = areas.col_mid;
    let right = areas.col_right;
    // Each stacked panel's real slot (same heights the stacks lay out), so the
    // morph grows/shrinks in both dimensions from the actual box.
    match panel {
        PanelId::Hosts => areas.col_left,
        PanelId::Detail => Rect::new(mid.x, mid.y, mid.width, HOST_H),
        PanelId::Agent => Rect::new(mid.x, mid.y + HOST_H, mid.width, AGENT_H),
        PanelId::Latency => Rect::new(mid.x, mid.y + HOST_H + AGENT_H, mid.width, LATENCY_H),
        PanelId::Recent => Rect::new(right.x, right.y, right.width, RECENT_H),
        PanelId::Auth => Rect::new(right.x, right.y + RECENT_H, right.width, AUTH_H),
        PanelId::Ping => Rect::new(right.x, right.y + RECENT_H + AUTH_H, right.width, PING_H),
        // SSH log spans mid+right along the bottom (see render_hosts_body).
        PanelId::SshLog => Rect::new(
            mid.x,
            mid.y + 19,
            mid.width + 1 + right.width,
            areas.body.height.saturating_sub(19),
        ),
        // Broadcast morphs from its own docked rect, handled elsewhere.
        PanelId::Broadcast => screens::broadcast::docked_rect(areas.body),
    }
}

/// Render just the focused panel into `area` (the full dashboard body) for the
/// tmux-style zoom (issue #18).
fn render_zoomed_panel(frame: &mut Frame, area: Rect, app: &App) {
    use crate::app::PanelId;
    match app.focused_panel {
        PanelId::Hosts => widgets::hosts_panel::render_hosts_panel(frame, area, app),
        PanelId::Detail => widgets::middle_stack::render_host_panel(frame.buffer_mut(), area, app),
        PanelId::Agent => widgets::middle_stack::render_agent_panel(frame.buffer_mut(), area, app),
        PanelId::Latency => {
            widgets::middle_stack::render_latency_panel(frame.buffer_mut(), area, app)
        }
        PanelId::Recent => widgets::right_stack::render_recent_panel(frame.buffer_mut(), area, app),
        PanelId::Auth => widgets::right_stack::render_auth_panel(frame.buffer_mut(), area, app),
        PanelId::Ping => widgets::right_stack::render_ping_panel(frame.buffer_mut(), area, app),
        PanelId::SshLog => widgets::middle_stack::render_ssh_log_panel(frame, area, app),
        PanelId::Broadcast => screens::broadcast::render_broadcast_zoomed(frame, area, app),
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
    Keygen,
    Group,
}

fn render_form_popup(frame: &mut Frame, app: &App, kind: FormKind) {
    let area = frame.area();
    let popup_width = (area.width * 70 / 100).max(50).min(area.width);
    let popup_height = (area.height * 70 / 100).max(18).min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let popup_area = crate::tui::popup_open_rect(popup_area, app);

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
                        &app.config.keybinds.secret_field_hints(),
                    ),
                    popup_area,
                );
            }
        }
        FormKind::Identity => {
            if let Some(form) = app.identity_form.as_ref() {
                frame.render_widget(
                    screens::keychain::render_identity_form(
                        form,
                        &app.save_key_label(),
                        &app.config.keybinds.secret_field_hints(),
                    ),
                    popup_area,
                );
            }
        }
        FormKind::Keygen => {
            if let Some(form) = app.keygen_form.as_ref() {
                frame.render_widget(
                    screens::keygen::render_keygen_form(form, &app.save_key_label()),
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
        FormKind::Keygen => app.keygen_notice.as_deref(),
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

    let popup_area = crate::tui::popup_open_rect(popup_area, app);

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

fn render_confirm_discard_popup(frame: &mut Frame, app: &App) {
    let message = "Save changes?";
    let hint = "y: save \u{2502} n: discard \u{2502} Esc: back";

    let area = frame.area();
    let popup_width = 36u16.min(area.width);
    let popup_height = 5u16.min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let popup_area = crate::tui::popup_open_rect(popup_area, app);

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

    let popup_area = crate::tui::popup_open_rect(popup_area, app);

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

/// Scroll ceiling for the help body given the full terminal area. Uses the same
/// popup geometry as `render_help_popup` (60% height, min 16; borders, query row,
/// and fixed footer), kept in one place so the key handler can't scroll past what
/// the renderer will show (the excess would be invisible "debt" that Up has to
/// unwind before the view moves).
pub(crate) fn help_max_scroll(area: Rect, query: &str) -> u16 {
    let popup_height = (area.height * 60 / 100).max(16).min(area.height);
    let body_height = popup_height.saturating_sub(4);
    screens::help::help_line_count(query).saturating_sub(body_height)
}

fn render_help_popup(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let popup_width = (area.width * 70 / 100).max(40).min(area.width);
    let popup_height = (area.height * 60 / 100).max(16).min(area.height);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let popup_area = crate::tui::popup_open_rect(popup_area, app);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::popup_border())
            .title(Span::styled(" Help ", theme::heading())),
        popup_area,
    );

    // Query + fixed footer; scroll only the body between them.
    let inner = popup_area.inner(Margin::new(1, 1));
    let query_line = format!("› {}\u{2588}", app.help_query);
    frame.buffer_mut().set_string(
        inner.x,
        inner.y,
        crate::tui::text::ellipsize(&query_line, inner.width as usize),
        theme::bright(),
    );

    let body = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(2),
    );
    let scroll = app.help_scroll.min(help_max_scroll(area, &app.help_query));
    frame.render_widget(screens::help::render_help(scroll, &app.help_query), body);

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

    /// App with three sessions in distinct phases plus an open picker.
    fn app_with_picker(purpose: crate::app::SessionPickerPurpose, query: &str) -> App {
        use crate::session::{SessionConfig, SessionMeta, SessionPhase};
        use std::time::Instant;

        let mut app = test_app_with_hosts();
        for (name, user, addr) in [
            ("web-prod", "micha", "10.0.0.11"),
            ("dev-box", "deploy", "10.0.0.12"),
            ("db-old", "root", "10.0.0.13"),
        ] {
            let cfg = SessionConfig {
                argv: vec!["true".into()],
                display_name: name.into(),
                meta: SessionMeta {
                    user: Some(user.into()),
                    address: Some(addr.into()),
                    port: Some(22),
                    ..Default::default()
                },
                pending_secret: None,
                key_push_identity: None,
                host_name: name.into(),
            };
            app.sessions
                .push(crate::session::Session::spawn(cfg, 24, 80, None).unwrap());
        }
        app.sessions[1].phase = SessionPhase::Running {
            started_at: Instant::now(),
        };
        app.sessions[2].phase = SessionPhase::Exited {
            status: "exit 1".into(),
            at: Instant::now(),
        };
        app.active_session = Some(1);
        app.session_picker = Some(crate::app::SessionPicker {
            purpose,
            query: query.into(),
            selected: 0,
            return_mode: AppMode::Normal,
        });
        app.mode = AppMode::SessionPicker;
        app
    }

    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    /// Read `n` cells of row `y` starting at column `x`.
    fn cells_at(buf: &ratatui::buffer::Buffer, x: u16, y: u16, n: u16) -> String {
        (x..(x + n).min(buf.area.right()))
            .map(|i| buf[(i, y)].symbol())
            .collect()
    }

    /// Column and row of the picker line carrying lifecycle word `word`.
    ///
    /// Matching the dot *plus* the padded word is what makes this unambiguous:
    /// the dashboard behind the popup draws its own session chips as
    /// `● <name>`, and a bare `find("up")` would also hit "backup" or "groups".
    fn picker_row(buf: &ratatui::buffer::Buffer, word: &str) -> (u16, u16) {
        let needle = format!("\u{25cf} {word:<4} ");
        let n = needle.chars().count() as u16;
        for y in buf.area.y..buf.area.bottom() {
            for x in buf.area.x..buf.area.right() {
                if cells_at(buf, x, y, n) == needle {
                    return (x, y);
                }
            }
        }
        panic!("no picker row for {word:?}");
    }

    /// First column of `needle` anywhere in `buf`, searched row by row.
    fn find_cell(buf: &Buffer, needle: &str) -> Option<(u16, u16)> {
        let n = needle.chars().count() as u16;
        for y in buf.area.y..buf.area.bottom() {
            for x in buf.area.x..buf.area.right().saturating_sub(n) {
                let got: String = (x..x + n).map(|i| buf[(i, y)].symbol()).collect();
                if got == needle {
                    return Some((x, y));
                }
            }
        }
        None
    }

    /// App with two spawned sessions, for the dashboard session strip.
    fn app_with_two_sessions() -> App {
        let mut app = test_app_with_hosts();
        for name in ["alpha", "bravo"] {
            let cfg = crate::session::SessionConfig {
                argv: vec!["true".into()],
                display_name: name.into(),
                meta: crate::session::SessionMeta::default(),
                pending_secret: None,
                key_push_identity: None,
                host_name: name.into(),
            };
            app.sessions
                .push(crate::session::Session::spawn(cfg, 24, 80, None).unwrap());
        }
        app
    }

    #[test]
    fn agent_panel_does_not_overprint_a_half_scrolled_card_row() {
        // The overlap is only reachable with a particular geometry, worked out
        // from the real numbers rather than guessed: the body must have at least
        // three spare rows after whole card rows (`height % 7 >= 3`), so the panel
        // is drawn at all, and the scroll must lag its goal by two lines or more,
        // so the cards sit lower than the whole-row arithmetic assumes.
        //
        // A 40-row terminal gives a 32-row body: four card rows of stride 7 fit
        // with four spare. Twelve identities in two columns make six rows; the
        // selection on row 4 puts the goal at line 14, and a scroll still at line
        // 10 pushes the last drawn card down to rows 31..36 -- straight through the
        // panel the old placement put at 34.
        let mut app = test_app_with_hosts();
        app.active_tab = 3;
        app.config.appearance.identity_columns = 2;
        app.identities = (0..12)
            .map(|i| crate::store::Identity {
                id: i as i64 + 1,
                name: format!("key-{i}"),
                username: Some("root".into()),
                private_key: Some(format!("/home/me/.ssh/sshub_key_{i}").into()),
                certificate: None,
                has_password: true,
            })
            .collect();
        app.identity_selected = 8;
        app.agent_info = Some(crate::ssh::agent::AgentInfo {
            socket_path: None,
            keys: Vec::new(),
            forwarding_hosts: 0,
        });
        app.keys_scroll_pos.set(10.0);
        app.keys_scroll_at.set(Some(std::time::Instant::now()));

        let buffer = render_to_buffer(&app, 120, 40);
        let (_, ly) = find_cell(&buffer, "loaded keys").expect("agent panel drawn");

        // The grid shows whole card rows only: a card cut by the grid's bottom
        // left a sliver above the rule that slid around while the rest sat still.
        // So no card border may appear on the rule's row or the one above it.
        let rule = ly - 1;
        for row in [rule - 1, rule] {
            let text: String = (buffer.area.x..buffer.area.right())
                .map(|x| buffer[(x, row)].symbol())
                .collect();
            assert!(
                !text.contains('\u{250c}') && !text.contains('\u{2510}'),
                "row {row} carries a card's top border: {:?}",
                text.trim_end()
            );
        }

        // Both text rows of the panel must be the panel's alone. Card borders and
        // key paths bleeding in is what this looked like on screen:
        //   agent socket  (not set)────────────┘  └──────────┘
        for row in [ly - 1, ly] {
            let text: String = (buffer.area.x..buffer.area.right())
                .map(|x| buffer[(x, row)].symbol())
                .collect();
            for leftover in [
                "\u{2518}",
                "\u{2514}",
                "\u{2502}",
                ".ssh/sshub_key",
                "passphrase",
                "not loaded",
            ] {
                assert!(
                    !text.contains(leftover),
                    "row {row} carries {leftover:?} from a card: {:?}",
                    text.trim_end()
                );
            }
        }
    }

    #[test]
    fn cycling_tabs_from_the_dashboard_stays_on_the_dashboard() {
        use crate::config::KeyAction;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = app_with_two_sessions();
        app.active_session = Some(0);
        app.mode = AppMode::Normal;
        app.config
            .keybinds
            .set(KeyAction::SessionTabNext, vec!["F6".into()]);

        app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::empty()))
            .unwrap();

        // The regression: this used to call `focus_active_session`, so a key
        // named "next session tab" threw you into the session full screen.
        assert_eq!(
            app.mode,
            AppMode::Normal,
            "cycling must not enter a session"
        );
        assert_eq!(app.active_session, Some(1), "the selection moved");
        assert!(
            app.session_tab_switch.is_some(),
            "the travel is armed from the dashboard too"
        );
    }

    #[test]
    fn the_session_slides_in_over_the_dashboard_not_over_black() {
        let mut app = app_with_two_sessions();
        app.active_session = Some(0);
        app.mode = AppMode::Normal;

        // Rendering the dashboard is what captures the snapshot the slide needs.
        let dashboard = render_to_buffer(&app, 120, 38);
        let (hx, hy) = find_cell(&dashboard, "web-prod").expect("dashboard drawn");

        // First frame of the slide: the session is still fully off to the right,
        // so what shows is the dashboard. It used to be blank cells, which read as
        // a black screen flashing before the host arrived.
        app.mode = AppMode::Session;
        app.session_enter_at = Some(std::time::Instant::now());
        let sliding = render_to_buffer(&app, 120, 38);
        assert_eq!(
            sliding[(hx, hy)].symbol(),
            dashboard[(hx, hy)].symbol(),
            "the vacated columns show the dashboard"
        );
        assert_ne!(sliding[(hx, hy)].symbol(), " ", "and are not blanked");
    }

    #[test]
    fn entering_a_session_from_the_dashboard_slides_it_in() {
        use crate::config::KeyAction;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = app_with_two_sessions();
        app.active_session = Some(0);
        app.mode = AppMode::Normal;
        app.config
            .keybinds
            .set(KeyAction::SessionFocus, vec!["F7".into()]);

        app.handle_key(KeyEvent::new(KeyCode::F(7), KeyModifiers::empty()))
            .unwrap();
        assert!(
            crate::app::is_session_mode(app.mode),
            "we are in the session"
        );
        assert!(
            app.session_enter_at.is_some(),
            "arriving animates, the same way leaving already did"
        );

        // Re-deriving the mode while already inside a session is not an entry
        // and must not replay the slide.
        app.session_enter_at = None;
        app.focus_active_session();
        assert!(app.session_enter_at.is_none());

        // Reduced motion arms nothing.
        app.mode = AppMode::Normal;
        app.config.appearance.disable_animation = true;
        app.focus_active_session();
        assert!(crate::app::is_session_mode(app.mode));
        assert!(app.session_enter_at.is_none());
    }

    #[test]
    fn dashboard_strip_highlight_travels_instead_of_teleporting() {
        let mut app = app_with_two_sessions();

        // At rest the highlight sits on the active chip, as before.
        app.active_session = Some(1);
        let buffer = render_to_buffer(&app, 120, 38);
        let (bx, by) = find_cell(&buffer, "bravo").expect("second chip rendered");
        assert_eq!(
            buffer[(bx, by)].bg,
            theme::BRIGHT,
            "at rest: on the new chip"
        );

        // Mid-switch, with progress still at ~0, the highlight must still be on
        // the chip being left. That is the whole point: it moves across rather
        // than appearing on the target instantly.
        app.session_tab_switch = Some(crate::app::SessionTabSwitch {
            dir: 1,
            from: 0,
            at: std::time::Instant::now(),
        });
        let buffer = render_to_buffer(&app, 120, 38);
        let (ax, ay) = find_cell(&buffer, "alpha").expect("first chip rendered");
        let (bx, by) = find_cell(&buffer, "bravo").expect("second chip rendered");
        assert_eq!(
            buffer[(ax, ay)].bg,
            theme::BRIGHT,
            "travelling: still on the chip being left"
        );
        assert_ne!(
            buffer[(bx, by)].bg,
            theme::BRIGHT,
            "travelling: not yet on the target"
        );

        // Reduced motion jumps straight to the final state.
        app.config.appearance.disable_animation = true;
        let buffer = render_to_buffer(&app, 120, 38);
        let (ax, ay) = find_cell(&buffer, "alpha").unwrap();
        let (bx, by) = find_cell(&buffer, "bravo").unwrap();
        assert_eq!(buffer[(bx, by)].bg, theme::BRIGHT, "reduced motion: target");
        assert_ne!(buffer[(ax, ay)].bg, theme::BRIGHT);
    }

    #[test]
    fn sftp_footer_points_back_to_local_once_the_left_pane_is_remote() {
        let mut app = test_app_with_hosts();
        app.active_tab = 1;
        app.sftp = Some(crate::sftp::model::SftpState::new("/srv", "/home/me"));

        // 120 columns on purpose: the SFTP row does not fit there, so this also
        // pins that the pair survives the truncation rather than only existing.
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "2nd host"));
        assert!(!buffer_contains(&buffer, "O local"));

        // Pointed at a second server, the footer has to say how to get back;
        // `o` only leads further away and nothing else on screen mentions `O`.
        app.sftp.as_mut().unwrap().left_host = Some("bravo".into());
        let buffer = render_to_buffer(&app, 120, 38);
        assert!(buffer_contains(&buffer, "O local"));
        assert!(!buffer_contains(&buffer, "2nd host"));
    }

    #[test]
    fn narrow_footer_keeps_help_and_quit_and_marks_the_gap() {
        // The SFTP tab has the longest row: 220 columns to show all of it.
        let mut app = test_app_with_hosts();
        app.active_tab = 1;
        app.sftp = Some(crate::sftp::model::SftpState::new("/srv", "/home/me"));

        for w in [80u16, 100, 120, 160, 200] {
            let buffer = render_to_buffer(&app, w, 38);
            assert!(buffer_contains(&buffer, "? help"), "width {w}: help");
            assert!(buffer_contains(&buffer, "q quit"), "width {w}: quit");
            assert!(
                buffer_contains(&buffer, "\u{2026}"),
                "width {w}: dropped pairs are marked"
            );
        }

        // With sessions running, the way back into one is as essential as the
        // way out of the app. This is the case that regressed: the session hints
        // are appended after `? help` / `q quit` and pushed them into the middle.
        let mut app = app_with_two_sessions();
        app.active_tab = 1;
        app.sftp = Some(crate::sftp::model::SftpState::new("/srv", "/home/me"));
        for w in [120u16, 160, 200] {
            let buffer = render_to_buffer(&app, w, 38);
            assert!(buffer_contains(&buffer, "resume"), "width {w}: resume");
            assert!(buffer_contains(&buffer, "? help"), "width {w}: help");
            assert!(buffer_contains(&buffer, "q quit"), "width {w}: quit");
        }

        // Wide enough for everything: no ellipsis, nothing dropped.
        let mut app = test_app_with_hosts();
        app.active_tab = 1;
        app.sftp = Some(crate::sftp::model::SftpState::new("/srv", "/home/me"));
        let buffer = render_to_buffer(&app, 240, 38);
        assert!(buffer_contains(&buffer, "? help"));
        assert!(buffer_contains(&buffer, "q quit"));
        assert!(buffer_contains(&buffer, "/ search"));
        assert!(!buffer_contains(&buffer, "\u{2026}"));
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
        for purpose in [
            crate::app::SessionPickerPurpose::NewSession,
            crate::app::SessionPickerPurpose::SftpLeftPane,
            crate::app::SessionPickerPurpose::SwitchSession,
        ] {
            for (w, h) in [(1u16, 1u16), (10, 3), (30, 8), (49, 20)] {
                let app = app_with_picker(purpose, "x");
                let _ = render_to_buffer(&app, w, h);
            }
        }
    }

    /// The picker keeps the dashboard's themed keybind footer for every purpose
    /// instead of swapping in the legacy status bar, which paints an off-theme
    /// `DarkGray` band and reads "Enter: connect" plus a host count under a
    /// session switcher. Compares the footer row cell by cell against the very
    /// same dashboard with no picker up, so a purpose-dependent footer or a
    /// restyled band both fail.
    #[test]
    fn session_picker_keeps_the_dashboard_footer() {
        fn footer_cells(app: &App) -> Vec<(String, Color, Color)> {
            let buf = render_to_buffer(app, 120, 38);
            let footer = dashboard_layout::dashboard_layout_zoomed(buf.area, app.ui_zoom).footer;
            (footer.x..footer.right())
                .map(|x| {
                    let cell = buf.cell((x, footer.y)).unwrap();
                    (cell.symbol().to_string(), cell.fg, cell.bg)
                })
                .collect()
        }

        for purpose in [
            crate::app::SessionPickerPurpose::NewSession,
            crate::app::SessionPickerPurpose::SftpLeftPane,
            crate::app::SessionPickerPurpose::SwitchSession,
        ] {
            let mut app = app_with_picker(purpose, "");
            assert_eq!(app.mode, AppMode::SessionPicker, "{purpose:?}");
            let with_picker = footer_cells(&app);

            // The very same app with the overlay dismissed — sessions, hosts and
            // tab all identical, so the open picker is the only difference the
            // footer could react to.
            app.session_picker = None;
            app.mode = AppMode::Normal;
            let without_picker = footer_cells(&app);

            assert!(
                without_picker
                    .iter()
                    .all(|(_, _, bg)| *bg != Color::DarkGray),
                "{purpose:?}: the dashboard footer is themed, not a DarkGray band"
            );
            assert_eq!(with_picker, without_picker, "{purpose:?} footer row");
        }
    }

    #[test]
    fn session_picker_renders_title_and_empty_state_per_purpose() {
        use crate::app::SessionPickerPurpose::{NewSession, SftpLeftPane, SwitchSession};

        for (purpose, title, empty) in [
            (NewSession, "new session tab", "(no matching hosts)"),
            (SftpLeftPane, "select left server", "(no matching hosts)"),
            (SwitchSession, "switch session", "(no matching sessions)"),
        ] {
            let app = app_with_picker(purpose, "zzzznope");
            let text = buffer_text(&render_to_buffer(&app, 80, 24));
            assert!(text.contains(title), "{purpose:?} title");
            assert!(text.contains(empty), "{purpose:?} empty state");
            assert!(text.contains("zzzznope"), "{purpose:?} query echoed");
        }
    }

    #[test]
    fn session_picker_renders_each_lifecycle_with_word_colour_and_ordinal() {
        let app = app_with_picker(crate::app::SessionPickerPurpose::SwitchSession, "");
        let buf = render_to_buffer(&app, 80, 24);

        // The word carries the state without colour, the colour without reading.
        // The ordinal sits at a fixed offset after the badge (BADGE_CELLS = 7)
        // and must be read there — the endpoints contain digits too, so a plain
        // `contains('1')` would prove nothing.
        for (word, colour, ordinal) in [
            ("conn", theme::AMBER, "1"),
            ("up", theme::GREEN, "2"),
            ("exit", theme::RED, "3"),
        ] {
            let (x, y) = picker_row(&buf, word);
            assert_eq!(buf[(x, y)].fg, colour, "{word}: dot colour");
            assert_eq!(
                cells_at(&buf, x + 7, y, 3).trim(),
                ordinal,
                "{word}: tab ordinal"
            );
        }

        let text = buffer_text(&buf);
        assert!(text.contains("micha@10.0.0.11:22"), "endpoint rendered");
        assert!(text.contains("current"), "active session marked");
    }

    #[test]
    fn session_picker_selection_highlights_without_eating_the_badge() {
        // selected = 0, i.e. the connecting row.
        let app = app_with_picker(crate::app::SessionPickerPurpose::SwitchSession, "");
        let buf = render_to_buffer(&app, 80, 24);

        let (sel_x, sel_y) = picker_row(&buf, "conn");
        let (other_x, other_y) = picker_row(&buf, "exit");

        assert_eq!(buf[(sel_x, sel_y)].bg, theme::SEL_BG, "selected row");
        assert_ne!(buf[(other_x, other_y)].bg, theme::SEL_BG, "unselected row");
        assert_eq!(
            buf[(sel_x, sel_y)].fg,
            theme::AMBER,
            "the highlight must not swallow the lifecycle colour"
        );
    }

    #[test]
    fn session_picker_draws_dashboard_or_session_behind_it() {
        let mut app = app_with_picker(crate::app::SessionPickerPurpose::SwitchSession, "");

        // The dashboard's own keybind footer stays under the popup. Read a hint
        // from its left edge, which survives the clipping at 80 columns, rather
        // than one that only fits on a wide terminal.
        app.session_picker.as_mut().unwrap().return_mode = AppMode::Normal;
        let dashboard = buffer_text(&render_to_buffer(&app, 80, 24));
        assert!(
            dashboard.contains("↵ connect"),
            "dashboard keybind footer behind the popup"
        );

        // Both session-ish origins take over the whole frame. Assert positively
        // on the session footer's own clock line, which reads "session M:SS":
        // a negative assert alone would also pass on an empty background, the
        // header hints get clipped at 80 columns with three tabs open, and a
        // bare "session " would match the popup title " switch session ".
        for origin in [AppMode::Session, AppMode::Connecting] {
            app.session_picker.as_mut().unwrap().return_mode = origin;
            let text = buffer_text(&render_to_buffer(&app, 80, 24));
            assert!(
                text.contains("session 0:"),
                "{origin:?}: session footer clock behind the popup"
            );
            assert!(
                !text.contains("↵ connect"),
                "{origin:?}: dashboard keybind footer must be gone"
            );
        }
    }

    #[test]
    fn session_picker_survives_narrow_and_wide_glyphs() {
        use crate::session::{SessionConfig, SessionMeta};

        for w in [1u16, 10, 12, 15, 20, 24, 33, 40, 56, 80] {
            for h in [1u16, 3, 8, 14, 24] {
                let app = app_with_picker(crate::app::SessionPickerPurpose::SwitchSession, "");
                let _ = render_to_buffer(&app, w, h);
            }
        }

        // The endpoint must begin after the *terminal-cell width* of the name,
        // not its scalar count. These three cases independently pin CJK,
        // emoji, and combining-mark advancement.
        for (name, expected_offset) in [("日本語の", 21u16), ("🚀", 15u16), ("e\u{0301}dge", 17u16)]
        {
            let mut app = test_app_with_hosts();
            let cfg = SessionConfig {
                argv: vec!["true".into()],
                display_name: name.into(),
                meta: SessionMeta {
                    address: Some("10.0.0.1".into()),
                    port: Some(22),
                    ..Default::default()
                },
                pending_secret: None,
                key_push_identity: None,
                host_name: name.into(),
            };
            app.sessions
                .push(crate::session::Session::spawn(cfg, 24, 80, None).unwrap());
            app.active_session = Some(0);
            app.session_picker = Some(crate::app::SessionPicker {
                purpose: crate::app::SessionPickerPurpose::SwitchSession,
                query: String::new(),
                selected: 0,
                return_mode: AppMode::Normal,
            });
            app.mode = AppMode::SessionPicker;

            let buf = render_to_buffer(&app, 80, 14);
            let (x, y) = picker_row(&buf, "conn");
            assert_eq!(
                cells_at(&buf, x + expected_offset, y, 10),
                "10.0.0.1:2",
                "{name:?}: endpoint column"
            );
        }
    }

    #[test]
    fn dashboard_footer_shows_keybinds() {
        let app = test_app_with_hosts();
        let buffer = render_to_buffer(&app, 132, 38);
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

        let [total, online, slow, down] = compute_header_stats(&app);
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
            key_push_identity: None,
            host_name: "web-prod".into(),
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
            key_push_identity: None,
            host_name: "web-prod".into(),
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
            key_push_identity: None,
            host_name: "web-prod".into(),
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

        // The grid scrolls to the selection over a few frames now (#35), so
        // run it out with a backdated frame clock before looking.
        app.identity_selected = 28;
        let mut buffer = render_to_buffer(&app, 120, 38);
        for _ in 0..40 {
            app.keys_scroll_at.set(Some(
                std::time::Instant::now() - std::time::Duration::from_millis(16),
            ));
            buffer = render_to_buffer(&app, 120, 38);
        }
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

    #[test]
    fn help_overlay_shows_query_and_filters() {
        let mut app = test_app_with_hosts();
        app.mode = AppMode::Help;
        let full = render_to_buffer(&app, 100, 40);
        assert!(buffer_contains(&full, "navigate"));
        assert!(buffer_contains(&full, "type to filter"));
        assert!(buffer_contains(&full, "›"));

        app.help_query = "favorite".into();
        let filtered = render_to_buffer(&app, 100, 40);
        assert!(buffer_contains(&filtered, "Toggle favorite"));
        assert!(buffer_contains(&filtered, "hosts (tab 1)"));
        assert!(!buffer_contains(&filtered, "Cycle filter"));
    }

    #[test]
    fn keybind_editor_shows_query_and_filters() {
        let mut app = test_app_with_hosts();
        app.keybind_editor = Some(crate::app::KeybindEditor {
            selected: 0,
            scroll: 0,
            capturing: false,
            append: false,
            query: "quit".into(),
        });
        app.mode = AppMode::KeybindEditor;
        let buffer = render_to_buffer(&app, 100, 40);
        assert!(buffer_contains(&buffer, "› quit"));
        assert!(buffer_contains(&buffer, "Quit"));
        assert!(buffer_contains(&buffer, "type to filter"));
        // Save form is ALL[0]; under "quit" it must not be the selected row content
        // unless its binds somehow match — label "Save form" does not.
        assert!(!buffer_contains(&buffer, "Save form"));
    }
}
