use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, AppMode, DetailEditField, HostEntry};
use crate::theme::catalog::{ColorRole, StyleRole};
use crate::theme::model::ResolvedTheme;

/// The three styles this panel writes with.
///
/// The hard-coded `Cyan` / `Yellow` / `DarkGray` these used to be were never
/// design tokens; the spec lists this file under the direct colours that get
/// unified onto semantic roles.
#[derive(Clone, Copy)]
struct DetailStyles {
    label: Style,
    value: Style,
    favorite: Style,
    hint: Style,
}

impl DetailStyles {
    fn of(theme: &ResolvedTheme) -> Self {
        Self {
            label: theme.style(StyleRole::DashboardDetailsLabel),
            value: theme.style(StyleRole::DashboardDetailsValue),
            favorite: Style::default().fg(theme.color(ColorRole::StatusWarning)),
            hint: theme.style(StyleRole::TextDim),
        }
    }
}

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

fn detail_line(styles: DetailStyles, label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), styles.label),
        Span::styled(value, styles.value),
    ])
}

fn detail_fav_line(styles: DetailStyles, fav: bool) -> Line<'static> {
    if fav {
        Line::from(vec![
            Span::styled("Favorite: ", styles.label),
            Span::styled("yes ★", styles.favorite),
        ])
    } else {
        Line::from(vec![
            Span::styled("Favorite: ", styles.label),
            Span::styled("no", styles.value),
        ])
    }
}

fn tri_state_line(label: &str, value: &str, active: bool) -> String {
    let prefix = if active { "> " } else { "  " };
    format!("{prefix}{label}: {value} (Space or arrows to cycle)")
}

fn host_detail_view(
    app: &App,
    entry: &HostEntry,
    _host_idx: usize,
    styles: DetailStyles,
) -> Vec<Line<'static>> {
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

    let hint_style = styles.hint;

    let mut lines = vec![
        detail_line(styles, "Host", entry.name().to_string()),
        detail_line(styles, "Label", entry.display_name().to_string()),
        detail_line(styles, "Address", dash(&ssh.hostname).to_string()),
        detail_line(styles, "User", dash(&ssh.user).to_string()),
        detail_line(styles, "Port", format_port(ssh.port)),
        detail_line(styles, "Group", group_line),
        detail_line(styles, "Identity", identity_line),
        detail_line(styles, "ProxyJump", dash(&ssh.proxy_jump).to_string()),
        detail_line(styles, "Source", source.to_string()),
        Line::from(""),
        detail_line(
            styles,
            "Tags",
            if entry.tags().is_empty() {
                "—".into()
            } else {
                entry.tags().join(", ")
            },
        ),
        detail_line(
            styles,
            "Environment",
            dash(&entry.environment().map(str::to_string)).to_string(),
        ),
        detail_line(
            styles,
            "Description",
            dash(&entry.description().map(str::to_string)).to_string(),
        ),
        detail_fav_line(styles, entry.favorite()),
        detail_line(styles, "Last connected", last),
    ];

    lines.push(detail_line(
        styles,
        "Session log",
        entry.session_logging_override().label().to_string(),
    ));
    lines.push(detail_line(
        styles,
        "Transport",
        entry.session_transport().label().to_string(),
    ));

    lines.extend([
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
    ]);

    // Prepend the detected OS logo (colored ASCII art) when enabled and the
    // host's os_icon resolves to a known logo. render_detail_panel returns a
    // Paragraph, so the logo is composed as colored Lines rather than rendered
    // into a carved sub-column.
    if app.config.appearance.os_logo {
        if let Some(logo) = entry
            .managed()
            .and_then(|m| m.os_icon.as_deref())
            .and_then(crate::osinfo::logo_for)
        {
            let mut prefixed = crate::osinfo::widget::logo_to_lines(logo);
            prefixed.push(Line::from(""));
            prefixed.extend(lines);
            lines = prefixed;
        }
    }

    lines
}

