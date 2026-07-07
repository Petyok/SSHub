mod audit;
mod connect;
mod field_picker;
mod groups;
mod host_crud;
mod host_detail;
mod host_form;
mod hostlist;
mod identities;
mod import;
mod keys;
mod mouse;
mod session;
mod tags;
mod tunnels;
mod types;
mod util;

#[cfg(test)]
mod tests;

pub use types::*;
pub use util::*;

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::config::{self, AppConfig, KeyAction};
use crate::launcher::{self, TerminalLauncher};
use crate::metadata::{MetadataDb, MetadataStore};
use crate::search::HostSearch;
use crate::ssh::{
    export_launcher_hosts, import_ssh_config, sync_ssh_config_hosts, HostResolver, ImportReport,
    SshConfigResolver, SshHost,
};
use crate::store::{
    DeleteHostOutcome, HostGroup, HostGroupUpdate, HostSource, HostUpdate, Identity,
    IdentityUpdate, LauncherStore, ManagedHost, NewHost, NewHostGroup, NewIdentity,
};
use crate::text_input;
use crate::watcher::WatchEvent;

/// Virtual group label for hosts without a DB group.
pub const UNGROUPED_LABEL: &str = "_ungrouped";

/// Collapsed-state key for the virtual "ungrouped" bucket (real group ids are
/// positive, so -1 never collides).
pub const UNGROUPED_KEY: i64 = -1;

/// Base host-name column width (chars) at zoom level 0.
pub const NAME_WIDTH_BASE: usize = 14;
/// Extra name-column width added per zoom level.
pub const NAME_WIDTH_STEP: usize = 8;
/// Maximum UI zoom level.
pub const UI_ZOOM_MAX: usize = 3;

pub const OS_ICON_OPTIONS: [&str; 4] = ["(none)", "generic", "ubuntu", "debian"];

/// Injectable dependencies for [`App`].
pub struct AppDeps {
    pub resolver: Box<dyn HostResolver>,
    pub metadata: Arc<dyn MetadataStore>,
    pub store: Arc<LauncherStore>,
    pub launcher: Box<dyn TerminalLauncher>,
    pub password_store: Box<dyn crate::credentials::PasswordStore>,
}

