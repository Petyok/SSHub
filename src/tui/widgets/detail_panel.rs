use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, AppMode, DetailEditField, HostEntry};

fn dash(opt: &Option<String>) -> &str {
    match opt {
        Some(s) if !s.is_empty() => s.as_str(),
        _ => "—",
    }
}

fn format_port(port: Option<u16>) -> String {
    port.map(|p| p.to_string())
        .unwrap_or_else(|| "—".to_string())
}

/// Format unix timestamp for display; supports default `%Y-%m-%d %H:%M` in UTC.
fn format_last_connected(ts: i64, date_format: &str) -> String {
    if date_format == "%Y-%m-%d %H:%M" {
        format_utc_ymd_hm(ts)
    } else {
        ts.to_string()
    }
}

fn format_utc_ymd_hm(ts: i64) -> String {
    const SECS_PER_DAY: i64 = 86_400;
    const SECS_PER_HOUR: i64 = 3_600;
    const SECS_PER_MIN: i64 = 60;

    let days = ts.div_euclid(SECS_PER_DAY);
    let rem = ts.rem_euclid(SECS_PER_DAY);
    let hour = rem / SECS_PER_HOUR;
    let minute = (rem % SECS_PER_HOUR) / SECS_PER_MIN;

    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Algorithm from Howard Hinnant (civil calendar from unix days).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as i64, d as i64)
}

fn field_with_cursor(label: &str, value: &str, cursor: usize, active: bool) -> String {
    let prefix = if active { "> " } else { "  " };
    let display = if value.is_empty() {
        "_".to_string()
    } else {
        let clamped = crate::text_input::byte_index(value, cursor);
        let (before, after) = value.split_at(clamped);
        format!("{before}_{after}")
    };
    format!("{prefix}{label}: {display}")
}

fn detail_line(label: &str, value: String) -> Line<'static> {
    let label_style = Style::default().fg(Color::Cyan);
    Line::from(vec![
        Span::styled(format!("{label}: "), label_style),
        Span::raw(value),
    ])
}

fn detail_fav_line(fav: bool) -> Line<'static> {
    let label_style = Style::default().fg(Color::Cyan);
    if fav {
        Line::from(vec![
            Span::styled("Favorite: ", label_style),
            Span::styled("yes ★", Style::default().fg(Color::Yellow)),
        ])
    } else {
        Line::from(vec![
            Span::styled("Favorite: ", label_style),
            Span::raw("no"),
        ])
    }
}

fn host_detail_view(app: &App, entry: &HostEntry, _host_idx: usize) -> Vec<Line<'static>> {
    let ssh = entry.ssh_host();
    let last = entry
        .last_connected()
        .map(|ts| format_last_connected(ts, &app.config.appearance.date_format))
        .unwrap_or_else(|| "—".to_string());

    let group_line = match entry.managed().and_then(|m| m.group.as_ref()) {
        Some(g) => g.name.clone(),
        None => "—".to_string(),
    };
    let identity_line = match entry.managed().and_then(|m| m.identity.as_ref()) {
        Some(i) => i.name.clone(),
        None => dash(&ssh.identity_file).to_string(),
    };
    let source = match entry.source() {
        crate::store::HostSource::Launcher => "launcher",
        crate::store::HostSource::SshConfig => "ssh_config",
    };

    let hint_style = Style::default().fg(Color::DarkGray);

    vec![
        detail_line("Host", entry.name().to_string()),
        detail_line("Label", entry.display_name().to_string()),
        detail_line("Address", dash(&ssh.hostname).to_string()),
        detail_line("User", dash(&ssh.user).to_string()),
        detail_line("Port", format_port(ssh.port)),
        detail_line("Group", group_line),
        detail_line("Identity", identity_line),
        detail_line("ProxyJump", dash(&ssh.proxy_jump).to_string()),
        detail_line("Source", source.to_string()),
        Line::from(""),
        detail_line(
            "Tags",
            if entry.tags().is_empty() {
                "—".into()
            } else {
                entry.tags().join(", ")
            },
        ),
        detail_line(
            "Environment",
            dash(&entry.environment().map(str::to_string)).to_string(),
        ),
        detail_line(
            "Description",
            dash(&entry.description().map(str::to_string)).to_string(),
        ),
        detail_fav_line(entry.favorite()),
        detail_line("Last connected", last),
        Line::from(""),
        Line::from(Span::styled(
            if entry.is_launcher() {
                "[e] edit host"
            } else {
                "[e] edit metadata"
            },
            hint_style,
        )),
        Line::from(Span::styled("[f] toggle favourite", hint_style)),
    ]
}