fn host_detail_edit(
    app: &App,
    entry: &HostEntry,
    _host_idx: usize,
    styles: DetailStyles,
) -> Vec<Line<'static>> {
    let edit = app
        .detail_edit
        .as_ref()
        .expect("HostDetail requires detail_edit");
    let ssh = entry.ssh_host();

    let tags_line = field_with_cursor(
        "Tags (comma-separated)",
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
    let session_log_line = tri_state_line(
        "Session log",
        edit.session_logging.label(),
        edit.field == DetailEditField::SessionLogging,
    );

    let hint_style = styles.hint;

    vec![
        detail_line(styles, "Host", entry.name().to_string()),
        detail_line(styles, "Address", dash(&ssh.hostname).to_string()),
        Line::from(""),
        Line::from(Span::styled("Editing metadata", styles.label)),
        Line::from(tags_line),
        Line::from(desc_line),
        Line::from(env_line),
        Line::from(session_log_line),
        detail_fav_line(styles, entry.favorite()),
        Line::from(""),
        Line::from(Span::styled("[Enter] save", hint_style)),
        Line::from(Span::styled("[Esc] cancel", hint_style)),
        Line::from(Span::styled("[Tab/j/k] field", hint_style)),
        Line::from(Span::styled("[f] toggle favourite", hint_style)),
    ]
}

fn host_detail_text(
    app: &App,
    entry: &HostEntry,
    host_idx: usize,
    styles: DetailStyles,
) -> Vec<Line<'static>> {
    if app.mode == AppMode::HostDetail && app.detail_edit.is_some() {
        host_detail_edit(app, entry, host_idx, styles)
    } else {
        host_detail_view(app, entry, host_idx, styles)
    }
}

pub fn render_detail_panel(app: &App) -> Paragraph<'static> {
    let styles = DetailStyles::of(app.theme());
    let lines = if let Some(host_idx) = app.selected_host_index() {
        let entry = &app.hosts[host_idx];
        host_detail_text(app, entry, host_idx, styles)
    } else {
        vec![Line::from(Span::styled("No host selected", styles.hint))]
    };
    Paragraph::new(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppDeps, HostDetailEdit};
    use crate::config::AppConfig;
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
            session_logging: crate::session_log::SessionLoggingOverride::Inherit,
            field: DetailEditField::Tags,
            cursor: 4,
        });

        let lines = host_detail_text(&app, &app.hosts[0], 0, DetailStyles::of(app.theme()));
        let text: String = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("> Tags (comma-separated): prod_"));
        assert!(text.contains("[Enter] save"));
        assert!(!text.contains("[e] edit metadata"));
    }

    /// The panel's four styles come from four roles. `Color::Cyan` /
    /// `Color::Yellow` / `Color::DarkGray` were direct ANSI, so nothing about
    /// `default` would have caught them staying put — a marker theme does.
    #[test]
    fn the_detail_lines_take_their_own_marker_roles() {
        use crate::tui::widgets::panel_box::tests::resolved_source;
        use ratatui::style::Color;

        let theme = resolved_source(
            "markers",
            "schema_version = 1\nname = \"Markers\"\nextends = \"default\"\n\n\
             [components.dashboard.details]\n\
             label = { foreground = \"#ff00ff\" }\n\
             value = { foreground = \"#00ffff\" }\n\n\
             [components.status]\nwarning = \"#ffaa00\"\n\n\
             [components.text]\ndim = { foreground = \"#333333\" }\n",
        );
        let styles = DetailStyles::of(&theme);

        let line = detail_line(styles, "Host", "web-prod".into());
        assert_eq!(line.spans[0].style.fg, Some(Color::Rgb(0xff, 0x00, 0xff)));
        assert_eq!(line.spans[1].style.fg, Some(Color::Rgb(0x00, 0xff, 0xff)));

        let fav = detail_fav_line(styles, true);
        assert_eq!(fav.spans[1].style.fg, Some(Color::Rgb(0xff, 0xaa, 0x00)));

        assert_eq!(styles.hint.fg, Some(Color::Rgb(0x33, 0x33, 0x33)));
    }
}
