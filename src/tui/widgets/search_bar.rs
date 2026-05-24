use ratatui::prelude::{Modifier, Style};
use ratatui::style::Color;
use ratatui::widgets::Paragraph;

use crate::app::{App, AppMode};

/// Collect unique tags from all hosts, sorted.
pub fn unique_tags(app: &App) -> Vec<String> {
    let mut tags: Vec<String> = app
        .hosts
        .iter()
        .flat_map(|entry| entry.tags().iter().cloned())
        .collect();
    tags.sort();
    tags.dedup();
    tags
}

fn tag_chips(app: &App) -> String {
    let tags = unique_tags(app);
    if tags.is_empty() && app.tag_filter.is_none() {
        return String::new();
    }

    let mut chips = String::new();
    if let Some(active) = &app.tag_filter {
        chips.push_str(&format!("[#{active}]"));
    }
    for tag in tags {
        if app.tag_filter.as_deref() == Some(tag.as_str()) {
            continue;
        }
        chips.push_str(&format!("[#{tag}]"));
    }
    chips
}

pub fn render_search_bar(app: &App) -> Paragraph<'static> {
    let prefix = match app.mode {
        AppMode::Search => "/ ",
        AppMode::TagFilter => "T ",
        _ => "  ",
    };
    let chips = tag_chips(app);
    let query_part = format!("{prefix}{}", app.search_query);
    let line = if chips.is_empty() {
        query_part
    } else {
        format!("{query_part}  {chips}")
    };

    let style = if app.mode == AppMode::Search || app.mode == AppMode::TagFilter {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    };

    Paragraph::new(line).style(style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppDeps, HostEntry};
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

    fn app_with_hosts(hosts: Vec<HostEntry>, tag_filter: Option<String>) -> App {
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
        app.hosts = hosts;
        app.tag_filter = tag_filter;
        app
    }

    #[test]
    fn unique_tags_dedups_and_sorts() {
        let app = app_with_hosts(
            vec![
                HostEntry::Legacy {
                    host: SshHost::new("a"),
                    meta: crate::metadata::HostMetadata {
                        tags: vec!["prod".into(), "db".into()],
                        ..Default::default()
                    },
                },
                HostEntry::Legacy {
                    host: SshHost::new("b"),
                    meta: crate::metadata::HostMetadata {
                        tags: vec!["prod".into(), "web".into()],
                        ..Default::default()
                    },
                },
            ],
            None,
        );
        assert_eq!(unique_tags(&app), vec!["db", "prod", "web"]);
    }

    #[test]
    fn tag_chips_include_active_filter_first() {
        let app = app_with_hosts(
            vec![HostEntry::Legacy {
                host: SshHost::new("a"),
                meta: crate::metadata::HostMetadata {
                    tags: vec!["prod".into(), "web".into()],
                    ..Default::default()
                },
            }],
            Some("prod".into()),
        );
        assert_eq!(tag_chips(&app), "[#prod][#web]");
    }
}