/// Application state and input handling (TUI loop wired in F9).
pub struct App {
    pub hosts: Vec<HostEntry>,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
    pub search_query: String,
    pub mode: AppMode,
    pub config: AppConfig,
    /// Active tag filters. A host matches when it carries every selected tag
    /// (AND). Empty means no tag filtering.
    pub tag_filters: Vec<String>,
    /// Highlighted row in the tag-filter popup (0 = "all"; 1.. = a tag).
    pub tag_filter_selected: usize,
    pub watcher_rx: Option<Receiver<WatchEvent>>,
    pub should_quit: bool,
    pub detail_focus: bool,
    pub detail_edit: Option<HostDetailEdit>,
    pub identities: Vec<Identity>,
    pub identity_selected: usize,
    pub identity_form: Option<IdentityFormEdit>,
    pub identity_notice: Option<String>,
    pub groups: Vec<HostGroup>,
    pub host_form: Option<HostFormEdit>,
    pub field_picker: Option<FieldPicker>,
    pub group_form: Option<GroupFormEdit>,
    /// Dedicated default-identity picker for a group (opened with `e`).
    pub group_field_picker: Option<GroupFieldPicker>,
    /// Searchable SSH-server picker for the tunnel form.
    pub tunnel_host_picker: Option<TunnelHostPicker>,
    /// Searchable host picker for a new embedded session tab.
    pub session_host_picker: Option<SessionHostPicker>,
    pub import_prompt: Option<ImportPromptEdit>,
    /// UI zoom level (0 = default). Widens the hosts column in the layout and
    /// the host-name column within it.
    pub ui_zoom: usize,
    pub group_manage_selected: usize,
    pub group_notice: Option<String>,
    pub host_notice: Option<String>,
    pub sort_mode: SortMode,
    pub pending_delete: Option<PendingDelete>,
    pub pre_help_mode: Option<AppMode>,
    /// Mode to return to if the quit dialog is cancelled.
    pub pre_quit_mode: Option<AppMode>,
    pub group_sections: Vec<HostGroupSection>,
    /// Selectable rows (group headers + hosts of expanded groups).
    pub nav_rows: Vec<NavRow>,
    /// Group keys ([`HostGroupSection::key`]) that are currently collapsed.
    pub collapsed_groups: std::collections::HashSet<i64>,
    /// Keybind editor state: `(selected action row, capturing next key)`.
    pub keybind_editor: Option<KeybindEditor>,
    pub active_tab: usize,
    pub palette_query: String,
    pub palette_selected: usize,
    pub palette_results: Vec<usize>,
    pub ping_rx: Option<Receiver<crate::ping::PingResult>>,
    pub ping_data: std::collections::HashMap<String, Vec<u32>>,
    pub probe_rx: Option<Receiver<crate::ssh::probe::SshLogEntry>>,
    pub ssh_log: Vec<crate::ssh::probe::SshLogEntry>,
    pub ssh_log_scroll: usize,
    pub auth_events_cache: Vec<crate::store::AuthEvent>,
    pub auth_stats_cache: (i64, i64),
    auth_cache_updated: std::time::Instant,
    pub audit_filter: AuditFilter,
    pub audit_range: AuditRange,
    pub audit_selected: usize,
    pub audit_scroll: usize,
    pub agent_info: Option<crate::ssh::agent::AgentInfo>,
    agent_info_updated: std::time::Instant,
    pub tunnels: Vec<crate::store::Tunnel>,
    pub tunnel_selected: usize,
    pub tunnel_form: Option<TunnelFormEdit>,
    pub tunnel_notice: Option<String>,
    pub tunnel_manager: crate::tunnel::TunnelManager,
    pub terminal_area: ratatui::layout::Rect,
    /// Embedded PTY sessions. Multiple may coexist (Ctrl+T opens a new tab).
    /// Empty when not in `Connecting` / `Session` mode.
    pub sessions: Vec<crate::session::Session>,
    /// Index into `sessions` of the visible tab. `None` when `sessions` is empty.
    pub active_session: Option<usize>,
    last_click: Option<(std::time::Instant, u16, u16)>,
    resolver: Box<dyn HostResolver>,
    metadata: Arc<dyn MetadataStore>,
    store: Arc<LauncherStore>,
    // Retained for AppDeps compatibility but no longer called now that
    // sessions run on an embedded PTY. The launcher impls in src/launcher/
    // stay in the binary but are dead at runtime.
    #[allow(dead_code)]
    launcher: Box<dyn TerminalLauncher>,
    password_store: Box<dyn crate::credentials::PasswordStore>,
    search: HostSearch,
}

impl App {
    /// Build app with default resolver, on-disk metadata db, and config-derived launcher.
    pub fn new(config: AppConfig) -> Result<Self> {
        let data_dir = config::data_dir()?;
        std::fs::create_dir_all(&data_dir)?;

        let launcher_path = data_dir.join("launcher.db");
        let first_run = !launcher_path.exists();

        let db_path = data_dir.join("metadata.db");
        let metadata = Arc::new(MetadataDb::open(db_path)?);
        let store = Arc::new(LauncherStore::open(launcher_path)?);
        let resolver = Box::new(SshConfigResolver::default());
        let launcher = launcher::launcher_from_config(&config)?;
        let mut app = Self::new_with_deps(
            config,
            AppDeps {
                resolver,
                metadata,
                store,
                launcher,
                password_store: Box::new(crate::credentials::OsKeyring),
            },
        );
        app.reload_hosts()?;
        app.refresh_auth_cache();
        app.start_ping_worker();

        if first_run && app.hosts.is_empty() {
            app.mode = AppMode::Help;
        }

        Ok(app)
    }

