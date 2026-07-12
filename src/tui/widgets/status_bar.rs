use ratatui::prelude::Style;
use ratatui::style::Color;
use ratatui::widgets::Paragraph;

use crate::app::{App, AppMode};

fn mode_label(mode: AppMode) -> &'static str {
    match mode {
        AppMode::Normal => "Normal",
        AppMode::Search => "Search",
        AppMode::TagFilter => "Tag filter",
        AppMode::HostDetail => "Detail",
        AppMode::HostForm => "Host form",
        AppMode::IdentityForm => "Identity form",
        AppMode::GroupForm => "Group form",
        AppMode::GroupFieldPicker => "Select",
        AppMode::TunnelHostPicker => "Select server",
        AppMode::SessionHostPicker => "New session",
        AppMode::GroupManage => "Groups",
        AppMode::FieldPicker => "Select",
        AppMode::KeybindEditor => "Keybindings",
        AppMode::Settings => "Settings",
        AppMode::ConfirmQuit => "Quit?",
        AppMode::ConfirmDelete => "Confirm delete",
        AppMode::ConfirmDiscard => "Save changes?",
        AppMode::Help => "Help",
        AppMode::Palette => "Palette",
        AppMode::TunnelForm => "Tunnel form",
        AppMode::ImportPrompt => "Import",
        AppMode::SftpPrompt => "SFTP",
        AppMode::Connecting => "Connecting",
        AppMode::Session => "Session",
    }
}

pub fn render_status_bar(app: &App) -> Paragraph<'static> {
    if app.mode == AppMode::GroupManage {
        let total = app.groups.len();
        let mut line =
            format!("{total} groups │ a: add │ e: edit │ d: delete │ Esc/h: back │ q: quit");
        if let Some(notice) = &app.group_notice {
            line.push_str(" │ ");
            line.push_str(notice);
        }
        return Paragraph::new(line).style(Style::default().bg(Color::DarkGray).fg(Color::White));
    }

    let total = app.hosts.len();
    let shown = app.filtered_indices.len();
    let mode = mode_label(app.mode);
    let action = match app.mode {
        AppMode::HostDetail => "Enter: save",
        AppMode::HostForm => "Enter: save",
        AppMode::GroupForm => "Enter: save",
        _ => "Enter: connect",
    };
    let mut line = format!(
        "{shown}/{total} hosts │ sort: {} │ {mode} │ {action} │ q: quit",
        app.sort_mode.label()
    );
    if !app.sessions.is_empty() {
        let n = app.sessions.len();
        let tab = app.active_session.map(|i| i + 1).unwrap_or(1);
        line.push_str(&format!(
            " │ {n} session{} (tab {tab})",
            if n == 1 { "" } else { "s" }
        ));
    }
    if let Some(notice) = &app.host_notice {
        line.push_str(" │ ");
        line.push_str(notice);
    }
    Paragraph::new(line).style(Style::default().bg(Color::DarkGray).fg(Color::White))
}