fn host_detail_edit(app: &App, entry: &HostEntry, _host_idx: usize) -> Vec<Line<'static>> {
    let edit = app
        .detail_edit
        .as_ref()
        .expect("HostDetail requires detail_edit");
    let ssh = entry.ssh_host();

    let tags_line = field_with_cursor(
        "Tags",
        &edit.tags,
        edit.cursor,
        edit.field == DetailEditField::Tags,
    );
    let desc_line = field_with_cursor(
        "Description",
        &edit.description,
        edit.cursor,
        edit.field == DetailEditField::Description,
    );
    let env_line = field_with_cursor(
        "Environment",
        &edit.environment,
        edit.cursor,
        edit.field == DetailEditField::Environment,
    );

    let hint_style = Style::default().fg(Color::DarkGray);

    vec![
        detail_line("Host", entry.name().to_string()),
        detail_line("Address", dash(&ssh.hostname).to_string()),
        Line::from(""),
        Line::from(Span::styled(
            "Editing metadata",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(tags_line),
        Line::from(desc_line),
        Line::from(env_line),
        detail_fav_line(entry.favorite()),
        Line::from(""),
        Line::from(Span::styled("[Enter] save", hint_style)),
        Line::from(Span::styled("[Esc] cancel", hint_style)),
        Line::from(Span::styled("[Tab/j/k] field", hint_style)),
        Line::from(Span::styled("[f] toggle favourite", hint_style)),
    ]
}

fn host_detail_text(app: &App, entry: &HostEntry, host_idx: usize) -> Vec<Line<'static>> {
    if app.mode == AppMode::HostDetail && app.detail_edit.is_some() {
        host_detail_edit(app, entry, host_idx)
    } else {
        host_detail_view(app, entry, host_idx)
    }
}

pub fn render_detail_panel(app: &App) -> Paragraph<'static> {
    let lines = if let Some(host_idx) = app.selected_host_index() {
        let entry = &app.hosts[host_idx];
        host_detail_text(app, entry, host_idx)
    } else {
        vec![Line::from("No host selected")]
    };
    Paragraph::new(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppDeps, HostDetailEdit};
    use crate::config::AppConfig;
    use crate::launcher::TerminalLauncher;
    use crate::metadata::MetadataDb;
    use crate::ssh::{HostResolver, SshHost};
    use crate::store::LauncherStore;
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

    #[test]
    fn format_utc_ymd_hm_known_epoch() {
        // 2024-01-01 00:00:00 UTC
        assert_eq!(format_utc_ymd_hm(1_704_067_200), "2024-01-01 00:00");
    }

    #[test]
    fn host_detail_edit_shows_active_field_marker() {
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
        app.hosts = vec![HostEntry::new(SshHost::new("web"))];
        app.filtered_indices = vec![0];
        app.mode = AppMode::HostDetail;
        app.detail_edit = Some(HostDetailEdit {
            tags: "prod".into(),
            description: String::new(),
            environment: String::new(),
            field: DetailEditField::Tags,
            cursor: 4,
        });

        let lines = host_detail_text(&app, &app.hosts[0], 0);
        let text: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("> Tags: prod_"));
        assert!(text.contains("[Enter] save"));
        assert!(!text.contains("[e] edit metadata"));
    }
}