    /// Build app from explicit dependencies (tests inject mocks here).
    pub fn new_with_deps(config: AppConfig, deps: AppDeps) -> Self {
        Self {
            hosts: Vec::new(),
            filtered_indices: Vec::new(),
            selected: 0,
            search_query: String::new(),
            mode: AppMode::Normal,
            config,
            tag_filters: Vec::new(),
            tag_filter_selected: 0,
            watcher_rx: None,
            should_quit: false,
            detail_focus: false,
            detail_edit: None,
            identities: Vec::new(),
            identity_selected: 0,
            identity_form: None,
            identity_notice: None,
            groups: Vec::new(),
            host_form: None,
            field_picker: None,
            group_form: None,
            group_field_picker: None,
            tunnel_host_picker: None,
            session_host_picker: None,
            import_prompt: None,
            ui_zoom: 0,
            group_manage_selected: 0,
            group_notice: None,
            host_notice: None,
            sort_mode: SortMode::default(),
            pending_delete: None,
            pre_help_mode: None,
            pre_quit_mode: None,
            group_sections: Vec::new(),
            nav_rows: Vec::new(),
            collapsed_groups: std::collections::HashSet::new(),
            keybind_editor: None,
            active_tab: 0,
            palette_query: String::new(),
            palette_selected: 0,
            palette_results: Vec::new(),
            ping_rx: None,
            ping_data: std::collections::HashMap::new(),
            probe_rx: None,
            ssh_log: Vec::new(),
            ssh_log_scroll: 0,
            auth_events_cache: Vec::new(),
            auth_stats_cache: (0, 0),
            auth_cache_updated: std::time::Instant::now() - std::time::Duration::from_secs(60),
            audit_filter: AuditFilter::default(),
            audit_range: AuditRange::default(),
            audit_selected: 0,
            audit_scroll: 0,
            agent_info: None,
            agent_info_updated: std::time::Instant::now() - std::time::Duration::from_secs(60),
            tunnels: Vec::new(),
            tunnel_selected: 0,
            tunnel_form: None,
            tunnel_notice: None,
            tunnel_manager: crate::tunnel::TunnelManager::new(),
            terminal_area: ratatui::layout::Rect::default(),
            sessions: Vec::new(),
            active_session: None,
            last_click: None,
            resolver: deps.resolver,
            metadata: deps.metadata,
            store: deps.store,
            launcher: deps.launcher,
            password_store: deps.password_store,
            search: HostSearch::new(),
        }
    }

    pub fn set_watcher_rx(&mut self, rx: Receiver<WatchEvent>) {
        self.watcher_rx = Some(rx);
    }

    /// Refresh the auth events cache if more than 10 seconds have elapsed.
    pub fn refresh_auth_cache(&mut self) {
        if self.auth_cache_updated.elapsed() > std::time::Duration::from_secs(10) {
            self.auth_events_cache = self.store.list_auth_events(20).unwrap_or_default();
            self.auth_stats_cache = self.store.auth_event_stats(7).unwrap_or((0, 0));
            self.auth_cache_updated = std::time::Instant::now();
        }
    }

    /// Launch a background thread that pings all known host addresses periodically.
    /// Should NOT be called in test/CI environments.
    pub fn start_ping_worker(&mut self) {
        let hosts: Vec<(String, String)> = self
            .hosts
            .iter()
            .filter_map(|h| {
                let addr = match h {
                    HostEntry::Managed(m) => m.address.clone(),
                    HostEntry::Legacy { host, .. } => host.hostname.clone()?,
                };
                if addr.is_empty() {
                    return None;
                }
                Some((h.name().to_string(), addr))
            })
            .collect();
        if !hosts.is_empty() {
            self.ping_rx = Some(crate::ping::spawn_ping_worker(
                hosts.clone(),
                std::time::Duration::from_secs(30),
            ));
            // We used to also spawn `ssh -v` against every host every 60s
            // and dump its output into the SSH log — but that buried the
            // events the user actually cares about (their own connect
            // attempts + auto-auth diagnostics) under hundreds of probe
            // lines. Status freshness still comes from the ping worker
            // above; the SSH log is now reserved for user-initiated events.
        }
    }

    /// Reload host list from launcher store + ssh_config resolver, rebuild filter.
    /// Append to the SSH log, keeping a bounded history so a long-running
    /// session doesn't grow memory without limit.
    pub fn push_ssh_log(&mut self, entry: crate::ssh::probe::SshLogEntry) {
        self.ssh_log.push(entry);
        const MAX_SSH_LOG: usize = 200;
        if self.ssh_log.len() > MAX_SSH_LOG {
            let excess = self.ssh_log.len() - MAX_SSH_LOG;
            self.ssh_log.drain(..excess);
        }
    }

    /// Drop all SSH log entries for `host_name`. Called once a session has
    /// authenticated so connect-time debug noise doesn't linger on the dashboard.
    pub fn clear_ssh_log_for_host(&mut self, host_name: &str) {
        self.ssh_log.retain(|e| e.host_name != host_name);
    }

    pub fn reload_hosts(&mut self) -> Result<()> {
        let selected_name = self.selected_entry().map(|e| e.name().to_string());
        self.load_collapsed_groups();
        self.load_ui_zoom();

        sync_ssh_config_hosts(self.resolver.as_ref(), &self.store)?;

        let launcher_hosts = self.store.list_hosts_filtered(Some(HostSource::Launcher))?;
        let ssh_config_hosts = self
            .store
            .list_hosts_filtered(Some(HostSource::SshConfig))?;
        let db_names: std::collections::HashSet<String> = launcher_hosts
            .iter()
            .chain(ssh_config_hosts.iter())
            .map(|h| h.name.clone())
            .collect();

        let mut hosts: Vec<HostEntry> = launcher_hosts
            .into_iter()
            .chain(ssh_config_hosts)
            .map(HostEntry::from_managed)
            .collect();

        let config_names = self.resolver.list_hosts()?;
        self.metadata.ensure_defaults(&config_names)?;

        for name in config_names {
            if db_names.contains(&name) {
                continue;
            }
            let host = match self.resolver.resolve_host(&name) {
                Ok(host) => host,
                Err(_) => {
                    // Can't resolve this alias via `ssh -G`; skip it. We run
                    // under raw mode, so never write to stderr here (it would
                    // corrupt the TUI). The host simply won't be listed.
                    continue;
                }
            };
            let meta = self
                .metadata
                .get(&name)?
                .unwrap_or_else(|| crate::metadata::HostMetadata::new(&name));
            hosts.push(HostEntry::Legacy { host, meta });
        }

        self.hosts = hosts;
        self.groups = self.store.list_groups()?;
        self.rebuild_filter();
        if let Some(name) = selected_name {
            self.restore_selection_by_name(&name);
        }
        // Restart ping worker with updated host list (only if already running)
        if self.ping_rx.is_some() {
            self.start_ping_worker();
        }
        Ok(())
    }

    /// Current host-name column width in chars, driven by [`App::ui_zoom`].
    pub fn name_col_width(&self) -> usize {
        NAME_WIDTH_BASE + self.ui_zoom * NAME_WIDTH_STEP
    }

    /// Set the UI zoom level and persist it so it survives restarts.
    pub(crate) fn set_ui_zoom(&mut self, level: usize) {
        let level = level.min(UI_ZOOM_MAX);
        if level == self.ui_zoom {
            return;
        }
        self.ui_zoom = level;
        let _ = self.store.set_ui_state("ui_zoom", &level.to_string());
    }

    pub(crate) fn load_ui_zoom(&mut self) {
        // Fall back to the pre-rename "name_zoom" key so an upgraded user keeps
        // their previous zoom level.
        let raw = self
            .store
            .get_ui_state("ui_zoom")
            .ok()
            .flatten()
            .or_else(|| self.store.get_ui_state("name_zoom").ok().flatten());
        if let Some(level) = raw.and_then(|r| r.parse::<usize>().ok()) {
            self.ui_zoom = level.min(UI_ZOOM_MAX);
        }
    }

    pub fn store(&self) -> &LauncherStore {
        &self.store
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}
