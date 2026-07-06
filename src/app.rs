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

/// Host list sort mode (cycle with `s`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    Label,
    LastConnected,
    FavoriteFirst,
    GroupThenLabel,
    Manual,
}

impl SortMode {
    pub const ALL: [SortMode; 5] = [
        SortMode::Label,
        SortMode::LastConnected,
        SortMode::FavoriteFirst,
        SortMode::GroupThenLabel,
        SortMode::Manual,
    ];

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Label => "label",
            SortMode::LastConnected => "last connected",
            SortMode::FavoriteFirst => "favorite first",
            SortMode::GroupThenLabel => "group+label",
            SortMode::Manual => "manual",
        }
    }
}

/// Virtual group label for hosts without a DB group.
pub const UNGROUPED_LABEL: &str = "_ungrouped";

/// Collapsed-state key for the virtual "ungrouped" bucket (real group ids are
/// positive, so -1 never collides).
pub const UNGROUPED_KEY: i64 = -1;

/// One section in the group tree (real group or virtual ungrouped bucket).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostGroupSection {
    pub group: Option<HostGroup>,
    pub label: String,
    pub host_indices: Vec<usize>,
    /// Whether this section is collapsed (host rows hidden).
    pub collapsed: bool,
}

impl HostGroupSection {
    /// Stable collapse-state key: the group id, or [`UNGROUPED_KEY`].
    pub fn key(&self) -> i64 {
        self.group.as_ref().map(|g| g.id).unwrap_or(UNGROUPED_KEY)
    }
}

/// A selectable row in the hosts tree: either a group header or a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavRow {
    /// Index into `group_sections`.
    Header(usize),
    /// Index into `hosts`.
    Host(usize),
}

/// A rendered row in the hosts tree (superset of [`NavRow`] with blank
/// separators). The single source of truth for rendering, scrolling and click
/// mapping so they never drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualRow {
    /// Blank separator between sections.
    Blank,
    Header {
        section: usize,
        collapsed: bool,
        selected: bool,
    },
    Host {
        host_idx: usize,
        selected: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    Search,
    TagFilter,
    HostDetail,
    HostForm,
    IdentityForm,
    GroupForm,
    GroupManage,
    /// Dropdown over the host form's Group/Identity field.
    FieldPicker,
    /// Keybinding editor overlay.
    KeybindEditor,
    /// Quit confirmation dialog.
    ConfirmQuit,
    TunnelForm,
    ConfirmDelete,
    ConfirmDiscard,
    Help,
    Palette,
    ImportPrompt,
    /// Embedded session is spawning; ConnectScreen visible.
    Connecting,
    /// Live embedded SSH session; PTY drives the fullscreen view.
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuditFilter {
    #[default]
    All,
    Ok,
    Fail,
}

impl AuditFilter {
    pub fn next(self) -> Self {
        match self {
            AuditFilter::All => AuditFilter::Ok,
            AuditFilter::Ok => AuditFilter::Fail,
            AuditFilter::Fail => AuditFilter::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AuditFilter::All => "all",
            AuditFilter::Ok => "ok",
            AuditFilter::Fail => "fail",
        }
    }

    pub fn sql_status(self) -> Option<&'static str> {
        match self {
            AuditFilter::All => None,
            AuditFilter::Ok => Some("launched"),
            AuditFilter::Fail => Some("fail"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuditRange {
    #[default]
    All,
    Today,
    Week,
    Month,
}

impl AuditRange {
    pub fn next(self) -> Self {
        match self {
            AuditRange::All => AuditRange::Today,
            AuditRange::Today => AuditRange::Week,
            AuditRange::Week => AuditRange::Month,
            AuditRange::Month => AuditRange::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AuditRange::All => "all",
            AuditRange::Today => "24h",
            AuditRange::Week => "week",
            AuditRange::Month => "month",
        }
    }

    pub fn since_timestamp(self) -> Option<i64> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        match self {
            AuditRange::All => None,
            AuditRange::Today => Some(now - 86400),
            AuditRange::Week => Some(now - 7 * 86400),
            AuditRange::Month => Some(now - 30 * 86400),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelFormField {
    Type,
    LocalPort,
    RemoteHost,
    RemotePort,
    Host,
    Label,
}

impl TunnelFormField {
    const ALL: [TunnelFormField; 6] = [
        TunnelFormField::Host,
        TunnelFormField::Type,
        TunnelFormField::LocalPort,
        TunnelFormField::RemoteHost,
        TunnelFormField::RemotePort,
        TunnelFormField::Label,
    ];

    fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone)]
pub struct TunnelFormEdit {
    pub editing_id: Option<i64>,
    pub tunnel_type: crate::store::TunnelType,
    pub local_port: String,
    pub remote_host: String,
    pub remote_port: String,
    pub host_id: Option<i64>,
    pub label: String,
    pub active_field: TunnelFormField,
    pub editing: bool,
    pub edit_snapshot: String,
    pub dirty: bool,
}

/// Item pending confirmation before deletion.
#[derive(Debug, Clone)]
pub enum PendingDelete {
    Host { id: i64, name: String },
    Identity { id: i64, name: String },
    Group { id: i64, name: String },
    Tunnel { id: i64, label: String },
}

/// Editable metadata field index in [`AppMode::HostDetail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailEditField {
    #[default]
    Tags = 0,
    Description = 1,
    Environment = 2,
}

impl DetailEditField {
    const ALL: [DetailEditField; 3] = [
        DetailEditField::Tags,
        DetailEditField::Description,
        DetailEditField::Environment,
    ];

    fn next(self) -> Self {
        let idx = self as usize;
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let idx = self as usize;
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// In-progress metadata edits while in HostDetail mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostDetailEdit {
    pub tags: String,
    pub description: String,
    pub environment: String,
    pub field: DetailEditField,
    pub cursor: usize,
}

#[derive(Debug, Clone)]
pub enum HostEntry {
    Managed(ManagedHost),
    Legacy {
        host: SshHost,
        meta: crate::metadata::HostMetadata,
    },
}

impl HostEntry {
    pub fn new(host: SshHost) -> Self {
        let meta = crate::metadata::HostMetadata::new(host.name.clone());
        Self::Legacy { host, meta }
    }

    pub fn from_managed(managed: ManagedHost) -> Self {
        Self::Managed(managed)
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Managed(m) => &m.name,
            Self::Legacy { host, .. } => &host.name,
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            Self::Managed(m) => m.label.as_deref().unwrap_or(&m.name),
            Self::Legacy { host, .. } => &host.name,
        }
    }

    pub fn tags(&self) -> &[String] {
        match self {
            Self::Managed(m) => &m.tags,
            Self::Legacy { meta, .. } => &meta.tags,
        }
    }

    pub fn favorite(&self) -> bool {
        match self {
            Self::Managed(m) => m.favorite,
            Self::Legacy { meta, .. } => meta.favorite,
        }
    }

    pub fn last_connected(&self) -> Option<i64> {
        match self {
            Self::Managed(m) => m.last_connected,
            Self::Legacy { meta, .. } => meta.last_connected,
        }
    }

    pub fn description(&self) -> Option<&str> {
        match self {
            Self::Managed(m) => m.notes.as_deref(),
            Self::Legacy { meta, .. } => meta.description.as_deref(),
        }
    }

    pub fn environment(&self) -> Option<&str> {
        match self {
            Self::Managed(m) => m.environment.as_deref(),
            Self::Legacy { meta, .. } => meta.environment.as_deref(),
        }
    }

    pub fn source(&self) -> HostSource {
        match self {
            Self::Managed(m) => m.source,
            Self::Legacy { .. } => HostSource::SshConfig,
        }
    }

    pub fn is_launcher(&self) -> bool {
        matches!(self, Self::Managed(_))
    }

    pub fn managed_id(&self) -> Option<i64> {
        match self {
            Self::Managed(m) => Some(m.id),
            Self::Legacy { .. } => None,
        }
    }

    pub fn managed(&self) -> Option<&ManagedHost> {
        match self {
            Self::Managed(m) => Some(m),
            Self::Legacy { .. } => None,
        }
    }

    pub fn group_id(&self) -> Option<i64> {
        match self {
            Self::Managed(m) => m.group_id,
            Self::Legacy { .. } => None,
        }
    }

    pub fn sort_order(&self) -> i32 {
        match self {
            Self::Managed(m) => m.sort_order,
            Self::Legacy { .. } => i32::MAX,
        }
    }

    pub fn ssh_host(&self) -> SshHost {
        match self {
            Self::Managed(m) => managed_to_ssh_host(m),
            Self::Legacy { host, .. } => host.clone(),
        }
    }

    pub fn legacy_mut(&mut self) -> Option<(&mut SshHost, &mut crate::metadata::HostMetadata)> {
        match self {
            Self::Legacy { host, meta } => Some((host, meta)),
            Self::Managed(_) => None,
        }
    }
}

/// Look up the stored credential for a host entry and decide whether it's
/// a host password (sent at `password:` prompts) or an identity passphrase
/// (sent at `Enter passphrase for …`). Returns the pending secret and a
/// human-readable diagnostic line for the SSH log.
pub fn resolve_pending_secret(
    entry: &HostEntry,
    password_store: &dyn crate::credentials::PasswordStore,
) -> (Option<crate::session::PendingSecret>, String) {
    let Some(managed) = entry.managed() else {
        return (
            None,
            "auth: legacy ssh_config host — no stored credential".into(),
        );
    };

    if managed.has_password {
        let key = crate::credentials::host_key(managed.id);
        return match password_store.get(&key) {
            Ok(Some(pw)) => (
                Some(crate::session::PendingSecret::Password(pw)),
                format!("auth: using stored password ({key})"),
            ),
            Ok(None) => (
                None,
                format!(
                    "auth: has_password=true but keyring entry {key} is empty — ssh will prompt"
                ),
            ),
            Err(e) => (
                None,
                format!("auth: keyring lookup failed for {key}: {e:#} — ssh will prompt"),
            ),
        };
    }

    if let Some(identity) = managed.identity.as_ref() {
        if identity.has_password {
            let key = crate::credentials::identity_key(identity.id);
            // A secret on an identity WITH a key unlocks that key (passphrase);
            // on a keyless identity it's a shared login password, letting many
            // hosts reuse one user+password credential.
            let has_key = identity.private_key.is_some();
            return match password_store.get(&key) {
                Ok(Some(pw)) => (
                    Some(if has_key {
                        crate::session::PendingSecret::Passphrase(pw)
                    } else {
                        crate::session::PendingSecret::Password(pw)
                    }),
                    format!(
                        "auth: using stored {} ({key})",
                        if has_key { "passphrase" } else { "password" }
                    ),
                ),
                Ok(None) => (
                    None,
                    format!(
                        "auth: identity has_password=true but keyring entry {key} is empty — ssh will prompt"
                    ),
                ),
                Err(e) => (
                    None,
                    format!("auth: keyring lookup failed for {key}: {e:#} — ssh will prompt"),
                ),
            };
        }
    }

    (
        None,
        "auth: no stored credential — using agent / unlocked key / interactive prompt".into(),
    )
}

/// Capture host metadata used by the embedded session header + connect
/// animation.
pub fn session_meta_for_entry(entry: &HostEntry) -> crate::session::SessionMeta {
    match entry {
        HostEntry::Managed(m) => crate::session::SessionMeta {
            user: m
                .username
                .clone()
                .or_else(|| m.identity.as_ref().and_then(|i| i.username.clone())),
            address: Some(m.address.clone()),
            port: Some(m.port),
            identity: m
                .identity
                .as_ref()
                .and_then(|i| i.private_key.as_ref())
                .map(|p| p.to_string_lossy().into_owned()),
            proxy_jump: m.proxy_jump.clone(),
            host_id: Some(m.id),
        },
        HostEntry::Legacy { host, .. } => crate::session::SessionMeta {
            user: host.user.clone(),
            address: host.hostname.clone(),
            port: host.port,
            identity: host.identity_file.clone(),
            proxy_jump: host.proxy_jump.clone(),
            host_id: None,
        },
    }
}

/// Build the bare `ssh` argv for a host entry (no env / askpass prefix).
///
/// - Launcher-managed hosts: full options via `build_ssh_argv` so we don't
///   require an `~/.ssh/config` alias.
/// - SSH-config-sourced hosts: alias-only argv via `build_ssh_alias_argv` so
///   ssh inherits all options from the user's config.
/// - Legacy entries (ssh_config only, not in launcher DB): alias-only argv.
pub fn ssh_argv_for_entry(entry: &HostEntry) -> Vec<String> {
    match entry {
        HostEntry::Managed(m) => {
            let ssh_host = managed_to_ssh_host(m);
            if m.source == HostSource::SshConfig {
                crate::ssh::build_ssh_alias_argv(&ssh_host)
            } else {
                crate::ssh::build_ssh_argv(&ssh_host)
            }
        }
        HostEntry::Legacy { host, .. } => crate::ssh::build_ssh_alias_argv(host),
    }
}

fn managed_to_ssh_host(m: &ManagedHost) -> SshHost {
    let mut host = SshHost::new(&m.name);
    host.hostname = Some(m.address.clone());
    host.port = Some(m.port);
    host.user = m
        .username
        .clone()
        .or_else(|| m.identity.as_ref().and_then(|i| i.username.clone()));
    host.identity_file = m
        .identity
        .as_ref()
        .and_then(|i| i.private_key.as_ref())
        .map(|p| p.to_string_lossy().into_owned());
    host.certificate_file = m
        .identity
        .as_ref()
        .and_then(|i| i.certificate.as_ref())
        .map(|p| p.to_string_lossy().into_owned());
    host.proxy_jump = m.proxy_jump.clone();
    host.forward_agent = Some(m.forward_agent);
    host.remote_command = m.remote_command.clone();
    host
}

/// State of the keybinding editor overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeybindEditor {
    /// Index into [`KeyAction::ALL`].
    pub selected: usize,
    /// When true, the next key press is captured as a binding.
    pub capturing: bool,
    /// When capturing, whether to append (`true`) or replace (`false`).
    pub append: bool,
}

/// Which host-form field the dropdown is editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Group,
    Identity,
}

/// Dropdown overlay for the host form's Group/Identity picker fields.
///
/// For `Group`, the last row is a "+ New group…" affordance: selecting it
/// switches the overlay into inline text entry (`creating`) that creates the
/// group in the store and selects it — no trip to the group-manage screen.
#[derive(Debug, Clone)]
pub struct FieldPicker {
    pub kind: PickerKind,
    pub selected: usize,
    /// `Some(name)` while typing a brand-new group name inline.
    pub creating: Option<String>,
    pub cursor: usize,
}

/// In-progress host form while in [`AppMode::HostForm`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFormEdit {
    pub id: Option<i64>,
    pub address: String,
    pub username: String,
    pub label: String,
    pub name: String,
    pub port: String,
    pub group_index: usize,
    pub identity_index: usize,
    pub tags: String,
    pub proxy_jump: String,
    pub forward_agent: bool,
    pub remote_command: String,
    pub os_icon_index: usize,
    pub password: String,
    pub has_password: bool,
    pub field: HostFormField,
    pub cursor: usize,
    /// Connection fields (address/name/port) are read-only; only launcher metadata is saved.
    pub metadata_only: bool,
    /// When true, a per-field edit popup is open and keystrokes go to it.
    pub editing: bool,
    /// Snapshot of field value before editing (for cancel/revert).
    pub edit_snapshot: String,
    /// Whether any field has been modified since the form was opened.
    pub dirty: bool,
}

/// Editable host form field index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostFormField {
    #[default]
    Address = 0,
    Label = 1,
    Name = 2,
    Port = 3,
    Group = 4,
    Identity = 5,
    Tags = 6,
    ProxyJump = 7,
    ForwardAgent = 8,
    RemoteCommand = 9,
    OsIcon = 10,
    Password = 11,
    Username = 12,
}

pub const OS_ICON_OPTIONS: [&str; 4] = ["(none)", "generic", "ubuntu", "debian"];

impl HostFormField {
    pub const ALL: [HostFormField; 13] = [
        HostFormField::Address,
        HostFormField::Password,
        HostFormField::Username,
        HostFormField::Label,
        HostFormField::Name,
        HostFormField::Port,
        HostFormField::Group,
        HostFormField::Identity,
        HostFormField::Tags,
        HostFormField::ProxyJump,
        HostFormField::ForwardAgent,
        HostFormField::RemoteCommand,
        HostFormField::OsIcon,
    ];

    pub fn is_connection_field(self) -> bool {
        matches!(
            self,
            HostFormField::Address
                | HostFormField::Name
                | HostFormField::Port
                | HostFormField::ProxyJump
                | HostFormField::ForwardAgent
                | HostFormField::RemoteCommand
                | HostFormField::OsIcon
        )
    }

    fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            HostFormField::Address => "Address",
            HostFormField::Label => "Label",
            HostFormField::Name => "Name (alias)",
            HostFormField::Port => "Port",
            HostFormField::Group => "Group",
            HostFormField::Identity => "Identity",
            HostFormField::Tags => "Tags",
            HostFormField::ProxyJump => "ProxyJump",
            HostFormField::ForwardAgent => "Agent forward",
            HostFormField::RemoteCommand => "Startup command",
            HostFormField::OsIcon => "OS icon",
            HostFormField::Password => "Password",
            HostFormField::Username => "Username",
        }
    }

    fn is_picker(self) -> bool {
        matches!(
            self,
            HostFormField::Group | HostFormField::Identity | HostFormField::OsIcon
        )
    }

    fn is_toggle(self) -> bool {
        matches!(self, HostFormField::ForwardAgent)
    }
}

/// In-progress group form while in [`AppMode::GroupForm`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupFormEdit {
    pub id: Option<i64>,
    pub name: String,
    pub cursor: usize,
    /// Default identity new hosts in this group inherit. Cycled with ←/→.
    pub default_identity_id: Option<i64>,
    /// Return to GroupManage after save/cancel (vs Normal when opened from Ctrl+G shortcut).
    pub return_to_manage: bool,
}

/// Single-field path prompt for the Termius CSV import ([`AppMode::ImportPrompt`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportPromptEdit {
    /// Path to the Termius export directory (contains `L00t.csv`, `ssh_keys/`).
    pub path: String,
    pub cursor: usize,
    /// Feedback shown inside the popup (e.g. why the last attempt failed).
    pub error: Option<String>,
}

/// In-progress identity form while in [`AppMode::IdentityForm`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityFormEdit {
    pub id: Option<i64>,
    pub name: String,
    pub username: String,
    pub private_key: String,
    pub certificate: String,
    pub password: String,
    pub has_password: bool,
    /// Full key material pasted into the Private key field; written to
    /// `~/.ssh/sshub_<name>` on save (the path field then points at it).
    pub pasted_key: Option<String>,
    pub field: IdentityFormField,
    pub cursor: usize,
    pub editing: bool,
    pub edit_snapshot: String,
    pub dirty: bool,
}

/// Editable identity form field index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IdentityFormField {
    #[default]
    Name = 0,
    Username = 1,
    PrivateKey = 2,
    Certificate = 3,
    Password = 4,
}

impl IdentityFormField {
    pub const ALL: [IdentityFormField; 5] = [
        IdentityFormField::Name,
        IdentityFormField::Username,
        IdentityFormField::Password,
        IdentityFormField::PrivateKey,
        IdentityFormField::Certificate,
    ];

    fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            IdentityFormField::Name => "Name",
            IdentityFormField::Username => "Username",
            IdentityFormField::PrivateKey => "Private key path",
            IdentityFormField::Certificate => "Certificate path",
            IdentityFormField::Password => "Passphrase",
        }
    }
}

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
    pub tag_filter: Option<String>,
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
    pub import_prompt: Option<ImportPromptEdit>,
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
            tag_filter: None,
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
            import_prompt: None,
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

    pub fn reload_hosts(&mut self) -> Result<()> {
        let selected_name = self.selected_entry().map(|e| e.name().to_string());
        self.load_collapsed_groups();

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

    /// Import hosts from ssh config into the launcher store (`source=ssh_config`).
    pub fn import_ssh_config(&mut self) -> Result<ImportReport> {
        let report =
            import_ssh_config(self.resolver.as_ref(), &self.store, self.metadata.as_ref())?;
        self.reload_hosts()?;
        Ok(report)
    }

    /// Open the Termius CSV import prompt (asks for the export directory).
    pub fn open_import_prompt(&mut self) {
        let path = crate::import::termius_csv::default_export_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let cursor = path.chars().count();
        self.import_prompt = Some(ImportPromptEdit {
            path,
            cursor,
            error: None,
        });
        self.mode = AppMode::ImportPrompt;
    }

    fn import_prompt_insert(&mut self, ch: char) {
        if let Some(prompt) = self.import_prompt.as_mut() {
            prompt.cursor = text_input::insert_at(&mut prompt.path, prompt.cursor, ch);
            prompt.error = None;
        }
    }

    fn import_prompt_backspace(&mut self) {
        if let Some(prompt) = self.import_prompt.as_mut() {
            prompt.cursor = text_input::backspace_at(&mut prompt.path, prompt.cursor);
            prompt.error = None;
        }
    }

    fn handle_key_import_prompt(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.import_prompt = None;
                self.mode = AppMode::Normal;
            }
            KeyCode::Enter | KeyCode::F(2) => self.run_termius_import()?,
            KeyCode::Backspace if key.modifiers.is_empty() => self.import_prompt_backspace(),
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.import_prompt_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    /// Run the Termius CSV import using the path entered in the prompt.
    fn run_termius_import(&mut self) -> Result<()> {
        let Some(prompt) = self.import_prompt.as_ref() else {
            return Ok(());
        };
        let raw = prompt.path.trim();
        if raw.is_empty() {
            if let Some(p) = self.import_prompt.as_mut() {
                p.error = Some("Enter the Termius export folder path".into());
            }
            return Ok(());
        }

        // Accept a path pointing directly at L00t.csv by using its parent folder.
        let mut dir = shellexpand_home(raw);
        if dir.is_file() {
            if let Some(parent) = dir.parent() {
                dir = parent.to_path_buf();
            }
        }

        match crate::import::termius_csv::import_csv_export(
            &dir,
            &self.store,
            self.password_store.as_ref(),
        ) {
            Ok(report) => {
                let mut msg = format!(
                    "Termius: {} hosts new, {} skipped · {} passwords + {} passphrases stored",
                    report.hosts_imported,
                    report.skipped,
                    report.passwords_stored,
                    report.passphrases_stored,
                );
                if report.identities_created > 0 {
                    msg.push_str(&format!(" · {} new keys", report.identities_created));
                }
                if report.keyring_failures > 0 {
                    msg.push_str(&format!(
                        " · ⚠ {} keyring writes failed verification",
                        report.keyring_failures
                    ));
                }
                self.host_notice = Some(msg);
                self.import_prompt = None;
                self.mode = AppMode::Normal;
                self.reload_hosts()?;
            }
            Err(e) => {
                // Keep the prompt open and show why, so the user can fix the path.
                if let Some(p) = self.import_prompt.as_mut() {
                    p.error = Some(format!("{e:#}"));
                }
            }
        }
        Ok(())
    }

    /// Export launcher-native hosts to `config_dir/exported.conf`.
    pub fn export_ssh_config(&mut self) -> Result<std::path::PathBuf> {
        export_launcher_hosts(&self.store)
    }

    /// Launch SSH for the currently selected host and record last-connected time.
    pub fn connect_selected(&mut self) -> Result<()> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(());
        };

        // Determine the stored secret to feed ssh at the first prompt. A
        // host-level credential is sent at `password:`-style prompts; an
        // identity-level credential is sent at `Enter passphrase for …`.
        // The Session itself watches the PTY screen and types it once.
        let (pending_secret, credential_diag): (Option<crate::session::PendingSecret>, String) =
            resolve_pending_secret(&entry, self.password_store.as_ref());

        // Build ssh argv. The session hands a stored secret to ssh via
        // SSH_ASKPASS. When a secret is present, auto-accept a genuinely new
        // host key: otherwise ssh (with SSH_ASKPASS_REQUIRE=force) would ask
        // the askpass helper to confirm the fingerprint, get the password back
        // instead of "yes", and deadlock. Changed keys are still refused.
        let mut ssh_argv = ssh_argv_for_entry(&entry);
        if ssh_argv.first().map(String::as_str) == Some("ssh") {
            // `-v` streams ssh's real handshake into the session terminal, so
            // the connect screen shows the genuine process instead of a
            // scripted animation.
            ssh_argv.insert(1, "-v".into());
            if pending_secret.is_some() {
                ssh_argv.insert(1, "-o".into());
                ssh_argv.insert(2, "StrictHostKeyChecking=accept-new".into());
            }
        }

        // Surface the credential decision so it's visible in the SSH log
        // panel after the session ends.
        {
            let level = if pending_secret.is_some() {
                crate::ssh::probe::LogLevel::Success
            } else {
                crate::ssh::probe::LogLevel::Info
            };
            self.push_ssh_log(crate::ssh::probe::SshLogEntry {
                host_name: entry.name().to_string(),
                line: credential_diag,
                level,
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
            });
        }

        // Pre-validate: check that the first command binary exists on PATH
        if let Some(first_cmd) = ssh_argv.first() {
            if std::process::Command::new("which")
                .arg(first_cmd)
                .output()
                .map(|o| !o.status.success())
                .unwrap_or(true)
            {
                let msg = format!(
                    "Command not found: '{}'. Check your PATH or install it.",
                    first_cmd
                );
                self.push_ssh_log(crate::ssh::probe::SshLogEntry {
                    host_name: entry.name().to_string(),
                    line: msg.clone(),
                    level: crate::ssh::probe::LogLevel::Error,
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64,
                });
                self.host_notice = Some(msg);
                return Ok(());
            }
        }

        // Log the actual command being run to ssh_log
        let now_ts_val = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.push_ssh_log(crate::ssh::probe::SshLogEntry {
            host_name: entry.name().to_string(),
            line: format!("$ {}", ssh_argv.join(" ")),
            level: crate::ssh::probe::LogLevel::Success,
            timestamp: now_ts_val,
        });

        // Log auth event based on launch result
        {
            let host_name = entry.name().to_string();
            let username = entry.managed().and_then(|m| {
                m.username
                    .as_deref()
                    .or_else(|| m.identity.as_ref().and_then(|i| i.username.as_deref()))
            });
            let via = entry
                .managed()
                .and_then(|m| m.proxy_jump.as_deref())
                .unwrap_or("direct");
            // Spawn an embedded PTY session in-process. No external terminal.
            let display_name = entry.name().to_string();
            let rows = self.terminal_area.height.max(3);
            let cols = self.terminal_area.width.max(20);
            let meta = session_meta_for_entry(&entry);
            let config = crate::session::SessionConfig {
                argv: ssh_argv.clone(),
                display_name,
                meta,
                pending_secret: pending_secret.clone(),
            };
            match crate::session::Session::spawn(config, rows, cols) {
                Ok(session) => {
                    self.sessions.push(session);
                    self.active_session = Some(self.sessions.len() - 1);
                    self.mode = AppMode::Connecting;
                    let _ = self.store.log_auth_event(
                        &host_name,
                        username,
                        via,
                        "launched",
                        "session started",
                    );
                }
                Err(e) => {
                    let err_msg = format!("Session spawn failed: {e:#}");
                    let _ = self
                        .store
                        .log_auth_event(&host_name, username, via, "fail", &err_msg);
                    self.push_ssh_log(crate::ssh::probe::SshLogEntry {
                        host_name: host_name.clone(),
                        line: err_msg.clone(),
                        level: crate::ssh::probe::LogLevel::Error,
                        timestamp: now_ts_val,
                    });
                    self.host_notice = Some(err_msg);
                    return Ok(());
                }
            }
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        if let Some(id) = entry.managed_id() {
            self.store.set_host_last_connected(id, timestamp)?;
            if let Some(idx) = self.hosts.iter().position(|e| e.managed_id() == Some(id)) {
                if let HostEntry::Managed(m) = &mut self.hosts[idx] {
                    m.last_connected = Some(timestamp);
                }
            }
        } else {
            self.metadata.set_last_connected(entry.name(), timestamp)?;
            if let Some(idx) = self.hosts.iter().position(|e| e.name() == entry.name()) {
                if let Some((_, meta)) = self.hosts[idx].legacy_mut() {
                    meta.last_connected = Some(timestamp);
                }
            }
        }

        // Force-refresh auth cache immediately after logging the event
        self.auth_cache_updated = std::time::Instant::now() - std::time::Duration::from_secs(60);
        self.refresh_auth_cache();

        Ok(())
    }

    /// Handle a keyboard event according to architecture keybindings.
    /// Handle a bracketed-paste event. Pasted text is delivered as one blob
    /// (not per-key), so multi-line content — e.g. a private key — no longer
    /// fires Enter/save mid-field and spills the rest as commands.
    pub fn handle_paste(&mut self, text: &str) -> Result<()> {
        // Embedded session: forward the paste straight to the remote PTY.
        if matches!(self.mode, AppMode::Session | AppMode::Connecting) {
            if let Some(s) = self.active_session_mut() {
                let _ = s.write(text.as_bytes());
            }
            return Ok(());
        }

        // Only insert into modes that own a focused text field. Everywhere else
        // a paste is meaningless and must NOT be run as commands.
        let text_entry = matches!(
            self.mode,
            AppMode::HostForm
                | AppMode::IdentityForm
                | AppMode::GroupForm
                | AppMode::TunnelForm
                | AppMode::HostDetail
                | AppMode::Search
                | AppMode::TagFilter
                | AppMode::Palette
                | AppMode::ImportPrompt
        );
        if !text_entry {
            return Ok(());
        }

        // Pasting key material into the identity "Private key path" field:
        // keep the full multi-line blob and write it to a key file on save.
        if self.mode == AppMode::IdentityForm
            && crate::ssh::looks_like_private_key(text)
            && self
                .identity_form
                .as_ref()
                .is_some_and(|f| f.field == IdentityFormField::PrivateKey)
        {
            if let Some(form) = self.identity_form.as_mut() {
                form.pasted_key = Some(text.to_string());
                form.private_key = "(pasted key — saved to ~/.ssh on save)".to_string();
                form.cursor = text_input::char_len(&form.private_key);
                form.dirty = true;
            }
            return Ok(());
        }

        // Feed printable characters through the normal typing path (reusing the
        // field's insert logic); drop newlines/tabs since all fields are
        // single-line.
        for ch in text.chars() {
            if ch.is_control() {
                continue;
            }
            self.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty()))?;
        }
        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        // When an embedded session is active, Ctrl+C inside the terminal must
        // reach the remote shell — not quit sshub. Session mode intercepts all
        // keys (except Ctrl+D / Esc, which end the session) before this check.
        if matches!(self.mode, AppMode::Connecting | AppMode::Session) {
            return self.handle_key_session(key);
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            // First Ctrl+C asks for confirmation (if enabled); a second Ctrl+C
            // while the dialog is up forces the quit.
            if self.mode == AppMode::ConfirmQuit || !self.config.appearance.confirm_quit {
                self.should_quit = true;
            } else {
                self.pre_quit_mode = Some(self.mode);
                self.mode = AppMode::ConfirmQuit;
            }
            return Ok(());
        }

        // Ctrl+K opens the keybinding editor from any normal navigation screen.
        if self.mode == AppMode::Normal
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('k') | KeyCode::Char('K'))
        {
            self.keybind_editor = Some(KeybindEditor {
                selected: 0,
                capturing: false,
                append: false,
            });
            self.mode = AppMode::KeybindEditor;
            return Ok(());
        }

        match self.mode {
            AppMode::KeybindEditor => self.handle_key_keybind_editor(key),
            AppMode::ConfirmQuit => self.handle_key_confirm_quit(key),
            AppMode::Help => self.handle_key_help(key),
            AppMode::ConfirmDiscard => self.handle_key_confirm_discard(key),
            AppMode::ConfirmDelete => self.handle_key_confirm_delete(key),
            AppMode::HostForm => self.handle_key_host_form(key),
            AppMode::IdentityForm => self.handle_key_identity_form(key),
            AppMode::GroupForm => self.handle_key_group_form(key),
            AppMode::FieldPicker => self.handle_key_field_picker(key),
            AppMode::ImportPrompt => self.handle_key_import_prompt(key),
            AppMode::GroupManage => self.handle_key_group_manage(key),
            AppMode::Palette => self.handle_key_palette(key),
            AppMode::Search => self.handle_key_search(key),
            AppMode::TagFilter => self.handle_key_tag_filter(key),
            AppMode::HostDetail => self.handle_key_host_detail(key),
            AppMode::TunnelForm => self.handle_key_tunnel_form(key),
            AppMode::Connecting | AppMode::Session => self.handle_key_session(key),
            AppMode::Normal => match self.active_tab {
                1 => self.handle_key_tunnels(key),
                2 => self.handle_key_keychain(key),
                3 => self.handle_key_audit(key),
                _ => self.handle_key_normal(key),
            },
        }
    }

    /// Handle a mouse event — clicks, scroll wheel, etc.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        if matches!(self.mode, AppMode::Connecting | AppMode::Session) {
            self.handle_mouse_session(mouse);
            return Ok(());
        }
        if self.mode != AppMode::Normal {
            return Ok(());
        }

        let areas = crate::tui::dashboard_layout::dashboard_layout(self.terminal_area);
        let x = mouse.column;
        let y = mouse.row;

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let now = std::time::Instant::now();
                let is_double = self
                    .last_click
                    .map(|(t, lx, ly)| {
                        now.duration_since(t).as_millis() < 400
                            && x.abs_diff(lx) <= 1
                            && y.abs_diff(ly) <= 1
                    })
                    .unwrap_or(false);
                self.last_click = Some((now, x, y));

                // Tab bar clicks
                if y == areas.tab_bar.y {
                    if let Some(tab) = tab_from_x(x) {
                        match tab {
                            0 => self.active_tab = 0,
                            1 => self.switch_to_tunnels_tab()?,
                            2 => self.switch_to_keys_tab()?,
                            3 => {
                                self.active_tab = 3;
                                self.refresh_audit_events();
                            }
                            _ => {}
                        }
                    }
                    return Ok(());
                }

                // Body area clicks
                if y >= areas.body.y && y < areas.body.y + areas.body.height {
                    match self.active_tab {
                        0 => {
                            // Host list panel
                            if x >= areas.col_left.x && x < areas.col_left.x + areas.col_left.width
                            {
                                let content_y = areas.col_left.y + 1;
                                // Mirror the panel's body height: ch = height-2,
                                // body reserves 2 rows for its footer.
                                let ch = areas.col_left.height.saturating_sub(2);
                                let body_h = if ch > 2 { ch - 2 } else { ch } as usize;
                                if y >= content_y {
                                    let rel = y - content_y;
                                    if let Some(idx) = self.host_row_to_index(rel, body_h) {
                                        if let Some(pos) = self.nav_rows.iter().position(
                                            |r| matches!(r, NavRow::Host(i) if *i == idx),
                                        ) {
                                            self.selected = pos;
                                            if is_double {
                                                self.connect_selected()?;
                                            }
                                        }
                                    } else if let Some(si) = self.host_row_to_header(rel, body_h) {
                                        // Click on a group header toggles collapse.
                                        self.toggle_group_by_section(si);
                                    }
                                }
                            }
                        }
                        1 => {
                            // Tunnels table — account for scroll offset
                            let data_y = areas.body.y + 4;
                            if y >= data_y {
                                let visible_row = (y - data_y) as usize;
                                let max_rows = (areas.body.y + areas.body.height)
                                    .saturating_sub(data_y)
                                    as usize;
                                let scroll = if self.tunnel_selected >= max_rows {
                                    self.tunnel_selected - max_rows + 1
                                } else {
                                    0
                                };
                                let idx = scroll + visible_row;
                                if idx < self.tunnels.len() {
                                    self.tunnel_selected = idx;
                                    if is_double {
                                        self.toggle_tunnel()?;
                                    }
                                }
                            }
                        }
                        2 => {
                            // Keys cards
                            if !self.identities.is_empty() {
                                let inner_w =
                                    crate::tui::screens::keys::inner_width(areas.body.width);
                                let cards_per_row = crate::tui::screens::keys::resolve_columns(
                                    inner_w,
                                    self.config.appearance.identity_columns,
                                );
                                let card_h = 6u16;
                                let rel_y = y.saturating_sub(areas.body.y);
                                let row_idx = rel_y / (card_h + 1);
                                let col_idx = if cards_per_row > 1 {
                                    let card_w =
                                        inner_w.saturating_sub((cards_per_row as u16 - 1) * 2)
                                            / cards_per_row as u16;
                                    let margin = if areas.body.width >= 132 {
                                        2
                                    } else if areas.body.width >= 80 {
                                        1
                                    } else {
                                        0
                                    };
                                    let rel_x = x.saturating_sub(areas.body.x + margin);
                                    (rel_x / (card_w + 2)).min(cards_per_row as u16 - 1)
                                } else {
                                    0
                                };
                                let row_offset = self.keys_scroll_row_offset(
                                    areas.body.height,
                                    cards_per_row,
                                    card_h + 1,
                                );
                                let idx = (row_idx as usize + row_offset) * cards_per_row
                                    + col_idx as usize;
                                if idx < self.identities.len() {
                                    self.identity_selected = idx;
                                }
                            }
                        }
                        3 => {
                            // Audit table (mirror the renderer's scroll math)
                            let data_y = areas.body.y + 3;
                            if y >= data_y {
                                let max_rows = (areas.body.y + areas.body.height)
                                    .saturating_sub(data_y)
                                    as usize;
                                let scroll = if max_rows > 0 && self.audit_selected >= max_rows {
                                    self.audit_selected - max_rows + 1
                                } else {
                                    0
                                };
                                let row = (y - data_y) as usize + scroll;
                                if row < self.auth_events_cache.len() {
                                    self.audit_selected = row;
                                }
                            }

                            // Filter strip clicks (row 0 of audit area)
                            if y == areas.body.y {
                                self.handle_audit_filter_click(x, areas.body.x)?;
                            }
                        }
                        _ => {}
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                if y >= areas.body.y && y < areas.body.y + areas.body.height {
                    match self.active_tab {
                        0 => {
                            if x >= areas.col_left.x && x < areas.col_left.x + areas.col_left.width
                            {
                                self.selected = self.selected.saturating_sub(3);
                            } else {
                                self.ssh_log_scroll = self.ssh_log_scroll.saturating_add(3);
                            }
                        }
                        1 => {
                            self.tunnel_selected = self.tunnel_selected.saturating_sub(1);
                        }
                        2 => {
                            self.identity_selected = self.identity_selected.saturating_sub(1);
                        }
                        3 => {
                            self.audit_selected = self.audit_selected.saturating_sub(1);
                        }
                        _ => {}
                    }
                }
            }
            MouseEventKind::ScrollDown
                if y >= areas.body.y && y < areas.body.y + areas.body.height =>
            {
                match self.active_tab {
                    0 => {
                        if x >= areas.col_left.x && x < areas.col_left.x + areas.col_left.width {
                            let max = self.filtered_indices.len().saturating_sub(1);
                            self.selected = (self.selected + 3).min(max);
                        } else {
                            self.ssh_log_scroll = self.ssh_log_scroll.saturating_sub(3);
                        }
                    }
                    1 => {
                        let max = self.tunnels.len().saturating_sub(1);
                        self.tunnel_selected = (self.tunnel_selected + 1).min(max);
                    }
                    2 => {
                        let max = self.identities.len().saturating_sub(1);
                        self.identity_selected = (self.identity_selected + 1).min(max);
                    }
                    3 => {
                        let max = self.auth_events_cache.len().saturating_sub(1);
                        self.audit_selected = (self.audit_selected + 1).min(max);
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_audit_filter_click(&mut self, click_x: u16, body_x: u16) -> Result<()> {
        let margin = if self.terminal_area.width >= 132 {
            2
        } else if self.terminal_area.width >= 80 {
            1
        } else {
            0
        };
        let base_x = body_x + margin;

        // "filter: " = 8 chars
        let mut cx = base_x + 8;
        for f in [AuditFilter::All, AuditFilter::Ok, AuditFilter::Fail] {
            let label_len = f.label().len() as u16;
            if click_x >= cx && click_x < cx + label_len {
                self.audit_filter = f;
                self.refresh_audit_events();
                return Ok(());
            }
            cx += label_len + 2;
        }

        // "  range: " gap
        cx += 2 + 7;
        for r in [
            AuditRange::All,
            AuditRange::Today,
            AuditRange::Week,
            AuditRange::Month,
        ] {
            let label_len = r.label().len() as u16;
            if click_x >= cx && click_x < cx + label_len {
                self.audit_range = r;
                self.refresh_audit_events();
                return Ok(());
            }
            cx += label_len + 2;
        }

        Ok(())
    }

    /// Map a Y offset (relative to hosts panel content area) to a host index,
    /// accounting for group headers and blank separators.
    /// Flattened host-tree layout: total visual rows (group headers + blank
    /// separators + host rows) and the visual row of the selected host.
    pub fn host_visual_layout(&self) -> (usize, Option<usize>) {
        let rows = self.host_visual_rows();
        let sel = rows.iter().position(|r| {
            matches!(
                r,
                VisualRow::Header { selected: true, .. } | VisualRow::Host { selected: true, .. }
            )
        });
        (rows.len(), sel)
    }

    /// Scroll offset (in visual rows) for a host panel of `body_h` rows that
    /// keeps the selected host roughly centered and on screen.
    pub fn host_scroll_offset(&self, body_h: usize) -> usize {
        if body_h == 0 {
            return 0;
        }
        let (total, sel) = self.host_visual_layout();
        let max_offset = total.saturating_sub(body_h);
        match sel {
            Some(s) => s.saturating_sub(body_h / 2).min(max_offset),
            None => 0,
        }
    }

    /// Scroll offset, in whole card-rows, for the keys tab. Keeps the selected
    /// identity card on screen (roughly centered) when the grid overflows.
    /// `card_row_stride` is the height of one card row (card height + gap).
    pub fn keys_scroll_row_offset(
        &self,
        area_height: u16,
        cards_per_row: usize,
        card_row_stride: u16,
    ) -> usize {
        let cpr = cards_per_row.max(1);
        let stride = card_row_stride.max(1) as usize;
        let total_rows = self.identities.len().div_ceil(cpr);
        let visible_rows = ((area_height as usize) / stride).max(1);
        let selected_row = self.identity_selected / cpr;
        let max_off = total_rows.saturating_sub(visible_rows);
        selected_row.saturating_sub(visible_rows / 2).min(max_off)
    }

    /// Map a click at visible row `rel_y` (within a `body_h`-row panel) to the
    /// host index under it, accounting for the current scroll offset.
    fn host_row_to_index(&self, rel_y: u16, body_h: usize) -> Option<usize> {
        let target = rel_y as usize + self.host_scroll_offset(body_h);
        match self.host_visual_rows().get(target) {
            Some(VisualRow::Host { host_idx, .. }) => Some(*host_idx),
            _ => None,
        }
    }

    /// Map a click at visible row `rel_y` to a group-header section index (for
    /// click-to-collapse), accounting for the current scroll offset.
    fn host_row_to_header(&self, rel_y: u16, body_h: usize) -> Option<usize> {
        let target = rel_y as usize + self.host_scroll_offset(body_h);
        match self.host_visual_rows().get(target) {
            Some(VisualRow::Header { section, .. }) => Some(*section),
            _ => None,
        }
    }

    pub fn selected_host_index(&self) -> Option<usize> {
        match self.nav_rows.get(self.selected) {
            Some(NavRow::Host(i)) => Some(*i),
            _ => None,
        }
    }

    /// The full rendered layout of the hosts tree: blank separators, group
    /// headers and host rows, with per-row selection state. Single source of
    /// truth shared by rendering, scroll math and click mapping.
    pub fn host_visual_rows(&self) -> Vec<VisualRow> {
        let mut rows = Vec::new();
        let mut nav_idx = 0usize;
        for (si, section) in self.group_sections.iter().enumerate() {
            if si > 0 {
                rows.push(VisualRow::Blank);
            }
            let has_header = self
                .nav_rows
                .iter()
                .any(|r| matches!(r, NavRow::Header(s) if *s == si));
            if has_header {
                rows.push(VisualRow::Header {
                    section: si,
                    collapsed: section.collapsed,
                    selected: self.selected == nav_idx,
                });
                nav_idx += 1;
            }
            if !section.collapsed {
                for &host_idx in &section.host_indices {
                    rows.push(VisualRow::Host {
                        host_idx,
                        selected: self.selected == nav_idx,
                    });
                    nav_idx += 1;
                }
            }
        }
        rows
    }

    pub fn selected_entry(&self) -> Option<&HostEntry> {
        let host_idx = self.selected_host_index()?;
        self.hosts.get(host_idx)
    }

    fn handle_key_normal(&mut self, key: KeyEvent) -> Result<()> {
        self.host_notice = None;

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_host_manual(-1)?
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_host_manual(1)?
            }
            KeyCode::Char('j') | KeyCode::Down if key.modifiers.is_empty() => {
                self.move_selection(1)
            }
            KeyCode::Char('k') | KeyCode::Up if key.modifiers.is_empty() => self.move_selection(-1),
            KeyCode::Esc if key.modifiers.is_empty() && self.tag_filter.is_some() => {
                self.tag_filter = None;
                self.search_query.clear();
                self.rebuild_filter();
            }
            // Collapse/expand the group under the selection.
            KeyCode::Char(' ') if key.modifiers.is_empty() => self.toggle_selected_group(),
            KeyCode::Left if key.modifiers.is_empty() => {
                if self.selected_nav_header().is_some_and(|si| {
                    !self.group_sections[si].collapsed
                }) {
                    self.toggle_selected_group();
                }
            }
            KeyCode::Right if key.modifiers.is_empty() => {
                if self
                    .selected_nav_header()
                    .is_some_and(|si| self.group_sections[si].collapsed)
                {
                    self.toggle_selected_group();
                }
            }
            KeyCode::Char('Z') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                // Collapse all, or expand all if everything is already collapsed.
                let all_collapsed = !self.group_sections.is_empty()
                    && self.group_sections.iter().all(|s| s.collapsed);
                self.set_all_groups_collapsed(!all_collapsed);
            }
            // Enter on a group header toggles it; on a host it connects.
            KeyCode::Enter if self.selected_nav_header().is_some() => {
                self.toggle_selected_group()
            }
            KeyCode::Enter => self.connect_selected()?,
            _ if self.is_action(KeyAction::AddHost, &key) => self.enter_host_form(None, false)?,
            _ if self.is_action(KeyAction::Delete, &key) => self.delete_selected_host()?,
            _ if self.is_action(KeyAction::Duplicate, &key) => self.duplicate_selected_host()?,
            KeyCode::Char('E') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                match self.export_ssh_config() {
                    Ok(path) => {
                        let count = self
                            .store
                            .list_hosts_filtered(Some(HostSource::Launcher))
                            .map(|h| h.len())
                            .unwrap_or(0);
                        self.host_notice =
                            Some(format!("Exported {count} host(s) to {}", path.display()));
                    }
                    Err(e) => self.host_notice = Some(format!("Export failed: {e:#}")),
                }
            }
            KeyCode::Char('I') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                match self.import_ssh_config() {
                    Ok(report) => {
                        let mut msg = format!(
                            "Imported {} new, {} updated, {} skipped",
                            report.inserted, report.updated, report.skipped_launcher
                        );
                        if report.failed > 0 {
                            msg.push_str(&format!(", {} failed", report.failed));
                        }
                        self.host_notice = Some(msg);
                    }
                    Err(e) => self.host_notice = Some(format!("Import failed: {e:#}")),
                }
            }
            KeyCode::Char('T') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.open_import_prompt();
            }
            KeyCode::Char('e') if key.modifiers.is_empty() => self.edit_selected_host()?,
            KeyCode::Char('f') if key.modifiers.is_empty() => self.toggle_favorite()?,
            KeyCode::Tab => self.detail_focus = !self.detail_focus,
            _ if self.is_action(KeyAction::Search, &key) => {
                self.palette_query.clear();
                self.palette_selected = 0;
                self.palette_results = (0..self.hosts.len()).collect();
                self.mode = AppMode::Palette;
            }
            _ if self.is_action(KeyAction::Help, &key) => {
                self.pre_help_mode = Some(self.mode);
                self.mode = AppMode::Help;
            }
            _ if self.is_action(KeyAction::TagFilter, &key) => {
                self.mode = AppMode::TagFilter;
                self.search_query.clear();
            }
            KeyCode::Char('c') if key.modifiers.is_empty() => {
                self.ssh_log.clear();
                self.ssh_log_scroll = 0;
                // The periodic ssh probe was removed — the receiver is left
                // around for the type signature but never produces anything.
                self.probe_rx = None;
                self.host_notice = Some("SSH log cleared.".into());
            }
            KeyCode::Char('i') if key.modifiers.is_empty() => self.switch_to_keys_tab()?,
            KeyCode::Char('h') if key.modifiers.is_empty() => self.active_tab = 0,
            KeyCode::Char('1') if key.modifiers.is_empty() => self.active_tab = 0,
            KeyCode::Char('2') if key.modifiers.is_empty() => self.switch_to_tunnels_tab()?,
            KeyCode::Char('3') if key.modifiers.is_empty() => self.switch_to_keys_tab()?,
            KeyCode::Char('4') if key.modifiers.is_empty() => {
                self.active_tab = 3;
                self.refresh_audit_events();
            }
            KeyCode::Char('s') if key.modifiers.is_empty() => self.cycle_sort_mode(),
            KeyCode::Char('y') if key.modifiers.is_empty() => self.yank_ssh_log()?,
            KeyCode::Char('g' | 'G')
                if key
                    .modifiers
                    .contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
            {
                self.delete_selected_host_group()?
            }
            KeyCode::Char('g' | 'G')
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.enter_group_manage()?
            }
            KeyCode::Char('g' | 'G')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.rename_selected_host_group()?
            }
            // Unmatched chars open the fuzzy palette instead of legacy search
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.palette_query.clear();
                self.palette_query.push(c);
                self.palette_selected = 0;
                self.rebuild_palette_results();
                self.mode = AppMode::Palette;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_key_palette(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Enter => {
                if let Some(&idx) = self.palette_results.get(self.palette_selected) {
                    // The palette searches ALL hosts; the chosen one may be
                    // hidden by an active tag/search filter. Clear filters in
                    // that case — never silently connect to a different host.
                    let pos = self.filtered_indices.iter().position(|&i| i == idx);
                    let pos = match pos {
                        Some(p) => Some(p),
                        None => {
                            self.tag_filter = None;
                            self.search_query.clear();
                            self.rebuild_filter();
                            self.filtered_indices.iter().position(|&i| i == idx)
                        }
                    };
                    self.mode = AppMode::Normal;
                    if let Some(p) = pos {
                        // First Enter selects the host in the list; connecting
                        // is a deliberate second Enter from Normal mode.
                        self.selected = p;
                    }
                } else {
                    self.mode = AppMode::Normal;
                }
            }
            KeyCode::Up => {
                if self.palette_selected > 0 {
                    self.palette_selected -= 1;
                }
            }
            KeyCode::Down => {
                if self.palette_selected + 1 < self.palette_results.len() {
                    self.palette_selected += 1;
                }
            }
            KeyCode::Backspace => {
                self.palette_query.pop();
                self.rebuild_palette_results();
            }
            KeyCode::Char(c) if !c.is_control() => {
                self.palette_query.push(c);
                self.rebuild_palette_results();
            }
            _ => {}
        }
        Ok(())
    }

    fn rebuild_palette_results(&mut self) {
        // nucleo fuzzy match (same engine as list search) — the palette is
        // advertised as fuzzy, so typos and abbreviations must match too.
        self.palette_results = self.search.update_query(&self.hosts, &self.palette_query);
        if self.palette_selected >= self.palette_results.len() {
            self.palette_selected = 0;
        }
    }

    fn handle_key_search(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.exit_search(true),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Enter => self.connect_selected()?,
            KeyCode::Backspace => {
                self.search_query.pop();
                self.rebuild_filter();
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.search_query.push(c);
                self.rebuild_filter();
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_key_tag_filter(&mut self, key: KeyEvent) -> Result<()> {
        if key.code == KeyCode::Esc {
            self.tag_filter = None;
            self.search_query.clear();
            self.mode = AppMode::Normal;
            self.rebuild_filter();
            return Ok(());
        }
        if key.code == KeyCode::Enter {
            self.tag_filter = optional_field(&self.search_query);
            self.rebuild_filter();
            self.mode = AppMode::Normal;
            return Ok(());
        }
        if key.code == KeyCode::Backspace {
            self.search_query.pop();
            return Ok(());
        }
        if let KeyCode::Char(c) = key.code {
            if key.modifiers.is_empty() && !c.is_control() {
                self.search_query.push(c);
            }
        }
        Ok(())
    }

    fn handle_key_host_detail(&mut self, key: KeyEvent) -> Result<()> {
        if self.detail_edit.is_none() {
            return Ok(());
        }

        match key.code {
            KeyCode::Esc => self.cancel_host_detail()?,
            KeyCode::Enter => self.save_host_detail()?,
            KeyCode::Char('f') if key.modifiers.is_empty() => self.toggle_favorite()?,
            KeyCode::Tab if key.modifiers.is_empty() => self.detail_edit_field_next(),
            KeyCode::BackTab => self.detail_edit_field_prev(),
            KeyCode::Char('j') | KeyCode::Down if key.modifiers.is_empty() => {
                self.detail_edit_field_next()
            }
            KeyCode::Char('k') | KeyCode::Up if key.modifiers.is_empty() => {
                self.detail_edit_field_prev()
            }
            KeyCode::Backspace if key.modifiers.is_empty() => self.detail_edit_backspace(),
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.detail_edit_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_key_keychain(&mut self, key: KeyEvent) -> Result<()> {
        self.identity_notice = None;

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            KeyCode::Char('h') if key.modifiers.is_empty() => {
                self.active_tab = 0;
            }
            KeyCode::Char('1') if key.modifiers.is_empty() => {
                self.active_tab = 0;
            }
            KeyCode::Char('2') if key.modifiers.is_empty() => self.switch_to_tunnels_tab()?,
            KeyCode::Char('3') if key.modifiers.is_empty() => self.switch_to_keys_tab()?,
            KeyCode::Char('4') if key.modifiers.is_empty() => {
                self.active_tab = 3;
                self.refresh_audit_events();
            }
            KeyCode::Esc if key.modifiers.is_empty() => {
                self.active_tab = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_identity_grid(1, 0),
            KeyCode::Char('k') | KeyCode::Up => self.move_identity_grid(-1, 0),
            KeyCode::Char('l') | KeyCode::Right => self.move_identity_grid(0, 1),
            KeyCode::Left => self.move_identity_grid(0, -1),
            KeyCode::Char(']') if key.modifiers.is_empty() => self.adjust_identity_columns(1),
            KeyCode::Char('[') if key.modifiers.is_empty() => self.adjust_identity_columns(-1),
            KeyCode::Char('a') if key.modifiers.is_empty() => self.enter_identity_form(None)?,
            KeyCode::Char('e') if key.modifiers.is_empty() => self.edit_selected_identity()?,
            KeyCode::Char('d') if key.modifiers.is_empty() => self.delete_selected_identity()?,
            KeyCode::Char('r') if key.modifiers.is_empty() => self.remove_selected_from_agent()?,
            KeyCode::Char('p') if key.modifiers.is_empty() => self.add_selected_to_agent()?,
            _ if self.is_action(KeyAction::Help, &key) => {
                self.pre_help_mode = Some(self.mode);
                self.mode = AppMode::Help;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_key_audit(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.auth_events_cache.is_empty() {
                    self.audit_selected =
                        (self.audit_selected + 1).min(self.auth_events_cache.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.audit_selected = self.audit_selected.saturating_sub(1);
            }
            KeyCode::Char('f') if key.modifiers.is_empty() => {
                self.audit_filter = self.audit_filter.next();
                self.audit_selected = 0;
                self.refresh_audit_events();
            }
            KeyCode::Char('r') if key.modifiers.is_empty() => {
                self.audit_range = self.audit_range.next();
                self.audit_selected = 0;
                self.refresh_audit_events();
            }
            KeyCode::Char('1') if key.modifiers.is_empty() => self.active_tab = 0,
            KeyCode::Char('2') if key.modifiers.is_empty() => self.switch_to_tunnels_tab()?,
            KeyCode::Char('3') if key.modifiers.is_empty() => self.switch_to_keys_tab()?,
            KeyCode::Char('4') if key.modifiers.is_empty() => {
                self.active_tab = 3;
                self.refresh_audit_events();
            }
            KeyCode::Char('h') if key.modifiers.is_empty() => self.active_tab = 0,
            _ if self.is_action(KeyAction::Help, &key) => {
                self.pre_help_mode = Some(self.mode);
                self.mode = AppMode::Help;
            }
            _ => {}
        }
        Ok(())
    }

    pub fn refresh_agent_info(&mut self) {
        if self.agent_info_updated.elapsed() > std::time::Duration::from_secs(30) {
            self.agent_info = Some(crate::ssh::agent::detect_agent());
            self.agent_info_updated = std::time::Instant::now();
        }
    }

    fn switch_to_tunnels_tab(&mut self) -> Result<()> {
        self.active_tab = 1;
        self.reload_tunnels()?;
        Ok(())
    }

    fn switch_to_keys_tab(&mut self) -> Result<()> {
        self.active_tab = 2;
        self.reload_identities()?;
        self.agent_info_updated = std::time::Instant::now() - std::time::Duration::from_secs(60);
        self.refresh_agent_info();
        Ok(())
    }

    fn remove_selected_from_agent(&mut self) -> Result<()> {
        let Some(identity) = self.identities.get(self.identity_selected) else {
            return Ok(());
        };
        let Some(ref key_path) = identity.private_key else {
            self.identity_notice = Some("No private key path set".into());
            return Ok(());
        };
        let name = identity.name.clone();
        match crate::ssh::agent::remove_key(&key_path.to_string_lossy()) {
            Ok(()) => {
                self.identity_notice = Some(format!("Removed {} from agent", name));
                let _ = self.store.log_auth_event(
                    &name,
                    None,
                    "agent",
                    "ok",
                    &format!("key removed from agent: {}", key_path.to_string_lossy()),
                );
                self.agent_info = None;
                self.agent_info_updated =
                    std::time::Instant::now() - std::time::Duration::from_secs(60);
                self.refresh_agent_info();
            }
            Err(e) => {
                self.identity_notice = Some(format!("Failed: {e:#}"));
                let _ = self.store.log_auth_event(
                    &name,
                    None,
                    "agent",
                    "fail",
                    &format!("remove from agent failed: {e:#}"),
                );
            }
        }
        Ok(())
    }

    fn add_selected_to_agent(&mut self) -> Result<()> {
        let Some(identity) = self.identities.get(self.identity_selected) else {
            return Ok(());
        };
        let Some(ref key_path) = identity.private_key else {
            self.identity_notice = Some("No private key path set".into());
            return Ok(());
        };
        let name = identity.name.clone();
        match crate::ssh::agent::add_key(&key_path.to_string_lossy()) {
            Ok(()) => {
                self.identity_notice = Some(format!("Added {} to agent", name));
                let _ = self.store.log_auth_event(
                    &name,
                    None,
                    "agent",
                    "launched",
                    &format!("key added to agent: {}", key_path.to_string_lossy()),
                );
                self.agent_info = None;
                self.agent_info_updated =
                    std::time::Instant::now() - std::time::Duration::from_secs(60);
                self.refresh_agent_info();
            }
            Err(e) => {
                self.identity_notice = Some(format!("Failed: {e:#}"));
                let _ = self.store.log_auth_event(
                    &name,
                    None,
                    "agent",
                    "fail",
                    &format!("add to agent failed: {e:#}"),
                );
            }
        }
        Ok(())
    }

    fn refresh_audit_events(&mut self) {
        let status = self.audit_filter.sql_status();
        let since = self.audit_range.since_timestamp();
        if let Ok(events) = self.store.list_auth_events_filtered(status, since, 500) {
            self.auth_events_cache = events;
        }
    }

    fn handle_key_tunnels(&mut self, key: KeyEvent) -> Result<()> {
        self.tunnel_notice = None;

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.tunnels.is_empty() {
                    self.tunnel_selected = (self.tunnel_selected + 1).min(self.tunnels.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.tunnel_selected = self.tunnel_selected.saturating_sub(1);
            }
            KeyCode::Char('a') if key.modifiers.is_empty() => {
                self.tunnel_form = Some(TunnelFormEdit {
                    editing_id: None,
                    tunnel_type: crate::store::TunnelType::Local,
                    local_port: String::new(),
                    remote_host: "localhost".into(),
                    remote_port: String::new(),
                    host_id: None,
                    label: String::new(),
                    active_field: TunnelFormField::Host,
                    editing: true,
                    edit_snapshot: String::new(),
                    dirty: false,
                });
                self.mode = AppMode::TunnelForm;
            }
            KeyCode::Char('e') if key.modifiers.is_empty() => {
                if let Some(tunnel) = self.tunnels.get(self.tunnel_selected) {
                    self.tunnel_form = Some(TunnelFormEdit {
                        editing_id: Some(tunnel.id),
                        tunnel_type: tunnel.tunnel_type,
                        local_port: tunnel.local_port.to_string(),
                        remote_host: tunnel.remote_host.clone(),
                        remote_port: tunnel.remote_port.to_string(),
                        host_id: tunnel.host_id,
                        label: tunnel.label.clone().unwrap_or_default(),
                        active_field: TunnelFormField::Host,
                        editing: true,
                        edit_snapshot: String::new(),
                        dirty: false,
                    });
                    self.mode = AppMode::TunnelForm;
                }
            }
            KeyCode::Char('d') if key.modifiers.is_empty() => {
                if let Some(tunnel) = self.tunnels.get(self.tunnel_selected) {
                    let label = tunnel
                        .label
                        .clone()
                        .unwrap_or_else(|| format!(":{}", tunnel.local_port));
                    self.pending_delete = Some(PendingDelete::Tunnel {
                        id: tunnel.id,
                        label,
                    });
                    self.mode = AppMode::ConfirmDelete;
                }
            }
            KeyCode::Enter => self.toggle_tunnel()?,
            KeyCode::Char('x') if key.modifiers.is_empty() => self.kill_selected_tunnel()?,
            KeyCode::Char('1') if key.modifiers.is_empty() => self.active_tab = 0,
            KeyCode::Char('2') if key.modifiers.is_empty() => self.switch_to_tunnels_tab()?,
            KeyCode::Char('3') if key.modifiers.is_empty() => self.switch_to_keys_tab()?,
            KeyCode::Char('4') if key.modifiers.is_empty() => {
                self.active_tab = 3;
                self.refresh_audit_events();
            }
            KeyCode::Char('h') if key.modifiers.is_empty() => self.active_tab = 0,
            _ if self.is_action(KeyAction::Help, &key) => {
                self.pre_help_mode = Some(self.mode);
                self.mode = AppMode::Help;
            }
            _ => {}
        }
        Ok(())
    }

    fn toggle_tunnel(&mut self) -> Result<()> {
        let Some(tunnel) = self.tunnels.get(self.tunnel_selected).cloned() else {
            return Ok(());
        };
        let host = tunnel
            .host_id
            .and_then(|hid| self.store.get_host(hid).ok().flatten());
        let host_name = host.as_ref().map(|h| h.name.as_str()).unwrap_or("unknown");
        let label = tunnel.label.as_deref().unwrap_or("");

        if self.tunnel_manager.is_running(tunnel.id) {
            self.tunnel_manager.stop(tunnel.id)?;
            self.tunnel_notice = Some(format!("Stopped tunnel :{}", tunnel.local_port));
            let _ = self.store.log_auth_event(
                host_name,
                None,
                "tunnel",
                "ok",
                &format!("tunnel stopped :{} {}", tunnel.local_port, label),
            );
        } else {
            match self.tunnel_manager.start(&tunnel, host.as_ref()) {
                Ok(()) => {
                    self.tunnel_notice = Some(format!("Started tunnel :{}", tunnel.local_port));
                    let _ = self.store.log_auth_event(
                        host_name,
                        None,
                        "tunnel",
                        "launched",
                        &format!("tunnel started :{} {}", tunnel.local_port, label),
                    );
                }
                Err(e) => {
                    self.tunnel_notice = Some(format!("Failed: {e:#}"));
                    let _ = self.store.log_auth_event(
                        host_name,
                        None,
                        "tunnel",
                        "fail",
                        &format!("tunnel failed :{} — {e:#}", tunnel.local_port),
                    );
                }
            }
        }
        Ok(())
    }

    fn kill_selected_tunnel(&mut self) -> Result<()> {
        let Some(tunnel) = self.tunnels.get(self.tunnel_selected) else {
            return Ok(());
        };
        if self.tunnel_manager.is_running(tunnel.id) {
            let host_name = tunnel
                .host_id
                .and_then(|hid| self.store.get_host(hid).ok().flatten())
                .map(|h| h.name)
                .unwrap_or_else(|| "unknown".into());
            self.tunnel_manager.stop(tunnel.id)?;
            self.tunnel_notice = Some(format!("Killed tunnel :{}", tunnel.local_port));
            let _ = self.store.log_auth_event(
                &host_name,
                None,
                "tunnel",
                "ok",
                &format!("tunnel killed :{}", tunnel.local_port),
            );
        }
        Ok(())
    }

    pub fn reload_tunnels(&mut self) -> Result<()> {
        self.tunnels = self.store.list_tunnels()?;
        if self.tunnel_selected >= self.tunnels.len() && !self.tunnels.is_empty() {
            self.tunnel_selected = self.tunnels.len() - 1;
        }
        Ok(())
    }

    fn handle_key_tunnel_form(&mut self, key: KeyEvent) -> Result<()> {
        let Some(form) = self.tunnel_form.as_ref() else {
            return Ok(());
        };
        let field = form.active_field;
        match key.code {
            KeyCode::Esc => {
                if self.tunnel_form.as_ref().is_some_and(|f| f.dirty) {
                    self.mode = AppMode::ConfirmDiscard;
                } else {
                    self.tunnel_form = None;
                    self.mode = AppMode::Normal;
                }
            }
            _ if self.is_save_key(&key) => self.save_tunnel_form()?,
            // Single-step model: Enter on the last field saves.
            KeyCode::Enter if field == TunnelFormField::Label => self.save_tunnel_form()?,
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down if key.modifiers.is_empty() => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    form.active_field = form.active_field.next();
                }
            }
            KeyCode::BackTab | KeyCode::Up => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    form.active_field = form.active_field.prev();
                }
            }
            KeyCode::Left | KeyCode::Right => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    if form.active_field == TunnelFormField::Type {
                        form.tunnel_type = form.tunnel_type.next();
                        form.dirty = true;
                    } else if form.active_field == TunnelFormField::Host {
                        let host_ids: Vec<i64> =
                            self.hosts.iter().filter_map(|h| h.managed_id()).collect();
                        if !host_ids.is_empty() {
                            let current_idx = form
                                .host_id
                                .and_then(|id| host_ids.iter().position(|&h| h == id));
                            let next = match key.code {
                                KeyCode::Right => {
                                    current_idx.map(|i| (i + 1) % host_ids.len()).unwrap_or(0)
                                }
                                _ => current_idx
                                    .map(|i| (i + host_ids.len() - 1) % host_ids.len())
                                    .unwrap_or(host_ids.len() - 1),
                            };
                            form.host_id = Some(host_ids[next]);
                            form.dirty = true;
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(form) = self.tunnel_form.as_mut() {
                    let field = match form.active_field {
                        TunnelFormField::LocalPort => &mut form.local_port,
                        TunnelFormField::RemoteHost => &mut form.remote_host,
                        TunnelFormField::RemotePort => &mut form.remote_port,
                        TunnelFormField::Label => &mut form.label,
                        _ => return Ok(()),
                    };
                    if field.pop().is_some() {
                        form.dirty = true;
                    }
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                if let Some(form) = self.tunnel_form.as_mut() {
                    let field = match form.active_field {
                        TunnelFormField::LocalPort => &mut form.local_port,
                        TunnelFormField::RemoteHost => &mut form.remote_host,
                        TunnelFormField::RemotePort => &mut form.remote_port,
                        TunnelFormField::Label => &mut form.label,
                        _ => return Ok(()),
                    };
                    field.push(c);
                    form.dirty = true;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn save_tunnel_form(&mut self) -> Result<()> {
        let Some(form) = self.tunnel_form.take() else {
            return Ok(());
        };

        let local_port: u16 = form.local_port.parse().unwrap_or(0);
        if local_port == 0 {
            self.tunnel_notice = Some("Invalid local port".into());
            self.tunnel_form = Some(form);
            return Ok(());
        }
        let remote_port: u16 = if form.tunnel_type == crate::store::TunnelType::Dynamic {
            0
        } else {
            form.remote_port.parse().unwrap_or(0)
        };

        let new = crate::store::NewTunnel {
            host_id: form.host_id,
            tunnel_type: form.tunnel_type,
            local_port,
            remote_host: form.remote_host,
            remote_port,
            label: if form.label.is_empty() {
                None
            } else {
                Some(form.label)
            },
            // Preserved below when editing an existing tunnel.
            auto_connect: false,
        };

        match form.editing_id {
            None => {
                self.store.create_tunnel(&new)?;
                self.tunnel_notice = Some(format!("Created tunnel :{local_port}"));
            }
            Some(id) => {
                // Recreate, carrying over fields the form doesn't expose.
                let mut new = new;
                if let Some(existing) = self.tunnels.iter().find(|t| t.id == id) {
                    new.auto_connect = existing.auto_connect;
                }
                self.store.delete_tunnel(id)?;
                self.store.create_tunnel(&new)?;
                self.tunnel_notice = Some(format!("Updated tunnel :{local_port}"));
            }
        }

        self.reload_tunnels()?;
        self.mode = AppMode::Normal;
        Ok(())
    }

    fn handle_key_identity_form(&mut self, key: KeyEvent) -> Result<()> {
        let Some(form) = self.identity_form.as_ref() else {
            return Ok(());
        };
        let field = form.field;
        match key.code {
            KeyCode::Esc => self.cancel_identity_form()?,
            _ if self.is_save_key(&key) => self.save_identity_form()?,
            // Single-step model: Enter on the last field saves.
            KeyCode::Enter if field == IdentityFormField::Password => {
                self.save_identity_form()?;
            }
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down if key.modifiers.is_empty() => {
                self.identity_form_field_next();
            }
            KeyCode::BackTab | KeyCode::Up => self.identity_form_field_prev(),
            KeyCode::Backspace => self.identity_form_backspace(),
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.identity_form_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn reload_identities(&mut self) -> Result<()> {
        let selected_name = self
            .identities
            .get(self.identity_selected)
            .map(|i| i.name.clone());
        self.identities = self.store.list_identities()?;
        if let Some(name) = selected_name {
            if let Some(pos) = self.identities.iter().position(|i| i.name == name) {
                self.identity_selected = pos;
            } else {
                self.clamp_identity_selected();
            }
        } else {
            self.clamp_identity_selected();
        }
        Ok(())
    }

    fn selected_host_group_id(&self) -> Option<i64> {
        self.selected_entry()
            .and_then(|e| e.managed())
            .and_then(|m| m.group_id)
    }

    fn enter_group_manage(&mut self) -> Result<()> {
        self.groups = self.store.list_groups()?;
        self.group_notice = None;
        self.clamp_group_manage_selected();
        self.mode = AppMode::GroupManage;
        Ok(())
    }

    fn clamp_group_manage_selected(&mut self) {
        if !self.groups.is_empty() {
            self.group_manage_selected = self.group_manage_selected.min(self.groups.len() - 1);
        } else {
            self.group_manage_selected = 0;
        }
    }

    fn move_group_manage_selection(&mut self, delta: isize) {
        if self.groups.is_empty() {
            return;
        }
        let new = self.group_manage_selected as isize + delta;
        self.group_manage_selected = new.clamp(0, self.groups.len() as isize - 1) as usize;
    }

    fn handle_key_group_manage(&mut self, key: KeyEvent) -> Result<()> {
        self.group_notice = None;

        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            KeyCode::Esc | KeyCode::Char('h') if key.modifiers.is_empty() => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Char('1') if key.modifiers.is_empty() => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Char('j') | KeyCode::Down if key.modifiers.is_empty() => {
                self.move_group_manage_selection(1)
            }
            KeyCode::Char('k') | KeyCode::Up if key.modifiers.is_empty() => {
                self.move_group_manage_selection(-1)
            }
            KeyCode::Char('a') if key.modifiers.is_empty() => {
                self.enter_group_form(None)?;
            }
            KeyCode::Char('e') if key.modifiers.is_empty() => {
                if let Some(group) = self.groups.get(self.group_manage_selected).cloned() {
                    self.enter_group_form(Some(&group))?;
                }
            }
            KeyCode::Char('d') if key.modifiers.is_empty() => {
                if let Some(group) = self.groups.get(self.group_manage_selected).cloned() {
                    self.pending_delete = Some(PendingDelete::Group {
                        id: group.id,
                        name: group.name.clone(),
                    });
                    self.mode = AppMode::ConfirmDelete;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_key_confirm_discard(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('y') if key.modifiers.is_empty() => {
                // Save; on validation failure the form survives — return to it
                // so the user sees the notice instead of a stuck dialog.
                if self.host_form.is_some() {
                    self.save_host_form()?;
                    if self.host_form.is_some() && self.mode == AppMode::ConfirmDiscard {
                        self.mode = AppMode::HostForm;
                    }
                } else if self.identity_form.is_some() {
                    self.save_identity_form()?;
                    if self.identity_form.is_some() && self.mode == AppMode::ConfirmDiscard {
                        self.mode = AppMode::IdentityForm;
                    }
                } else if self.tunnel_form.is_some() {
                    self.save_tunnel_form()?;
                    if self.tunnel_form.is_some() && self.mode == AppMode::ConfirmDiscard {
                        self.mode = AppMode::TunnelForm;
                    }
                }
            }
            KeyCode::Char('n') if key.modifiers.is_empty() => {
                // Discard
                if self.host_form.is_some() {
                    self.discard_host_form()?;
                } else if self.identity_form.is_some() {
                    self.discard_identity_form()?;
                } else if self.tunnel_form.is_some() {
                    self.tunnel_form = None;
                    self.mode = AppMode::Normal;
                }
            }
            KeyCode::Esc => {
                // Go back to form
                if self.host_form.is_some() {
                    self.mode = AppMode::HostForm;
                } else if self.identity_form.is_some() {
                    self.mode = AppMode::IdentityForm;
                } else if self.tunnel_form.is_some() {
                    self.mode = AppMode::TunnelForm;
                } else {
                    self.mode = AppMode::Normal;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn enter_group_form(&mut self, existing: Option<&HostGroup>) -> Result<()> {
        let return_to_manage = self.mode == AppMode::GroupManage;
        let form = if let Some(group) = existing {
            GroupFormEdit {
                id: Some(group.id),
                name: group.name.clone(),
                cursor: text_input::char_len(&group.name),
                default_identity_id: group.default_identity_id,
                return_to_manage,
            }
        } else {
            GroupFormEdit {
                id: None,
                name: String::new(),
                cursor: 0,
                default_identity_id: None,
                return_to_manage,
            }
        };
        self.group_form = Some(form);
        self.mode = AppMode::GroupForm;
        Ok(())
    }

    fn rename_selected_host_group(&mut self) -> Result<()> {
        let Some(group_id) = self.selected_host_group_id() else {
            self.host_notice = Some("Select a host in a group to rename it".into());
            return Ok(());
        };
        let Some(group) = self.groups.iter().find(|g| g.id == group_id).cloned() else {
            self.reload_hosts()?;
            return Ok(());
        };
        self.enter_group_form(Some(&group))
    }

    fn delete_selected_host_group(&mut self) -> Result<()> {
        let Some(group_id) = self.selected_host_group_id() else {
            self.host_notice = Some("Select a host in a group to delete it".into());
            return Ok(());
        };
        let name = self
            .groups
            .iter()
            .find(|g| g.id == group_id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| "group".into());
        self.pending_delete = Some(PendingDelete::Group { id: group_id, name });
        self.mode = AppMode::ConfirmDelete;
        Ok(())
    }

    fn handle_key_help(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Enter => {
                self.mode = self.pre_help_mode.take().unwrap_or(AppMode::Normal);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle a keystroke while an embedded session is active.
    ///
    /// - `Ctrl+D` closes the active tab (returns to dashboard when the last
    ///   tab closes).
    /// - `Ctrl+T` opens a new tab to the same host (duplicates the current
    ///   session config).
    /// - `Ctrl+W` closes the active tab (alias for Ctrl+D).
    /// - `Ctrl+PgUp` / `Ctrl+PgDn` switch tabs.
    /// - `Esc` during Connecting cancels and returns; after running, forwards.
    /// - `PgUp` / `PgDn` navigate scrollback locally (don't reach the shell).
    /// - Everything else snaps scrollback to live and forwards encoded bytes.
    fn handle_key_session(&mut self, key: KeyEvent) -> Result<()> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Tab management intercepts. These never reach the remote.
        if ctrl {
            match key.code {
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    self.duplicate_active_session();
                    return Ok(());
                }
                KeyCode::Char('w') | KeyCode::Char('W') => {
                    self.close_active_session();
                    return Ok(());
                }
                KeyCode::PageUp => {
                    self.switch_session(-1);
                    return Ok(());
                }
                KeyCode::PageDown => {
                    self.switch_session(1);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Capture self.terminal_area.height before we take a mutable borrow
        // on `session` — borrowck won't let us re-read self after that.
        let body_rows = self.terminal_area.height.saturating_sub(2).max(1) as usize;

        let Some(session) = self.active_session_mut() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        if session.phase.is_terminal() {
            self.close_active_session();
            return Ok(());
        }

        if ctrl && matches!(key.code, KeyCode::Char('d') | KeyCode::Char('D')) {
            self.close_active_session();
            return Ok(());
        }

        if key.code == KeyCode::Esc
            && matches!(
                session.phase,
                crate::session::SessionPhase::Connecting { .. }
            )
        {
            self.close_active_session();
            return Ok(());
        }

        // Local scrollback navigation. Half a screen per press.
        let half = (body_rows / 2).max(1);
        match key.code {
            KeyCode::PageUp => {
                session.parser.scroll_up(half);
                return Ok(());
            }
            KeyCode::PageDown => {
                session.parser.scroll_down(half);
                return Ok(());
            }
            _ => {}
        }

        // Any other key snaps the view back to live and forwards.
        if session.parser.scrollback() > 0 {
            session.parser.snap_to_bottom();
        }
        if let Some(bytes) = crate::session::keys::encode(key) {
            let _ = session.write(&bytes);
        }
        Ok(())
    }

    /// Shared accessor for the visible session, if any.
    pub fn active_session(&self) -> Option<&crate::session::Session> {
        self.active_session.and_then(|i| self.sessions.get(i))
    }

    pub fn active_session_mut(&mut self) -> Option<&mut crate::session::Session> {
        let idx = self.active_session?;
        self.sessions.get_mut(idx)
    }

    /// Tear down the active embedded session and return to the dashboard when
    /// it was the last one — otherwise switch to the next remaining tab.
    pub fn close_active_session(&mut self) {
        let Some(idx) = self.active_session else {
            self.mode = AppMode::Normal;
            return;
        };
        if idx < self.sessions.len() {
            // If we were armed with a secret but never fired, surface what
            // we actually saw on the screen so the user can tell us whether
            // the prompt text didn't match or no prompt arrived at all.
            let session = &mut self.sessions[idx];
            if session.was_armed() && !session.secret_was_sent() {
                let snippet = session.screen_tail_snippet();
                let preview: String = snippet
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("(blank)")
                    .chars()
                    .take(120)
                    .collect();
                let host_name = session.display_name.clone();
                self.push_ssh_log(crate::ssh::probe::SshLogEntry {
                    host_name,
                    line: format!(
                        "auth: armed but no prompt matched. last visible line: {preview:?}"
                    ),
                    level: crate::ssh::probe::LogLevel::Info,
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64,
                });
            }

            // Session::drop kills the child + joins the reader thread.
            self.sessions.remove(idx);
        }
        if self.sessions.is_empty() {
            self.active_session = None;
            self.mode = AppMode::Normal;
        } else {
            // Stay at the same index if possible, else drop back to the new last.
            self.active_session = Some(idx.min(self.sessions.len() - 1));
            self.mode = AppMode::Session;
        }
    }

    /// Spawn a fresh session reusing the active tab's config (same host).
    pub fn duplicate_active_session(&mut self) {
        let Some(idx) = self.active_session else {
            return;
        };
        let Some(active) = self.sessions.get(idx) else {
            return;
        };
        let cfg = active.config.clone();
        let rows = self.terminal_area.height.max(3);
        let cols = self.terminal_area.width.max(20);
        match crate::session::Session::spawn(cfg, rows, cols) {
            Ok(session) => {
                self.sessions.push(session);
                self.active_session = Some(self.sessions.len() - 1);
                self.mode = AppMode::Connecting;
            }
            Err(e) => {
                self.host_notice = Some(format!("New tab failed: {e:#}"));
            }
        }
    }

    /// Cycle tabs by `delta` (`+1` = next, `-1` = prev). Wraps at both ends.
    pub fn switch_session(&mut self, delta: isize) {
        if self.sessions.is_empty() {
            self.active_session = None;
            self.mode = AppMode::Normal;
            return;
        }
        let len = self.sessions.len() as isize;
        let cur = self.active_session.unwrap_or(0) as isize;
        let next = ((cur + delta) % len + len) % len;
        self.active_session = Some(next as usize);

        // Reflect the new active session's phase in app.mode, so render
        // dispatch picks the right path.
        let phase = &self.sessions[next as usize].phase;
        self.mode = match phase {
            crate::session::SessionPhase::Connecting { .. } => AppMode::Connecting,
            _ => AppMode::Session,
        };
    }

    /// Legacy alias retained for tests / callers that explicitly want to end
    /// the whole session stack.
    pub fn end_session(&mut self) {
        self.sessions.clear();
        self.active_session = None;
        self.mode = AppMode::Normal;
    }

    /// Copy the SSH log entries for the selected host to the system clipboard
    /// via OSC 52. Works in kitty / iTerm / wezterm / Alacritty out of the box
    /// without needing an external `xclip`/`pbcopy` dependency.
    pub fn yank_ssh_log(&mut self) -> Result<()> {
        let Some(entry) = self.selected_entry() else {
            return Ok(());
        };
        let host_name = entry.name().to_string();
        let lines: Vec<String> = self
            .ssh_log
            .iter()
            .filter(|e| e.host_name == host_name)
            .map(|e| format!("{} {}", crate::tui::format_local_time(e.timestamp), e.line))
            .collect();

        if lines.is_empty() {
            self.host_notice = Some(format!("no log entries to copy for {host_name}"));
            return Ok(());
        }

        let text = lines.join("\n");
        let n = lines.len();
        match write_osc52(&text) {
            Ok(()) => {
                self.host_notice = Some(format!(
                    "copied {n} log line{} for {host_name} to clipboard",
                    if n == 1 { "" } else { "s" }
                ));
            }
            Err(e) => {
                self.host_notice = Some(format!("clipboard copy failed: {e:#}"));
            }
        }
        Ok(())
    }

    /// Mouse events while in a session. When the remote app has enabled mouse
    /// reporting we forward; otherwise the scroll wheel drives local
    /// scrollback navigation and clicks are dropped.
    fn handle_mouse_session(&mut self, mouse: MouseEvent) {
        let Some(session) = self.active_session_mut() else {
            return;
        };

        let mode = session.parser.screen().mouse_protocol_mode();
        let encoding = session.parser.screen().mouse_protocol_encoding();

        if mode != vt100::MouseProtocolMode::None {
            // Remote app is consuming mouse — translate to the wire protocol.
            // Body starts on row 1 (header takes row 0). Translate the global
            // column / row to body-local coordinates.
            let local_y = mouse.row.saturating_sub(1);
            if let Some(bytes) =
                crate::session::keys::encode_mouse(mouse, mouse.column, local_y, mode, encoding)
            {
                let _ = session.write(&bytes);
            }
            return;
        }

        // No remote mouse handling — local scroll wheel drives scrollback.
        match mouse.kind {
            MouseEventKind::ScrollUp => session.parser.scroll_up(3),
            MouseEventKind::ScrollDown => session.parser.scroll_down(3),
            _ => {}
        }
    }

    fn handle_key_confirm_delete(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('y') => match self.pending_delete.take() {
                Some(PendingDelete::Host { id, name }) => {
                    match self.store.delete_host(id)? {
                        DeleteHostOutcome::Deleted => {
                            self.host_notice = Some(format!("Host '{name}' deleted"));
                            self.reload_hosts()?;
                        }
                        DeleteHostOutcome::NotLauncher => {
                            self.host_notice = Some("Only launcher hosts can be deleted".into());
                        }
                        DeleteHostOutcome::NotFound => self.reload_hosts()?,
                    }
                    self.mode = AppMode::Normal;
                }
                Some(PendingDelete::Identity { id, name }) => {
                    match self.store.delete_identity(id)? {
                        crate::store::DeleteIdentityOutcome::Deleted => {
                            self.identity_notice = Some(format!("Identity '{name}' deleted"));
                            self.reload_identities()?;
                        }
                        crate::store::DeleteIdentityOutcome::InUse { host_count } => {
                            self.identity_notice = Some(format!(
                                "Cannot delete '{name}': used by {host_count} host(s)"
                            ));
                        }
                        crate::store::DeleteIdentityOutcome::NotFound => {
                            self.reload_identities()?;
                        }
                    }
                    self.mode = AppMode::Normal;
                }
                Some(PendingDelete::Group { id, name }) => {
                    if self.store.delete_group(id)? {
                        self.group_notice = Some(format!("Group '{name}' deleted"));
                        self.reload_hosts()?;
                    }
                    self.enter_group_manage()?;
                }
                Some(PendingDelete::Tunnel { id, label }) => {
                    if self.tunnel_manager.is_running(id) {
                        self.tunnel_manager.stop(id)?;
                    }
                    self.store.delete_tunnel(id)?;
                    self.tunnel_notice = Some(format!("Tunnel '{label}' deleted"));
                    self.reload_tunnels()?;
                    self.mode = AppMode::Normal;
                }
                None => {
                    self.mode = AppMode::Normal;
                }
            },
            KeyCode::Char('n') | KeyCode::Esc => {
                let was_group = matches!(self.pending_delete, Some(PendingDelete::Group { .. }));
                self.pending_delete = None;
                if was_group {
                    self.enter_group_manage()?;
                } else {
                    self.mode = AppMode::Normal;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn cancel_group_form(&mut self) -> Result<()> {
        let return_to_manage = self.group_form.as_ref().is_some_and(|f| f.return_to_manage);
        self.group_form = None;
        if return_to_manage {
            self.enter_group_manage()?;
        } else {
            self.mode = AppMode::Normal;
        }
        Ok(())
    }

    fn save_group_form(&mut self) -> Result<()> {
        let Some(form) = self.group_form.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        let name = form.name.trim();
        if name.is_empty() {
            self.host_notice = Some("Group name is required".into());
            self.group_form = Some(form);
            return Ok(());
        }

        if let Some(id) = form.id {
            self.store.update_group(
                id,
                &HostGroupUpdate {
                    name: Some(name.to_string()),
                    sort_order: None,
                    default_identity_id: Some(form.default_identity_id),
                },
            )?;
        } else {
            let sort_order = self.groups.len() as i32;
            self.store.create_group(&NewHostGroup {
                name: name.to_string(),
                sort_order,
                default_identity_id: form.default_identity_id,
            })?;
        }

        let return_to_manage = form.return_to_manage;
        self.reload_hosts()?;
        if return_to_manage {
            self.enter_group_manage()?;
        } else {
            self.mode = AppMode::Normal;
        }
        Ok(())
    }

    fn handle_key_group_form(&mut self, key: KeyEvent) -> Result<()> {
        if self.group_form.is_none() {
            return Ok(());
        }

        match key.code {
            KeyCode::Esc => self.cancel_group_form()?,
            KeyCode::Enter => self.save_group_form()?,
            _ if self.is_save_key(&key) => self.save_group_form()?,
            KeyCode::Left => self.group_form_cycle_identity(-1),
            KeyCode::Right => self.group_form_cycle_identity(1),
            KeyCode::Backspace if key.modifiers.is_empty() => self.group_form_backspace(),
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.group_form_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    /// Cycle the group's default identity through `[none, id0, id1, …]`.
    fn group_form_cycle_identity(&mut self, delta: i32) {
        // Build the option ring: index 0 is "none", then each identity.
        let ids: Vec<i64> = self.identities.iter().map(|i| i.id).collect();
        let len = ids.len() as i32 + 1;
        let Some(form) = self.group_form.as_mut() else {
            return;
        };
        let cur = match form.default_identity_id {
            None => 0,
            Some(id) => ids.iter().position(|&x| x == id).map_or(0, |p| p as i32 + 1),
        };
        let next = (cur + delta).rem_euclid(len);
        form.default_identity_id = if next == 0 {
            None
        } else {
            Some(ids[(next - 1) as usize])
        };
    }

    fn group_form_insert(&mut self, ch: char) {
        let Some(form) = self.group_form.as_mut() else {
            return;
        };
        form.cursor = text_input::insert_at(&mut form.name, form.cursor, ch);
    }

    fn group_form_backspace(&mut self) {
        let Some(form) = self.group_form.as_mut() else {
            return;
        };
        form.cursor = text_input::backspace_at(&mut form.name, form.cursor);
    }

    fn enter_identity_form(&mut self, existing: Option<&Identity>) -> Result<()> {
        let form = if let Some(identity) = existing {
            IdentityFormEdit {
                id: Some(identity.id),
                name: identity.name.clone(),
                username: identity.username.clone().unwrap_or_default(),
                private_key: identity
                    .private_key
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                certificate: identity
                    .certificate
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                password: String::new(),
                has_password: identity.has_password,
                pasted_key: None,
                field: IdentityFormField::Name,
                cursor: text_input::char_len(&identity.name),
                editing: true,
                edit_snapshot: String::new(),
                dirty: false,
            }
        } else {
            IdentityFormEdit {
                id: None,
                name: String::new(),
                username: String::new(),
                private_key: String::new(),
                certificate: String::new(),
                password: String::new(),
                has_password: false,
                pasted_key: None,
                field: IdentityFormField::Name,
                cursor: 0,
                editing: true,
                edit_snapshot: String::new(),
                dirty: false,
            }
        };
        self.identity_form = Some(form);
        self.mode = AppMode::IdentityForm;
        Ok(())
    }

    fn edit_selected_identity(&mut self) -> Result<()> {
        let Some(identity) = self.selected_identity().cloned() else {
            return Ok(());
        };
        self.enter_identity_form(Some(&identity))
    }

    fn delete_selected_identity(&mut self) -> Result<()> {
        let Some(identity) = self.selected_identity().cloned() else {
            return Ok(());
        };
        self.pending_delete = Some(PendingDelete::Identity {
            id: identity.id,
            name: identity.name.clone(),
        });
        self.mode = AppMode::ConfirmDelete;
        Ok(())
    }

    fn cancel_identity_form(&mut self) -> Result<()> {
        if self.identity_form.as_ref().is_some_and(|f| f.dirty) {
            self.mode = AppMode::ConfirmDiscard;
        } else {
            self.discard_identity_form()?;
        }
        Ok(())
    }

    fn discard_identity_form(&mut self) -> Result<()> {
        self.identity_form = None;
        self.mode = AppMode::Normal;
        Ok(())
    }

    fn save_identity_form(&mut self) -> Result<()> {
        let Some(form) = self.identity_form.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        let name = form.name.trim();
        if name.is_empty() {
            self.identity_notice = Some("Identity name is required".into());
            self.identity_form = Some(form);
            return Ok(());
        }

        let username = optional_field(&form.username);
        let private_key = if let Some(blob) = form.pasted_key.as_deref() {
            match crate::ssh::write_key_material(name, blob) {
                Ok(path) => Some(path),
                Err(e) => {
                    self.identity_notice = Some(format!("Could not write key file: {e}"));
                    self.identity_form = Some(form);
                    return Ok(());
                }
            }
        } else {
            optional_path(&form.private_key)
        };
        let certificate = optional_path(&form.certificate);

        // If the key is passphrase-protected, require (and verify) the
        // passphrase before saving — otherwise auto-auth would silently fail
        // later. Skip when a passphrase is already stored (has_password).
        if let Some(ref key_path) = private_key {
            let expanded = crate::ssh::expand_tilde(&key_path.to_string_lossy());
            if form.password.is_empty() && !form.has_password {
                if crate::ssh::key_is_encrypted(&expanded) == Some(true) {
                    self.identity_notice =
                        Some("This key is passphrase-protected — enter its passphrase".into());
                    let mut form = form;
                    form.field = IdentityFormField::Password;
                    form.cursor = 0;
                    self.identity_form = Some(form);
                    return Ok(());
                }
            } else if !form.password.is_empty()
                && crate::ssh::passphrase_matches(&expanded, &form.password) == Some(false)
            {
                self.identity_notice = Some("Passphrase does not match this key".into());
                self.identity_form = Some(form);
                return Ok(());
            }
        }

        let password_changed = !form.password.is_empty();
        let new_has_password = if password_changed {
            true
        } else {
            form.has_password
        };

        if let Some(id) = form.id {
            if password_changed {
                if let Err(e) = self
                    .password_store
                    .set(&crate::credentials::identity_key(id), &form.password)
                {
                    self.identity_notice =
                        Some(format!("Saved, but storing the passphrase failed: {e}"));
                }
            }
            self.store.update_identity(
                id,
                &IdentityUpdate {
                    name: Some(name.to_string()),
                    username: Some(username),
                    private_key: Some(private_key),
                    certificate: Some(certificate),
                    has_password: Some(new_has_password),
                    ..Default::default()
                },
            )?;
        } else {
            let sort_order = self.identities.len() as i32;
            let created = self.store.create_identity(&NewIdentity {
                name: name.to_string(),
                username,
                private_key,
                certificate,
                sort_order,
                has_password: new_has_password,
            })?;
            if password_changed {
                if let Err(e) = self.password_store.set(
                    &crate::credentials::identity_key(created.id),
                    &form.password,
                ) {
                    self.identity_notice =
                        Some(format!("Saved, but storing the passphrase failed: {e}"));
                }
            }
        }

        self.mode = AppMode::Normal;
        self.reload_identities()?;
        if let Some(pos) = self.identities.iter().position(|i| i.name == name) {
            self.identity_selected = pos;
        }
        Ok(())
    }

    fn identity_form_field_next(&mut self) {
        let Some(form) = self.identity_form.as_mut() else {
            return;
        };
        form.field = form.field.next();
        form.cursor = text_input::char_len(form.active_field());
    }

    fn identity_form_field_prev(&mut self) {
        let Some(form) = self.identity_form.as_mut() else {
            return;
        };
        form.field = form.field.prev();
        form.cursor = text_input::char_len(form.active_field());
    }

    fn identity_form_backspace(&mut self) {
        let Some(form) = self.identity_form.as_mut() else {
            return;
        };
        if form.field == IdentityFormField::PrivateKey && form.pasted_key.is_some() {
            // One backspace discards the pasted blob entirely.
            form.pasted_key = None;
            form.private_key.clear();
            form.cursor = 0;
            return;
        }
        let c = form.cursor;
        if c > 0 {
            form.cursor = text_input::backspace_at(form.active_field_mut(), c);
            form.dirty = true;
        }
    }

    fn identity_form_insert(&mut self, ch: char) {
        let Some(form) = self.identity_form.as_mut() else {
            return;
        };
        form.clear_pasted_key_marker();
        let c = form.cursor;
        form.cursor = text_input::insert_at(form.active_field_mut(), c, ch);
        form.dirty = true;
    }

    /// Columns in the identities grid — the exact value the renderer uses.
    fn identity_cards_per_row(&self) -> i32 {
        let inner_w = crate::tui::screens::keys::inner_width(self.terminal_area.width);
        crate::tui::screens::keys::resolve_columns(inner_w, self.config.appearance.identity_columns)
            as i32
    }

    /// Change how many columns the identities grid shows (`delta` +1/-1),
    /// clamped to what fits, and persist it.
    fn adjust_identity_columns(&mut self, delta: i32) {
        let inner_w = crate::tui::screens::keys::inner_width(self.terminal_area.width);
        let max = crate::tui::screens::keys::max_columns(inner_w) as i32;
        // Start from the currently-shown count so +/- feels direct even when
        // the stored preference is 0 (auto).
        let current = self.identity_cards_per_row();
        let next = (current + delta).clamp(1, max);
        self.config.appearance.identity_columns = next as usize;
        self.save_config_quietly();
        self.identity_notice = Some(format!("Identity columns: {next}"));
    }

    /// Grid move: `dr` rows down/up, `dc` columns right/left. Left/right never
    /// wrap across rows so navigation stays predictable.
    fn move_identity_grid(&mut self, dr: i32, dc: i32) {
        if self.identities.is_empty() {
            self.identity_selected = 0;
            return;
        }
        let cpr = self.identity_cards_per_row();
        let len = self.identities.len() as i32;
        let cur = self.identity_selected as i32;
        if dc != 0 {
            let col = cur % cpr;
            let target_col = col + dc;
            if target_col < 0 || target_col >= cpr {
                return; // stay put at the row edge
            }
            let next = cur + dc;
            if next >= 0 && next < len {
                self.identity_selected = next as usize;
            }
        } else if dr != 0 {
            let mut next = cur + dr * cpr;
            // Moving down past the end: drop onto the (shorter) last row's card.
            if dr > 0 && next >= len && cur < len - 1 {
                next = len - 1;
            }
            if next >= 0 && next < len {
                self.identity_selected = next as usize;
            }
        }
    }

    fn clamp_identity_selected(&mut self) {
        if self.identities.is_empty() {
            self.identity_selected = 0;
        } else if self.identity_selected >= self.identities.len() {
            self.identity_selected = self.identities.len() - 1;
        }
    }

    pub fn selected_identity(&self) -> Option<&Identity> {
        self.identities.get(self.identity_selected)
    }

    pub fn store(&self) -> &LauncherStore {
        &self.store
    }

    fn edit_selected_host(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };
        let managed = self.hosts[host_idx].managed().cloned();
        match managed.as_ref().map(|m| m.source) {
            Some(HostSource::SshConfig) => {
                self.enter_host_form(managed.as_ref(), true)?;
            }
            Some(HostSource::Launcher) => {
                self.enter_host_form(managed.as_ref(), false)?;
            }
            None => {
                // Legacy ssh_config alias with no launcher row yet: materialize
                // it into launcher.db so it gains a group/identity/metadata
                // overlay, then edit that full form instead of the tags-only
                // HostDetail (which has no Group field).
                let name = self.hosts[host_idx].name().to_string();
                let materialized = crate::ssh::materialize_ssh_config_host(
                    self.resolver.as_ref(),
                    &self.store,
                    self.metadata.as_ref(),
                    &name,
                )?;
                if materialized {
                    self.reload_hosts()?;
                    self.restore_selection_by_name(&name);
                    let managed = self
                        .selected_host_index()
                        .and_then(|idx| self.hosts[idx].managed().cloned());
                    if managed.is_some() {
                        self.enter_host_form(managed.as_ref(), true)?;
                        return Ok(());
                    }
                }
                self.enter_host_detail()?;
            }
        }
        Ok(())
    }

    pub fn enter_host_form(
        &mut self,
        existing: Option<&ManagedHost>,
        metadata_only: bool,
    ) -> Result<()> {
        self.host_notice = None;
        self.groups = self.store.list_groups()?;
        if self.identities.is_empty() {
            self.identities = self.store.list_identities()?;
        }

        let default_identity_index = self
            .identities
            .iter()
            .position(|i| i.name == "Default")
            .unwrap_or(0);

        let form = if let Some(managed) = existing {
            let group_index = managed
                .group_id
                .and_then(|gid| self.groups.iter().position(|g| g.id == gid).map(|i| i + 1))
                .unwrap_or(0);
            let identity_index = managed
                .identity_id
                .and_then(|iid| self.identities.iter().position(|i| i.id == iid))
                .unwrap_or(default_identity_index);

            let start_field = if metadata_only {
                HostFormField::Label
            } else {
                HostFormField::Address
            };
            let start_cursor = if metadata_only {
                text_input::char_len(managed.label.as_deref().unwrap_or(""))
            } else {
                text_input::char_len(&managed.address)
            };

            HostFormEdit {
                id: Some(managed.id),
                address: managed.address.clone(),
                username: managed
                    .username
                    .clone()
                    .or_else(|| managed.identity.as_ref().and_then(|i| i.username.clone()))
                    .unwrap_or_default(),
                label: managed.label.clone().unwrap_or_default(),
                name: managed.name.clone(),
                port: managed.port.to_string(),
                group_index,
                identity_index,
                tags: managed.tags.join(", "),
                proxy_jump: managed.proxy_jump.clone().unwrap_or_default(),
                forward_agent: managed.forward_agent,
                remote_command: managed.remote_command.clone().unwrap_or_default(),
                os_icon_index: os_icon_index_from_option(&managed.os_icon),
                password: String::new(),
                has_password: managed.has_password,
                field: start_field,
                cursor: start_cursor,
                metadata_only,
                editing: true,
                edit_snapshot: String::new(),
                dirty: false,
            }
        } else {
            // Prefill group + identity from the group the user is currently in.
            // A new host added inside a group inherits the group's default identity.
            let selected_group_id = self.selected_host_group_id();
            let group_index = selected_group_id
                .and_then(|gid| self.groups.iter().position(|g| g.id == gid).map(|i| i + 1))
                .unwrap_or(0);
            let identity_index = selected_group_id
                .and_then(|gid| self.groups.iter().find(|g| g.id == gid))
                .and_then(|g| g.default_identity_id)
                .and_then(|iid| self.identities.iter().position(|i| i.id == iid))
                .unwrap_or(default_identity_index);

            HostFormEdit {
                id: None,
                address: String::new(),
                username: String::new(),
                label: String::new(),
                name: String::new(),
                port: "22".into(),
                group_index,
                identity_index,
                tags: String::new(),
                proxy_jump: String::new(),
                forward_agent: false,
                remote_command: String::new(),
                os_icon_index: 0,
                password: String::new(),
                has_password: false,
                field: HostFormField::Address,
                cursor: 0,
                metadata_only: false,
                editing: true,
                edit_snapshot: String::new(),
                dirty: false,
            }
        };

        self.host_form = Some(form);
        self.mode = AppMode::HostForm;
        Ok(())
    }

    fn cancel_host_form(&mut self) -> Result<()> {
        if self.host_form.as_ref().is_some_and(|f| f.dirty) {
            self.mode = AppMode::ConfirmDiscard;
        } else {
            self.discard_host_form()?;
        }
        Ok(())
    }

    fn discard_host_form(&mut self) -> Result<()> {
        self.host_form = None;
        self.mode = AppMode::Normal;
        Ok(())
    }

    fn save_host_form(&mut self) -> Result<()> {
        let Some(form) = self.host_form.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        let group_id = if form.group_index == 0 {
            None
        } else {
            self.groups.get(form.group_index - 1).map(|g| g.id)
        };
        let identity_id = self.identities.get(form.identity_index).map(|i| i.id);
        let tags = parse_tags(&form.tags);
        let label = optional_field(&form.label);
        let host_pw_changed = !form.password.is_empty();
        let new_has_password = if host_pw_changed {
            true
        } else {
            form.has_password
        };
        let username = optional_field(&form.username);

        if form.metadata_only {
            let Some(id) = form.id else {
                self.mode = AppMode::Normal;
                return Ok(());
            };
            let saved_name = form.name.clone();
            if host_pw_changed {
                if let Err(e) = self
                    .password_store
                    .set(&crate::credentials::host_key(id), &form.password)
                {
                    self.host_notice = Some(format!("Saved, but storing the password failed: {e}"));
                }
            }
            self.store.update_host(
                id,
                &HostUpdate {
                    label: Some(label),
                    group_id: Some(group_id),
                    identity_id: Some(identity_id),
                    tags: Some(tags),
                    has_password: Some(new_has_password),
                    username: Some(username.clone()),
                    ..Default::default()
                },
            )?;
            self.mode = AppMode::Normal;
            self.reload_hosts()?;
            self.restore_selection_by_name(&saved_name);
            return Ok(());
        }

        let address = form.address.trim();
        let name = form.name.trim();
        if address.is_empty() {
            self.host_notice = Some("Address is required".into());
            self.host_form = Some(form);
            return Ok(());
        }
        if name.is_empty() {
            self.host_notice = Some("Name (alias) is required".into());
            self.host_form = Some(form);
            return Ok(());
        }

        let port: u16 = match form.port.trim().parse() {
            Ok(p) if p > 0 => p,
            _ => {
                self.host_notice = Some("Port must be a positive number".into());
                self.host_form = Some(form);
                return Ok(());
            }
        };

        let os_icon = os_icon_from_index(form.os_icon_index);
        let proxy_jump = optional_field(&form.proxy_jump);
        let remote_command = optional_field(&form.remote_command);

        // Avoid the `hosts.name` UNIQUE constraint (which would otherwise abort
        // the app): if the name is taken, fall back to `name-2`, `name-3`, …
        // An edit keeps its own current name via `exclude_id`.
        let unique_name = self.store.unique_host_name(name, form.id)?;
        if unique_name != name {
            self.host_notice = Some(format!(
                "Name '{name}' already exists \u{2014} saved as '{unique_name}'"
            ));
        }
        let name = unique_name.as_str();
        let saved_name = name.to_string();
        if let Some(id) = form.id {
            if host_pw_changed {
                if let Err(e) = self
                    .password_store
                    .set(&crate::credentials::host_key(id), &form.password)
                {
                    self.host_notice = Some(format!("Saved, but storing the password failed: {e}"));
                }
            }
            self.store.update_host(
                id,
                &HostUpdate {
                    name: Some(name.to_string()),
                    label: Some(label),
                    address: Some(address.to_string()),
                    port: Some(port),
                    group_id: Some(group_id),
                    identity_id: Some(identity_id),
                    os_icon: Some(os_icon),
                    tags: Some(tags),
                    proxy_jump: Some(proxy_jump),
                    forward_agent: Some(form.forward_agent),
                    remote_command: Some(remote_command),
                    has_password: Some(new_has_password),
                    username: Some(username),
                    ..Default::default()
                },
            )?;
        } else {
            let created = self.store.create_host(&NewHost {
                name: name.to_string(),
                label,
                address: address.to_string(),
                port,
                group_id,
                identity_id,
                os_icon,
                tags,
                notes: None,
                proxy_jump,
                forward_agent: form.forward_agent,
                remote_command,
                source: HostSource::Launcher,
                has_password: new_has_password,
                username,
            })?;
            if host_pw_changed {
                if let Err(e) = self
                    .password_store
                    .set(&crate::credentials::host_key(created.id), &form.password)
                {
                    self.host_notice = Some(format!("Saved, but storing the password failed: {e}"));
                }
            }
        }

        self.mode = AppMode::Normal;
        self.reload_hosts()?;
        self.restore_selection_by_name(&saved_name);
        Ok(())
    }

    fn delete_selected_host(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };
        let Some(id) = self.hosts[host_idx].managed_id() else {
            self.host_notice = Some("Only launcher hosts can be deleted".into());
            return Ok(());
        };
        if self.hosts[host_idx].source() != HostSource::Launcher {
            self.host_notice = Some("Only launcher hosts can be deleted".into());
            return Ok(());
        }
        let name = self.hosts[host_idx].display_name().to_string();
        self.pending_delete = Some(PendingDelete::Host { id, name });
        self.mode = AppMode::ConfirmDelete;
        Ok(())
    }

    fn duplicate_selected_host(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };

        let copy_name = match &self.hosts[host_idx] {
            HostEntry::Managed(m) => {
                let Some(copy) = self.store.duplicate_host(m.id)? else {
                    self.host_notice = Some("Host not found".into());
                    return Ok(());
                };
                copy.name
            }
            HostEntry::Legacy { host, meta } => self.duplicate_legacy_to_launcher(host, meta)?,
        };

        self.reload_hosts()?;
        self.restore_selection_by_name(&copy_name);
        Ok(())
    }

    fn duplicate_legacy_to_launcher(
        &self,
        host: &SshHost,
        meta: &crate::metadata::HostMetadata,
    ) -> Result<String> {
        let mut name = format!("{}-copy", host.name);
        let mut suffix = 2u32;
        while self.store.get_host_by_name(&name)?.is_some() {
            name = format!("{}-copy-{}", host.name, suffix);
            suffix += 1;
        }

        let address = host.hostname.clone().unwrap_or_else(|| host.name.clone());
        let port = host.port.unwrap_or(22);

        let mut new_host = NewHost::launcher(name.clone(), address);
        new_host.port = port;
        new_host.tags = meta.tags.clone();
        new_host.notes = meta.description.clone();
        new_host.proxy_jump = host.proxy_jump.clone();
        new_host.forward_agent = host.forward_agent.unwrap_or(false);
        new_host.remote_command = host.remote_command.clone();
        new_host.identity_id = self.match_identity_for_ssh_host(host)?;
        self.store.create_host(&new_host)?;
        Ok(name)
    }

    fn match_identity_for_ssh_host(&self, host: &SshHost) -> Result<Option<i64>> {
        let user = host.user.as_deref();
        let key = host.identity_file.as_deref();
        if user.is_none() && key.is_none() {
            return Ok(None);
        }

        for identity in self.store.list_identities()? {
            let id_user = identity.username.as_deref();
            let id_key = identity
                .private_key
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned());
            let matches = match (user, key) {
                (Some(u), Some(k)) => id_user == Some(u) && id_key.as_deref() == Some(k),
                (Some(u), None) => id_user == Some(u),
                (None, Some(k)) => id_key.as_deref() == Some(k),
                (None, None) => false,
            };
            if matches {
                return Ok(Some(identity.id));
            }
        }

        let mut identity_name = format!("{}-identity", host.name);
        let mut suffix = 2u32;
        while self.store.get_identity_by_name(&identity_name)?.is_some() {
            identity_name = format!("{}-identity-{}", host.name, suffix);
            suffix += 1;
        }

        let created = self.store.create_identity(&NewIdentity {
            name: identity_name,
            username: host.user.clone(),
            private_key: key.map(PathBuf::from),
            ..Default::default()
        })?;
        Ok(Some(created.id))
    }

    fn handle_key_host_form(&mut self, key: KeyEvent) -> Result<()> {
        let Some(form) = self.host_form.as_ref() else {
            return Ok(());
        };
        let field = form.field;
        match key.code {
            KeyCode::Esc => self.cancel_host_form()?,
            _ if self.is_save_key(&key) => self.save_host_form()?,
            // Single-step model: type straight into the active field.
            // Enter/Tab/Down advance; Enter on the LAST field saves the form
            // (a modifier-free save path; F2/Ctrl+S always work).
            KeyCode::Enter if field == HostFormField::Group => self.open_field_picker(PickerKind::Group),
            KeyCode::Enter if field == HostFormField::Identity => {
                self.open_field_picker(PickerKind::Identity)
            }
            KeyCode::Enter if field == HostFormField::OsIcon => self.save_host_form()?,
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down if key.modifiers.is_empty() => {
                self.host_form_field_next();
            }
            KeyCode::BackTab | KeyCode::Up => self.host_form_field_prev(),
            KeyCode::Right if field.is_picker() || field.is_toggle() => {
                self.host_form_picker_scroll(1);
            }
            KeyCode::Left if field.is_picker() || field.is_toggle() => {
                self.host_form_picker_scroll(-1);
            }
            KeyCode::Char(' ')
                if key.modifiers.is_empty() && field == HostFormField::ForwardAgent =>
            {
                self.host_form_toggle();
            }
            KeyCode::Backspace => self.host_form_backspace(),
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control()
                    && !field.is_picker()
                    && !field.is_toggle() =>
            {
                self.host_form_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    fn host_form_field_next(&mut self) {
        let Some(form) = self.host_form.as_mut() else {
            return;
        };
        form.field = form.field.next();
        form.cursor = text_input::char_len(form.active_field());
    }

    fn host_form_field_prev(&mut self) {
        let Some(form) = self.host_form.as_mut() else {
            return;
        };
        form.field = form.field.prev();
        form.cursor = text_input::char_len(form.active_field());
    }

    /// Number of selectable rows in the dropdown (incl. the "+ New group" row).
    pub fn field_picker_len(&self, kind: PickerKind) -> usize {
        match kind {
            // (none) + groups + "+ New group…"
            PickerKind::Group => self.groups.len() + 2,
            PickerKind::Identity => self.identities.len(),
        }
    }

    /// Index of the "+ New group…" row (Group picker only).
    fn field_picker_create_index(&self) -> usize {
        self.groups.len() + 1
    }

    fn open_field_picker(&mut self, kind: PickerKind) {
        let Some(form) = self.host_form.as_ref() else {
            return;
        };
        if form.metadata_only && kind == PickerKind::Identity {
            // Identity is a connection field for imported hosts — read-only.
            return;
        }
        let selected = match kind {
            PickerKind::Group => form.group_index,
            PickerKind::Identity => form.identity_index,
        };
        self.field_picker = Some(FieldPicker {
            kind,
            selected,
            creating: None,
            cursor: 0,
        });
        self.mode = AppMode::FieldPicker;
    }

    fn handle_key_field_picker(&mut self, key: KeyEvent) -> Result<()> {
        let Some(picker) = self.field_picker.as_ref() else {
            self.mode = AppMode::HostForm;
            return Ok(());
        };

        // Inline "create new group" text entry.
        if picker.creating.is_some() {
            return self.handle_key_field_picker_creating(key);
        }

        let kind = picker.kind;
        let len = self.field_picker_len(kind);
        match key.code {
            KeyCode::Esc => {
                self.field_picker = None;
                self.mode = AppMode::HostForm;
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                if let Some(p) = self.field_picker.as_mut() {
                    p.selected = (p.selected + 1) % len.max(1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                if let Some(p) = self.field_picker.as_mut() {
                    p.selected = (p.selected + len.saturating_sub(1)) % len.max(1);
                }
            }
            KeyCode::Enter => self.field_picker_confirm()?,
            _ => {}
        }
        Ok(())
    }

    fn field_picker_confirm(&mut self) -> Result<()> {
        let Some(picker) = self.field_picker.as_ref() else {
            return Ok(());
        };
        match picker.kind {
            PickerKind::Group => {
                if picker.selected == self.field_picker_create_index() {
                    // Enter inline "new group" text entry.
                    if let Some(p) = self.field_picker.as_mut() {
                        p.creating = Some(String::new());
                        p.cursor = 0;
                    }
                    return Ok(());
                }
                let group_index = picker.selected;
                // Picking a group applies its default identity, if it has one.
                let default_identity_index = group_index
                    .checked_sub(1)
                    .and_then(|gi| self.groups.get(gi))
                    .and_then(|g| g.default_identity_id)
                    .and_then(|iid| self.identities.iter().position(|i| i.id == iid));
                if let Some(form) = self.host_form.as_mut() {
                    form.group_index = group_index;
                    if let Some(idx) = default_identity_index {
                        form.identity_index = idx;
                    }
                    form.dirty = true;
                }
            }
            PickerKind::Identity => {
                if let Some(form) = self.host_form.as_mut() {
                    form.identity_index = picker.selected;
                    form.dirty = true;
                }
            }
        }
        self.field_picker = None;
        self.mode = AppMode::HostForm;
        Ok(())
    }

    fn handle_key_field_picker_creating(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                // Back to the list, keep the dropdown open.
                if let Some(p) = self.field_picker.as_mut() {
                    p.creating = None;
                    p.cursor = 0;
                }
            }
            KeyCode::Enter => self.field_picker_create_group()?,
            KeyCode::Backspace => {
                if let Some(p) = self.field_picker.as_mut() {
                    if let Some(name) = p.creating.as_mut() {
                        let c = p.cursor;
                        p.cursor = text_input::backspace_at(name, c);
                    }
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                if let Some(p) = self.field_picker.as_mut() {
                    if let Some(name) = p.creating.as_mut() {
                        let cur = p.cursor;
                        p.cursor = text_input::insert_at(name, cur, c);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn field_picker_create_group(&mut self) -> Result<()> {
        let name = self
            .field_picker
            .as_ref()
            .and_then(|p| p.creating.clone())
            .unwrap_or_default();
        let name = name.trim().to_string();
        if name.is_empty() {
            return Ok(());
        }
        // Reuse an existing group with the same name instead of erroring.
        let id = match self.store.list_groups()?.into_iter().find(|g| g.name == name) {
            Some(g) => g.id,
            None => {
                self.store
                    .create_group(&crate::store::NewHostGroup {
                        name: name.clone(),
                        sort_order: self.groups.len() as i32,
                        default_identity_id: None,
                    })?
                    .id
            }
        };
        self.groups = self.store.list_groups()?;
        if let Some(form) = self.host_form.as_mut() {
            // group_index: 0 = (none), 1.. = groups in list order.
            form.group_index = self
                .groups
                .iter()
                .position(|g| g.id == id)
                .map(|i| i + 1)
                .unwrap_or(0);
            form.dirty = true;
        }
        self.field_picker = None;
        self.mode = AppMode::HostForm;
        Ok(())
    }

    fn host_form_picker_scroll(&mut self, delta: i32) {
        let Some(form) = self.host_form.as_mut() else {
            return;
        };
        if !form.field.is_picker() && !form.field.is_toggle() {
            return;
        }
        if form.field == HostFormField::ForwardAgent {
            form.forward_agent = !form.forward_agent;
            form.dirty = true;
            return;
        }
        match form.field {
            HostFormField::Group => {
                let max = self.groups.len();
                let next = form.group_index as i32 + delta;
                form.group_index = next.clamp(0, max as i32) as usize;
                form.dirty = true;
            }
            HostFormField::Identity => {
                if !self.identities.is_empty() {
                    let max = self.identities.len() - 1;
                    let next = form.identity_index as i32 + delta;
                    form.identity_index = next.clamp(0, max as i32) as usize;
                    form.dirty = true;
                }
            }
            HostFormField::OsIcon => {
                let max = OS_ICON_OPTIONS.len().saturating_sub(1);
                let next = form.os_icon_index as i32 + delta;
                form.os_icon_index = next.clamp(0, max as i32) as usize;
                form.dirty = true;
            }
            _ => {}
        }
    }

    fn host_form_toggle(&mut self) {
        let Some(form) = self.host_form.as_mut() else {
            return;
        };
        if form.metadata_only && form.field.is_connection_field() {
            return;
        }
        if form.field == HostFormField::ForwardAgent {
            form.forward_agent = !form.forward_agent;
            form.dirty = true;
        }
    }

    fn host_form_backspace(&mut self) {
        let Some(form) = self.host_form.as_mut() else {
            return;
        };
        if form.metadata_only && form.field.is_connection_field() {
            return;
        }
        if form.field.is_picker() || form.field.is_toggle() {
            return;
        }
        let c = form.cursor;
        if c > 0 {
            form.cursor = text_input::backspace_at(form.active_field_mut(), c);
            form.dirty = true;
        }
    }

    fn host_form_insert(&mut self, ch: char) {
        let Some(form) = self.host_form.as_mut() else {
            return;
        };
        if form.metadata_only && form.field.is_connection_field() {
            return;
        }
        if form.field.is_picker() || form.field.is_toggle() {
            return;
        }
        let c = form.cursor;
        form.cursor = text_input::insert_at(form.active_field_mut(), c, ch);
        form.dirty = true;
    }

    fn enter_host_detail(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };
        let tags = self.hosts[host_idx].tags().join(", ");
        let description = self.hosts[host_idx]
            .description()
            .unwrap_or_default()
            .to_string();
        let environment = self.hosts[host_idx]
            .environment()
            .unwrap_or_default()
            .to_string();
        self.detail_edit = Some(HostDetailEdit {
            tags: tags.clone(),
            description,
            environment,
            field: DetailEditField::Tags,
            cursor: text_input::char_len(&tags),
        });
        self.mode = AppMode::HostDetail;
        Ok(())
    }

    fn cancel_host_detail(&mut self) -> Result<()> {
        if let Some(host_idx) = self.selected_host_index() {
            let host_name = self.hosts[host_idx].name().to_string();
            if let Some((_, meta)) = self.hosts[host_idx].legacy_mut() {
                if let Some(stored) = self.metadata.get(&host_name)? {
                    *meta = stored;
                }
            }
        }
        self.detail_edit = None;
        self.mode = AppMode::Normal;
        Ok(())
    }

    fn save_host_detail(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            self.detail_edit = None;
            self.mode = AppMode::Normal;
            return Ok(());
        };
        let Some(edit) = self.detail_edit.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        let host_name = self.hosts[host_idx].name().to_string();
        let favorite = self.hosts[host_idx].favorite();
        let last_connected = self.hosts[host_idx].last_connected();
        let description = optional_field(&edit.description);
        let environment = optional_field(&edit.environment);
        let tags = parse_tags(&edit.tags);

        // Managed hosts (launcher + imported ssh_config rows) live in
        // launcher.db — persist there, or the edit is lost on reload.
        if let HostEntry::Managed(managed) = &self.hosts[host_idx] {
            let id = managed.id;
            let update = crate::store::HostUpdate {
                tags: Some(tags),
                notes: Some(description),
                environment: Some(environment),
                ..Default::default()
            };
            if let Some(updated) = self.store.update_host(id, &update)? {
                self.hosts[host_idx] = HostEntry::Managed(updated);
            }
        } else {
            let meta = crate::metadata::HostMetadata {
                host_name: host_name.clone(),
                tags,
                description,
                environment,
                favorite,
                last_connected,
            };
            self.metadata.upsert(&meta)?;
            if let Some((_, stored_meta)) = self.hosts[host_idx].legacy_mut() {
                *stored_meta = meta;
            }
        }
        self.rebuild_filter();
        self.mode = AppMode::Normal;
        Ok(())
    }

    fn detail_edit_field_next(&mut self) {
        let Some(edit) = self.detail_edit.as_mut() else {
            return;
        };
        edit.field = edit.field.next();
        edit.cursor = text_input::char_len(edit.active_field());
    }

    fn detail_edit_field_prev(&mut self) {
        let Some(edit) = self.detail_edit.as_mut() else {
            return;
        };
        edit.field = edit.field.prev();
        edit.cursor = text_input::char_len(edit.active_field());
    }

    fn detail_edit_backspace(&mut self) {
        let Some(edit) = self.detail_edit.as_mut() else {
            return;
        };
        let c = edit.cursor;
        edit.cursor = text_input::backspace_at(edit.active_field_mut(), c);
    }

    fn detail_edit_insert(&mut self, ch: char) {
        let Some(edit) = self.detail_edit.as_mut() else {
            return;
        };
        let c = edit.cursor;
        edit.cursor = text_input::insert_at(edit.active_field_mut(), c, ch);
    }

    fn exit_search(&mut self, reset_filter: bool) {
        self.search_query.clear();
        self.mode = AppMode::Normal;
        if reset_filter {
            self.tag_filter = None;
        }
        self.rebuild_filter();
    }

    fn move_selection(&mut self, delta: i32) {
        if self.nav_rows.is_empty() {
            self.selected = 0;
            return;
        }
        let len = self.nav_rows.len() as i32;
        let next = self.selected as i32 + delta;
        // Wrap around: going past the end wraps to the beginning and vice versa
        self.selected = ((next % len + len) % len) as usize;
    }

    /// Begin quitting: show the confirmation dialog, or quit immediately when
    /// confirmation is disabled in config.
    fn request_quit(&mut self) {
        if !self.config.appearance.confirm_quit {
            self.should_quit = true;
            return;
        }
        if self.mode != AppMode::ConfirmQuit {
            self.pre_quit_mode = Some(self.mode);
            self.mode = AppMode::ConfirmQuit;
        }
    }

    fn handle_key_confirm_quit(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                self.should_quit = true;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.mode = self.pre_quit_mode.take().unwrap_or(AppMode::Normal);
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_key_keybind_editor(&mut self, key: KeyEvent) -> Result<()> {
        let Some(editor) = self.keybind_editor else {
            self.mode = AppMode::Normal;
            return Ok(());
        };

        if editor.capturing {
            if key.code != KeyCode::Esc {
                if let Some(spec) = keyevent_to_spec(&key) {
                    let action = KeyAction::ALL[editor.selected];
                    if editor.append {
                        self.config.keybinds.add(action, spec);
                    } else {
                        self.config.keybinds.set(action, vec![spec]);
                    }
                    self.save_config_quietly();
                }
            }
            if let Some(e) = self.keybind_editor.as_mut() {
                e.capturing = false;
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.keybind_editor = None;
                self.mode = AppMode::Normal;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.selected = (e.selected + 1) % KeyAction::ALL.len();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.selected =
                        (e.selected + KeyAction::ALL.len() - 1) % KeyAction::ALL.len();
                }
            }
            // Enter/c: replace with a single new key. a: add another binding.
            KeyCode::Enter | KeyCode::Char('c') => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.capturing = true;
                    e.append = false;
                }
            }
            KeyCode::Char('a') => {
                if let Some(e) = self.keybind_editor.as_mut() {
                    e.capturing = true;
                    e.append = true;
                }
            }
            KeyCode::Char('r') => {
                let action = KeyAction::ALL[editor.selected];
                self.config.keybinds.reset_action(action);
                self.save_config_quietly();
            }
            KeyCode::Char('x') => {
                // Unbind the action entirely.
                let action = KeyAction::ALL[editor.selected];
                self.config.keybinds.set(action, Vec::new());
                self.save_config_quietly();
            }
            _ => {}
        }
        Ok(())
    }

    /// Persist config, surfacing failures as a non-fatal host notice.
    fn save_config_quietly(&mut self) {
        if let Err(e) = crate::config::save_config(&self.config) {
            self.host_notice = Some(format!("Could not save config: {e}"));
        }
    }

    /// Short human label of the configured save keys, e.g. `"F2/Ctrl+S"`,
    /// for form hints.
    pub fn save_key_label(&self) -> String {
        let keys = &self.config.keybinds.save;
        if keys.is_empty() {
            "F2".to_string()
        } else {
            keys.join("/")
        }
    }

    /// Whether `key` matches one of the user-configured bindings for `action`.
    pub fn is_action(&self, action: KeyAction, key: &KeyEvent) -> bool {
        self.config
            .keybinds
            .binds(action)
            .iter()
            .filter_map(|spec| parse_keyspec(spec))
            .any(|(code, mods)| keyspec_matches(code, mods, key))
    }

    /// Whether `key` matches the configured "save" binding (default F2/Ctrl+S).
    pub fn is_save_key(&self, key: &KeyEvent) -> bool {
        self.is_action(KeyAction::Save, key)
    }

    /// The section index if the current selection is a group header.
    pub fn selected_nav_header(&self) -> Option<usize> {
        match self.nav_rows.get(self.selected) {
            Some(NavRow::Header(si)) => Some(*si),
            _ => None,
        }
    }

    fn load_collapsed_groups(&mut self) {
        if let Ok(Some(raw)) = self.store.get_ui_state("collapsed_groups") {
            if let Ok(ids) = serde_json::from_str::<Vec<i64>>(&raw) {
                self.collapsed_groups = ids.into_iter().collect();
            }
        }
    }

    fn persist_collapsed_groups(&self) {
        let mut ids: Vec<i64> = self.collapsed_groups.iter().copied().collect();
        ids.sort_unstable();
        if let Ok(json) = serde_json::to_string(&ids) {
            let _ = self.store.set_ui_state("collapsed_groups", &json);
        }
    }

    /// Toggle collapse of the group header under the selection, keeping the
    /// selection on that header, and persist the new state.
    fn toggle_selected_group(&mut self) {
        if let Some(si) = self.selected_nav_header() {
            self.toggle_group_by_section(si);
        }
    }

    fn toggle_group_by_section(&mut self, si: usize) {
        let Some(section) = self.group_sections.get(si) else {
            return;
        };
        let key = section.key();
        if !self.collapsed_groups.remove(&key) {
            self.collapsed_groups.insert(key);
        }
        self.persist_collapsed_groups();
        self.rebuild_filter();
        if let Some(pos) = self.nav_rows.iter().position(
            |r| matches!(r, NavRow::Header(s) if self.group_sections[*s].key() == key),
        ) {
            self.selected = pos;
        }
    }

    /// Collapse (`false`) or expand (`true`) every group at once.
    fn set_all_groups_collapsed(&mut self, collapsed: bool) {
        if collapsed {
            self.collapsed_groups = self.group_sections.iter().map(|s| s.key()).collect();
        } else {
            self.collapsed_groups.clear();
        }
        self.persist_collapsed_groups();
        let sel_key = self
            .selected_nav_header()
            .map(|si| self.group_sections[si].key());
        self.rebuild_filter();
        if let Some(key) = sel_key {
            if let Some(pos) = self.nav_rows.iter().position(
                |r| matches!(r, NavRow::Header(s) if self.group_sections[*s].key() == key),
            ) {
                self.selected = pos;
            }
        }
    }

    fn toggle_favorite(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };

        if let HostEntry::Managed(m) = &self.hosts[host_idx] {
            let id = m.id;
            let new_fav = !m.favorite;
            self.store.update_host(
                id,
                &HostUpdate {
                    favorite: Some(new_fav),
                    ..Default::default()
                },
            )?;
            if let HostEntry::Managed(m) = &mut self.hosts[host_idx] {
                m.favorite = new_fav;
            }
            return Ok(());
        }

        let host_name = self.hosts[host_idx].name().to_string();
        self.metadata.toggle_favorite(&host_name)?;
        if let Some((_, meta)) = self.hosts[host_idx].legacy_mut() {
            if let Some(stored) = self.metadata.get(&host_name)? {
                meta.favorite = stored.favorite;
            }
        }
        Ok(())
    }

    pub fn cycle_sort_mode(&mut self) {
        self.sort_mode = self.sort_mode.next();
        self.rebuild_filter();
    }

    fn move_host_manual(&mut self, delta: i32) -> Result<()> {
        if self.sort_mode != SortMode::Manual {
            return Ok(());
        }
        let Some(id) = self.selected_entry().and_then(|e| e.managed_id()) else {
            return Ok(());
        };
        let name = self.selected_entry().map(|e| e.name().to_string());
        // Find the adjacent *host* nav row in the requested direction (skip
        // group headers so manual reorder only swaps hosts).
        let mut probe = self.selected as i32 + delta;
        let other_idx = loop {
            if probe < 0 || probe >= self.nav_rows.len() as i32 {
                return Ok(());
            }
            match self.nav_rows[probe as usize] {
                NavRow::Host(i) => break i,
                NavRow::Header(_) => probe += delta,
            }
        };
        let Some(other_id) = self.hosts[other_idx].managed_id() else {
            return Ok(());
        };

        self.store.swap_host_sort_orders(id, other_id)?;
        self.reload_hosts()?;
        if let Some(name) = name {
            self.restore_selection_by_name(&name);
        }
        Ok(())
    }

    pub(crate) fn rebuild_filter(&mut self) {
        let candidates: Vec<usize> = if let Some(tag) = &self.tag_filter {
            self.hosts
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.tags().iter().any(|t| t == tag))
                .map(|(idx, _)| idx)
                .collect()
        } else {
            (0..self.hosts.len()).collect()
        };

        let entries: Vec<HostEntry> = candidates
            .iter()
            .map(|&idx| self.hosts[idx].clone())
            .collect();
        let local_matches = self.search.update_query(&entries, &self.search_query);
        let mut filtered: Vec<usize> = local_matches
            .into_iter()
            .map(|local_idx| candidates[local_idx])
            .collect();

        sort_host_indices(&self.hosts, &mut filtered, self.sort_mode);
        // Partition by group, then flatten back so filtered_indices walks in
        // visual order. Within each section the existing sort_mode order is
        // preserved by build_group_sections. Without this, j/k steps through
        // the alphabetical list while the screen shows grouped sections, so
        // moving past a grouped host visually "teleports" to the group at the
        // top of the list and back.
        self.group_sections = build_group_sections(&self.hosts, &self.groups, &filtered);
        self.filtered_indices = self
            .group_sections
            .iter()
            .flat_map(|s| s.host_indices.iter().copied())
            .collect();

        // Tree mode (navigable, collapsible headers) kicks in only once the
        // user has real groups — a pure ssh_config list stays a flat host list.
        let tree_mode = !self.groups.is_empty();
        let mut nav = Vec::new();
        for (si, section) in self.group_sections.iter_mut().enumerate() {
            section.collapsed = tree_mode && self.collapsed_groups.contains(&section.key());
            if tree_mode {
                nav.push(NavRow::Header(si));
            }
            if !section.collapsed {
                nav.extend(section.host_indices.iter().map(|&h| NavRow::Host(h)));
            }
        }
        self.nav_rows = nav;
        self.clamp_selected();
    }

    fn clamp_selected(&mut self) {
        if self.nav_rows.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.nav_rows.len() {
            self.selected = self.nav_rows.len() - 1;
        }
    }

    fn restore_selection_by_name(&mut self, name: &str) {
        let host_idx = self
            .hosts
            .iter()
            .position(|h| h.name() == name);
        if let Some(hi) = host_idx {
            if let Some(pos) = self
                .nav_rows
                .iter()
                .position(|r| matches!(r, NavRow::Host(i) if *i == hi))
            {
                self.selected = pos;
                return;
            }
        }
        self.clamp_selected();
    }
}

fn optional_path(raw: &str) -> Option<std::path::PathBuf> {
    optional_field(raw).map(std::path::PathBuf::from)
}

impl HostFormEdit {
    pub fn active_field(&self) -> &str {
        match self.field {
            HostFormField::Address => &self.address,
            HostFormField::Username => &self.username,
            HostFormField::Label => &self.label,
            HostFormField::Name => &self.name,
            HostFormField::Port => &self.port,
            HostFormField::Group | HostFormField::Identity | HostFormField::OsIcon => "",
            HostFormField::Tags => &self.tags,
            HostFormField::ProxyJump => &self.proxy_jump,
            HostFormField::RemoteCommand => &self.remote_command,
            HostFormField::ForwardAgent => "",
            HostFormField::Password => &self.password,
        }
    }

    fn active_field_mut(&mut self) -> &mut String {
        match self.field {
            HostFormField::Address => &mut self.address,
            HostFormField::Username => &mut self.username,
            HostFormField::Label => &mut self.label,
            HostFormField::Name => &mut self.name,
            HostFormField::Port => &mut self.port,
            HostFormField::Group | HostFormField::Identity | HostFormField::OsIcon => {
                &mut self.address
            }
            HostFormField::Tags => &mut self.tags,
            HostFormField::ProxyJump => &mut self.proxy_jump,
            HostFormField::RemoteCommand => &mut self.remote_command,
            HostFormField::ForwardAgent => &mut self.address,
            HostFormField::Password => &mut self.password,
        }
    }
}

/// Write an OSC 52 set-clipboard sequence to stdout. Modern terminals
/// (kitty / iTerm2 / wezterm / Alacritty / foot) interpret this as
/// "put this base64-encoded payload on the system clipboard". The
/// sequence is invisible to the alternate-screen UI — the host terminal
/// consumes it before it ever lands on a buffer cell.
fn write_osc52(text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let encoded = base64_encode(text.as_bytes());
    let payload = format!("\x1b]52;c;{encoded}\x07");
    let mut out = std::io::stdout().lock();
    out.write_all(payload.as_bytes())?;
    out.flush()
}

/// Tiny base64 (standard alphabet, padded). Inlined so we don't pull in
/// another crate for a single ~20 line helper used in one place.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut chunks = input.chunks_exact(3);
    for chunk in chunks.by_ref() {
        let b = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        out.push(ALPHABET[((b >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((b >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((b >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(b & 0x3f) as usize] as char);
    }
    let rem = chunks.remainder();
    match rem.len() {
        1 => {
            let b = (rem[0] as u32) << 16;
            out.push(ALPHABET[((b >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((b >> 12) & 0x3f) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let b = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
            out.push(ALPHABET[((b >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((b >> 12) & 0x3f) as usize] as char);
            out.push(ALPHABET[((b >> 6) & 0x3f) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

/// Expand a leading `~` (or `~/`) in a path to the user's home directory.
fn shellexpand_home(path: &str) -> std::path::PathBuf {
    if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home);
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home).join(rest);
        }
    }
    std::path::PathBuf::from(path)
}

fn os_icon_from_index(index: usize) -> Option<String> {
    match OS_ICON_OPTIONS.get(index) {
        Some(&"(none)") | None => None,
        Some(s) => Some((*s).to_string()),
    }
}

fn os_icon_index_from_option(icon: &Option<String>) -> usize {
    icon.as_deref()
        .and_then(|name| OS_ICON_OPTIONS.iter().position(|opt| *opt == name))
        .unwrap_or(0)
}

impl IdentityFormEdit {
    pub fn active_field(&self) -> &str {
        match self.field {
            IdentityFormField::Name => &self.name,
            IdentityFormField::Username => &self.username,
            IdentityFormField::PrivateKey => &self.private_key,
            IdentityFormField::Certificate => &self.certificate,
            IdentityFormField::Password => &self.password,
        }
    }

    fn active_field_mut(&mut self) -> &mut String {
        match self.field {
            IdentityFormField::Name => &mut self.name,
            IdentityFormField::Username => &mut self.username,
            IdentityFormField::PrivateKey => &mut self.private_key,
            IdentityFormField::Certificate => &mut self.certificate,
            IdentityFormField::Password => &mut self.password,
        }
    }

    /// Typing over a pasted key blob discards it (the field reverts to a
    /// plain path input).
    fn clear_pasted_key_marker(&mut self) {
        if self.field == IdentityFormField::PrivateKey && self.pasted_key.is_some() {
            self.pasted_key = None;
            self.private_key.clear();
            self.cursor = 0;
        }
    }
}

impl HostDetailEdit {
    pub fn active_field(&self) -> &str {
        match self.field {
            DetailEditField::Tags => &self.tags,
            DetailEditField::Description => &self.description,
            DetailEditField::Environment => &self.environment,
        }
    }

    fn active_field_mut(&mut self) -> &mut String {
        match self.field {
            DetailEditField::Tags => &mut self.tags,
            DetailEditField::Description => &mut self.description,
            DetailEditField::Environment => &mut self.environment,
        }
    }
}

fn parse_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn sort_host_indices(hosts: &[HostEntry], indices: &mut [usize], mode: SortMode) {
    indices.sort_by(|&a, &b| compare_hosts(&hosts[a], &hosts[b], mode));
}

fn compare_hosts(a: &HostEntry, b: &HostEntry, mode: SortMode) -> std::cmp::Ordering {
    match mode {
        SortMode::Label => label_cmp(a, b),
        SortMode::LastConnected => match (b.last_connected(), a.last_connected()) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => label_cmp(a, b),
        },
        SortMode::FavoriteFirst => match (a.favorite(), b.favorite()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => label_cmp(a, b),
        },
        SortMode::GroupThenLabel => group_sort_key(a)
            .cmp(&group_sort_key(b))
            .then_with(|| label_cmp(a, b)),
        SortMode::Manual => a
            .sort_order()
            .cmp(&b.sort_order())
            .then_with(|| a.name().cmp(b.name())),
    }
}

fn label_cmp(a: &HostEntry, b: &HostEntry) -> std::cmp::Ordering {
    a.display_name()
        .to_lowercase()
        .cmp(&b.display_name().to_lowercase())
}

fn group_sort_key(entry: &HostEntry) -> String {
    match entry.managed().and_then(|m| m.group.as_ref()) {
        Some(g) => format!("{:08}_{}", g.sort_order, g.name.to_lowercase()),
        None => format!("z_{UNGROUPED_LABEL}"),
    }
}

fn build_group_sections(
    hosts: &[HostEntry],
    groups: &[HostGroup],
    filtered: &[usize],
) -> Vec<HostGroupSection> {
    let mut sections = Vec::new();

    for group in groups {
        let host_indices: Vec<usize> = filtered
            .iter()
            .copied()
            .filter(|&idx| hosts[idx].group_id() == Some(group.id))
            .collect();
        sections.push(HostGroupSection {
            group: Some(group.clone()),
            label: group.name.clone(),
            host_indices,
            collapsed: false,
        });
    }

    let ungrouped: Vec<usize> = filtered
        .iter()
        .copied()
        .filter(|&idx| hosts[idx].group_id().is_none())
        .collect();
    if !ungrouped.is_empty() {
        sections.push(HostGroupSection {
            group: None,
            label: UNGROUPED_LABEL.to_string(),
            host_indices: ungrouped,
            collapsed: false,
        });
    }

    sections
}

/// Parse a keybinding spec like `"Ctrl+S"`, `"F2"`, `"Alt+Enter"` into a
/// (code, modifiers) pair. Returns `None` for unrecognised specs.
fn parse_keyspec(spec: &str) -> Option<(KeyCode, KeyModifiers)> {
    let parts: Vec<&str> = spec.split('+').map(|p| p.trim()).collect();
    let (key_part, mod_parts) = parts.split_last()?;
    let mut mods = KeyModifiers::empty();
    for m in mod_parts {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "alt" | "option" => mods |= KeyModifiers::ALT,
            "shift" => mods |= KeyModifiers::SHIFT,
            _ => return None,
        }
    }
    let key = key_part.trim();
    if key.is_empty() {
        return None;
    }
    let code = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "esc" | "escape" => KeyCode::Esc,
        lower => {
            // Function key "F1".."F12"?
            if let Some(n) = lower
                .strip_prefix('f')
                .filter(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
                .and_then(|r| r.parse::<u8>().ok())
            {
                KeyCode::F(n)
            } else if lower.chars().count() == 1 {
                KeyCode::Char(lower.chars().next().unwrap())
            } else {
                return None;
            }
        }
    };
    Some((code, mods))
}

/// Serialize an incoming key event into a spec string (inverse of
/// [`parse_keyspec`]) for capturing a binding in the UI. Returns `None` for
/// keys that can't be a binding (bare modifiers, unsupported codes).
fn keyevent_to_spec(key: &KeyEvent) -> Option<String> {
    let base = match key.code {
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Char(c) => c.to_ascii_uppercase().to_string(),
        _ => return None,
    };
    let mut out = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        out.push_str("Ctrl+");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        out.push_str("Alt+");
    }
    // Shift is only meaningful for keys that aren't already shifted into a
    // distinct char (e.g. Shift+H stays "Shift+H"; '?' has no Shift prefix).
    if key.modifiers.contains(KeyModifiers::SHIFT) && !matches!(key.code, KeyCode::Char(c) if !c.is_ascii_alphabetic())
    {
        out.push_str("Shift+");
    }
    out.push_str(&base);
    Some(out)
}

/// Match a parsed spec against an incoming event, comparing char keys
/// case-insensitively (so `Ctrl+S` matches whatever case crossterm reports).
fn keyspec_matches(code: KeyCode, mods: KeyModifiers, key: &KeyEvent) -> bool {
    let code_eq = match (code, key.code) {
        (KeyCode::Char(a), KeyCode::Char(b)) => a.eq_ignore_ascii_case(&b),
        (a, b) => a == b,
    };
    code_eq && key.modifiers == mods
}

fn optional_field(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn tab_from_x(x: u16) -> Option<usize> {
    // Tab bar layout (from tab_bar.rs): 1-char left margin, then per tab:
    // 4 chars for number+brackets + label_len + 3 chars gap
    // Labels: "hosts"(5), "tunnels"(7), "identities"(10), "audit"(5)
    let labels = [5u16, 7, 10, 5];
    let mut cx = 1u16; // 1-char margin
    for (i, label_len) in labels.iter().enumerate() {
        let tab_w = 4 + label_len + 3;
        if x >= cx && x < cx + tab_w {
            return Some(i);
        }
        cx += tab_w;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{LauncherStore, NewHost};
    use std::collections::HashMap;

    fn test_store() -> Arc<LauncherStore> {
        Arc::new(LauncherStore::open_in_memory().unwrap())
    }

    struct MockResolver {
        hosts: HashMap<String, SshHost>,
        order: Vec<String>,
    }

    impl MockResolver {
        fn new(entries: Vec<(&str, SshHost)>) -> Self {
            let mut hosts = HashMap::new();
            let mut order = Vec::new();
            for (name, host) in entries {
                order.push(name.to_string());
                hosts.insert(name.to_string(), host);
            }
            Self { hosts, order }
        }
    }

    impl HostResolver for MockResolver {
        fn list_hosts(&self) -> Result<Vec<String>> {
            Ok(self.order.clone())
        }

        fn resolve_host(&self, name: &str) -> Result<SshHost> {
            self.hosts
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unknown host {name}"))
        }
    }

    struct RecordingLauncher {
        last: Arc<std::sync::Mutex<Option<String>>>,
    }

    impl RecordingLauncher {
        fn new() -> (Self, Arc<std::sync::Mutex<Option<String>>>) {
            let last = Arc::new(std::sync::Mutex::new(None));
            (
                Self {
                    last: Arc::clone(&last),
                },
                last,
            )
        }

        fn take(last: &Arc<std::sync::Mutex<Option<String>>>) -> Option<String> {
            last.lock().ok()?.take()
        }
    }

    impl TerminalLauncher for RecordingLauncher {
        fn launch(&self, host: &SshHost) -> Result<()> {
            if let Ok(mut guard) = self.last.lock() {
                *guard = Some(host.name.clone());
            }
            Ok(())
        }

        fn launch_ssh_argv(&self, ssh_argv: &[String]) -> Result<()> {
            // Record last argument (the hostname/alias) for test assertions
            if let Ok(mut guard) = self.last.lock() {
                *guard = ssh_argv.last().cloned();
            }
            Ok(())
        }
    }

    fn test_app(hosts: Vec<(&str, SshHost)>) -> App {
        let resolver = MockResolver::new(hosts);
        let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
        let (launcher, _launched) = RecordingLauncher::new();
        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(resolver),
                metadata,
                store: test_store(),
                launcher: Box::new(launcher),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.reload_hosts().unwrap();
        app
    }

    fn host(name: &str) -> SshHost {
        let mut h = SshHost::new(name);
        h.hostname = Some(format!("{name}.example.com"));
        h
    }

    #[test]
    fn keyevent_to_spec_roundtrips() {
        let f2 = KeyEvent::new(KeyCode::F(2), KeyModifiers::empty());
        assert_eq!(keyevent_to_spec(&f2).as_deref(), Some("F2"));
        let ctrl_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(keyevent_to_spec(&ctrl_s).as_deref(), Some("Ctrl+S"));
        // Round-trips through parse_keyspec back to a matching event.
        let spec = keyevent_to_spec(&ctrl_s).unwrap();
        let (code, mods) = parse_keyspec(&spec).unwrap();
        assert!(keyspec_matches(code, mods, &ctrl_s));
    }

    #[test]
    fn keybind_editor_captures_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SSHUB_CONFIG_DIR", dir.path());

        let mut app = test_app(vec![("web", host("web"))]);
        // Open the editor (Ctrl+K).
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.mode, AppMode::KeybindEditor);

        // Row 0 is "Save". Enter starts capture; press F10 to bind it.
        app.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(app.keybind_editor.unwrap().capturing);
        app.handle_key(key(KeyCode::F(10))).unwrap();
        assert!(!app.keybind_editor.unwrap().capturing);

        assert_eq!(app.config.keybinds.save, vec!["F10".to_string()]);
        assert!(app.is_save_key(&key(KeyCode::F(10))));
        assert!(!app.is_save_key(&key(KeyCode::F(2))));

        // Persisted to config.toml under the temp dir.
        let saved = crate::config::load_config().unwrap();
        assert_eq!(saved.keybinds.save, vec!["F10".to_string()]);

        // 'a' adds another binding without replacing.
        app.handle_key(key_char('a')).unwrap();
        assert!(app.keybind_editor.unwrap().append);
        app.handle_key(key(KeyCode::F(12))).unwrap();
        assert_eq!(app.config.keybinds.save, vec!["F10".to_string(), "F12".to_string()]);
        assert!(app.is_save_key(&key(KeyCode::F(10))));
        assert!(app.is_save_key(&key(KeyCode::F(12))));

        // 'x' unbinds the action entirely.
        app.handle_key(key_char('x')).unwrap();
        assert!(app.config.keybinds.save.is_empty());
        assert!(!app.is_save_key(&key(KeyCode::F(10))));

        // 'r' resets the selected action to defaults.
        app.handle_key(key_char('r')).unwrap();
        assert_eq!(app.config.keybinds.save, vec!["F2", "Ctrl+S"]);

        std::env::remove_var("SSHUB_CONFIG_DIR");
    }

    #[test]
    fn multiline_paste_into_form_stays_in_field() {
        let mut app = test_app(vec![("web", host("web"))]);
        app.active_tab = 2; // keys tab
        app.enter_identity_form(None).unwrap();
        assert_eq!(app.mode, AppMode::IdentityForm);

        // Navigate to the Private key path field.
        while app.identity_form.as_ref().unwrap().field != IdentityFormField::PrivateKey {
            app.handle_key(key(KeyCode::Down)).unwrap();
        }

        // Paste a multi-line PEM blob. Previously the newlines fired
        // Enter/save and the rest ran as commands; now it must all stay put.
        let key_blob = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc123\ndef456\n-----END OPENSSH PRIVATE KEY-----\n";
        app.handle_paste(key_blob).unwrap();

        // Still in the form, on the same field, no host connection triggered.
        assert_eq!(app.mode, AppMode::IdentityForm);
        let form = app.identity_form.as_ref().unwrap();
        assert_eq!(form.field, IdentityFormField::PrivateKey);
        // Key material captured as a blob (written to a file on save).
        assert_eq!(form.pasted_key.as_deref(), Some(key_blob));
        assert!(form.private_key.contains("pasted key"));
    }

    #[test]
    fn pasted_key_material_is_written_to_a_file_on_save() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());

        let mut app = test_app(vec![("web", host("web"))]);
        app.active_tab = 2;
        app.enter_identity_form(None).unwrap();

        // Name the identity, then paste key material into the key field.
        for c in "pasted-id".chars() {
            app.handle_key(key_char(c)).unwrap();
        }
        while app.identity_form.as_ref().unwrap().field != IdentityFormField::PrivateKey {
            app.handle_key(key(KeyCode::Down)).unwrap();
        }
        let blob = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc123\n-----END OPENSSH PRIVATE KEY-----";
        app.handle_paste(blob).unwrap();
        assert!(app.identity_form.as_ref().unwrap().pasted_key.is_some());

        app.handle_key(key(KeyCode::F(2))).unwrap(); // save
        assert_eq!(app.mode, AppMode::Normal);

        let created = app
            .store
            .get_identity_by_name("pasted-id")
            .unwrap()
            .expect("identity created");
        let path = created.private_key.expect("key path set");
        assert!(path.to_string_lossy().contains("sshub_pasted-id"));
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("BEGIN OPENSSH PRIVATE KEY"));
        assert!(contents.ends_with('\n'));

        std::env::remove_var("HOME");
    }

    #[test]
    fn backspace_discards_pasted_key_blob() {
        let mut app = test_app(vec![("web", host("web"))]);
        app.active_tab = 2;
        app.enter_identity_form(None).unwrap();
        while app.identity_form.as_ref().unwrap().field != IdentityFormField::PrivateKey {
            app.handle_key(key(KeyCode::Down)).unwrap();
        }
        app.handle_paste("-----BEGIN OPENSSH PRIVATE KEY-----\nx\n-----END OPENSSH PRIVATE KEY-----")
            .unwrap();
        assert!(app.identity_form.as_ref().unwrap().pasted_key.is_some());

        app.handle_key(key(KeyCode::Backspace)).unwrap();
        let form = app.identity_form.as_ref().unwrap();
        assert!(form.pasted_key.is_none());
        assert!(form.private_key.is_empty());
    }

    #[test]
    fn identity_grid_navigation_moves_by_row_and_column() {
        let mut app = test_app(vec![("web", host("web"))]);
        app.terminal_area = ratatui::layout::Rect::new(0, 0, 140, 40); // wide → 2 cols
        app.identities = (0..5)
            .map(|i| crate::store::Identity {
                id: i,
                name: format!("id{i}"),
                username: None,
                private_key: None,
                certificate: None,
                has_password: false,
            })
            .collect();
        // Grid: [0,1] [2,3] [4]
        app.identity_selected = 0;
        app.move_identity_grid(0, 1);
        assert_eq!(app.identity_selected, 1, "right");
        app.move_identity_grid(0, 1);
        assert_eq!(app.identity_selected, 1, "right at edge stays");
        app.move_identity_grid(1, 0);
        assert_eq!(app.identity_selected, 3, "down a row, same column");
        app.move_identity_grid(0, -1);
        assert_eq!(app.identity_selected, 2, "left");
        app.move_identity_grid(1, 0);
        assert_eq!(app.identity_selected, 4, "down into last row");
        app.move_identity_grid(1, 0);
        assert_eq!(app.identity_selected, 4, "no row below, stays");
        app.identity_selected = 3;
        app.move_identity_grid(1, 0);
        assert_eq!(app.identity_selected, 4, "down from col1 drops onto shorter last row");
    }

    #[test]
    fn keyless_identity_secret_is_a_login_password() {
        use std::collections::HashMap;
        use std::sync::Mutex;
        struct MapStore(Mutex<HashMap<String, String>>);
        impl crate::credentials::PasswordStore for MapStore {
            fn get(&self, k: &str) -> anyhow::Result<Option<String>> {
                Ok(self.0.lock().unwrap().get(k).cloned())
            }
            fn set(&self, k: &str, v: &str) -> anyhow::Result<()> {
                self.0.lock().unwrap().insert(k.into(), v.into());
                Ok(())
            }
            fn delete(&self, k: &str) -> anyhow::Result<()> {
                self.0.lock().unwrap().remove(k);
                Ok(())
            }
        }

        let store = test_store();
        // Identity with username + password, no key file.
        let id = store
            .create_identity(&crate::store::NewIdentity {
                name: "team".into(),
                username: Some("ops".into()),
                private_key: None,
                certificate: None,
                sort_order: 0,
                has_password: true,
            })
            .unwrap()
            .id;
        let mut nh = NewHost::launcher("h1", "10.0.0.1");
        nh.identity_id = Some(id);
        let host_id = store.create_host(&nh).unwrap().id;

        let pw = MapStore(Mutex::new(HashMap::new()));
        crate::credentials::PasswordStore::set(&pw, &crate::credentials::identity_key(id), "s3cret")
            .unwrap();

        let entry = HostEntry::Managed(store.get_host(host_id).unwrap().unwrap());
        let (secret, diag) = resolve_pending_secret(&entry, &pw);
        assert!(
            matches!(secret, Some(crate::session::PendingSecret::Password(ref p)) if p == "s3cret"),
            "keyless identity should yield a login password, got {secret:?} / {diag}"
        );
    }

    #[test]
    fn paste_in_normal_mode_is_ignored() {
        let mut app = test_app(vec![("web", host("web"))]);
        // A stray paste in Normal must not run commands or change mode.
        app.handle_paste("adq#/").unwrap();
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.host_form.is_none());
    }

    #[test]
    fn quit_asks_for_confirmation_by_default() {
        let mut app = test_app(vec![("web", host("web"))]);
        // 'q' opens the confirm dialog instead of quitting.
        app.handle_key(key_char('q')).unwrap();
        assert_eq!(app.mode, AppMode::ConfirmQuit);
        assert!(!app.should_quit);

        // 'n' cancels back to Normal.
        app.handle_key(key_char('n')).unwrap();
        assert_eq!(app.mode, AppMode::Normal);
        assert!(!app.should_quit);

        // 'q' then 'y' quits.
        app.handle_key(key_char('q')).unwrap();
        app.handle_key(key_char('y')).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_confirms_then_forces() {
        let mut app = test_app(vec![("web", host("web"))]);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        // First Ctrl+C asks.
        app.handle_key(ctrl_c).unwrap();
        assert_eq!(app.mode, AppMode::ConfirmQuit);
        assert!(!app.should_quit);
        // Second Ctrl+C forces quit.
        app.handle_key(ctrl_c).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn quit_confirmation_can_be_disabled() {
        let mut app = test_app(vec![("web", host("web"))]);
        app.config.appearance.confirm_quit = false;
        app.handle_key(key_char('q')).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn rebinding_add_host_action_takes_effect() {
        let mut app = test_app(vec![("web", host("web"))]);
        // Default: 'a' opens the new-host form.
        app.handle_key(key_char('a')).unwrap();
        assert_eq!(app.mode, AppMode::HostForm);
        app.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(app.mode, AppMode::Normal);

        // Rebind add-host to 'n'; now 'a' no longer opens the form, 'n' does.
        app.config.keybinds.set(KeyAction::AddHost, vec!["n".to_string()]);
        app.handle_key(key_char('a')).unwrap();
        assert_ne!(app.mode, AppMode::HostForm);
        // 'a' fell through to the palette (type-to-search).
        app.mode = AppMode::Normal;
        app.handle_key(key_char('n')).unwrap();
        assert_eq!(app.mode, AppMode::HostForm);
    }

    #[test]
    fn parse_keyspec_handles_common_forms() {
        assert_eq!(parse_keyspec("F2"), Some((KeyCode::F(2), KeyModifiers::empty())));
        assert_eq!(parse_keyspec("F10"), Some((KeyCode::F(10), KeyModifiers::empty())));
        assert_eq!(
            parse_keyspec("Ctrl+S"),
            Some((KeyCode::Char('s'), KeyModifiers::CONTROL))
        );
        assert_eq!(
            parse_keyspec("Alt+Enter"),
            Some((KeyCode::Enter, KeyModifiers::ALT))
        );
        assert_eq!(parse_keyspec(""), None);
        assert_eq!(parse_keyspec("Meta+X"), None);
    }

    #[test]
    fn is_save_key_respects_config() {
        let mut app = test_app(vec![("web", host("web"))]);
        // Defaults: F2 and Ctrl+S.
        assert!(app.is_save_key(&key(KeyCode::F(2))));
        assert!(app.is_save_key(&KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL
        )));
        assert!(!app.is_save_key(&key(KeyCode::F(4))));

        // Remap to Ctrl+Enter only.
        app.config.keybinds.save = vec!["Ctrl+Enter".to_string()];
        assert!(app.is_save_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)));
        assert!(!app.is_save_key(&key(KeyCode::F(2))));
    }

    #[test]
    fn base64_encode_known_vectors() {
        // Test the standard test vectors plus a few padding cases.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(
            base64_encode(b"Many hands make light work."),
            "TWFueSBoYW5kcyBtYWtlIGxpZ2h0IHdvcmsu"
        );
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn key_char(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
    }

    fn legacy_meta(entry: &mut HostEntry) -> &mut crate::metadata::HostMetadata {
        entry.legacy_mut().expect("legacy host").1
    }

    #[test]
    fn reload_hosts_builds_entries_with_metadata_defaults() {
        let app = test_app(vec![("alpha", host("alpha")), ("beta", host("beta"))]);
        assert_eq!(app.hosts.len(), 2);
        assert_eq!(app.filtered_indices, vec![0, 1]);
        assert_eq!(app.hosts[0].name(), "alpha");
        if let HostEntry::Legacy { meta, .. } = &app.hosts[0] {
            assert_eq!(meta.host_name, "alpha");
        }
    }

    #[test]
    fn slash_opens_palette_mode() {
        let mut app = test_app(vec![
            ("web-prod", host("web-prod")),
            ("db-staging", host("db-staging")),
        ]);
        legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
        legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
        app.rebuild_filter();

        app.handle_key(key_char('/')).unwrap();
        assert_eq!(app.mode, AppMode::Palette);
        assert_eq!(app.palette_results.len(), 2);

        app.handle_key(key_char('w')).unwrap();
        assert_eq!(app.palette_query, "w");
        assert_eq!(app.palette_results.len(), 1);
    }

    #[test]
    fn typing_char_opens_palette() {
        let mut app = test_app(vec![
            ("web-prod", host("web-prod")),
            ("db-staging", host("db-staging")),
        ]);
        legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
        legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
        app.rebuild_filter();

        // Typing a character in Normal mode opens the palette
        app.handle_key(key_char('w')).unwrap();
        assert_eq!(app.mode, AppMode::Palette);
        assert_eq!(app.palette_query, "w");
        assert_eq!(app.palette_results.len(), 1);
    }

    #[test]
    fn esc_exits_search_and_clears_query_and_tag_filter() {
        let mut app = test_app(vec![("alpha", host("alpha"))]);
        app.tag_filter = Some("prod".into());
        app.mode = AppMode::Search;
        app.search_query = "al".into();

        app.handle_key(key(KeyCode::Esc)).unwrap();

        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.search_query.is_empty());
        assert!(app.tag_filter.is_none());
    }

    #[test]
    fn navigation_wraps_around() {
        let mut app = test_app(vec![("a", host("a")), ("b", host("b"))]);
        assert_eq!(app.selected, 0);

        // Up from first wraps to last
        app.handle_key(key(KeyCode::Up)).unwrap();
        assert_eq!(app.selected, 1);

        // Down from last wraps to first
        app.handle_key(key(KeyCode::Down)).unwrap();
        assert_eq!(app.selected, 0);

        // Normal forward navigation
        app.handle_key(key(KeyCode::Down)).unwrap();
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn j_k_move_selection_in_search_mode() {
        let mut app = test_app(vec![("a", host("a")), ("b", host("b"))]);
        app.mode = AppMode::Search;

        app.handle_key(key_char('j')).unwrap();
        assert_eq!(app.selected, 1);

        app.handle_key(key_char('k')).unwrap();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn enter_starts_embedded_session() {
        // Pressing Enter no longer shells out to an external terminal; it
        // spawns a PTY in-process and flips into Connecting mode. We use
        // /bin/true as the program so the child exits immediately — the
        // session itself stays in App until Drop tears it down.
        let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
        let (launcher, _launched) = RecordingLauncher::new();
        let resolver = MockResolver::new(vec![("edge", host("edge"))]);
        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(resolver),
                metadata: Arc::clone(&metadata),
                store: test_store(),
                launcher: Box::new(launcher),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.reload_hosts().unwrap();
        // Pretend the terminal is 80x24 so the session has a sensible PTY size.
        app.terminal_area = ratatui::layout::Rect::new(0, 0, 80, 24);

        // Directly inject the session args we want (avoid spawning real ssh in
        // a unit test). This mirrors what connect_selected does after building
        // ssh_argv.
        let config = crate::session::SessionConfig {
            argv: vec!["true".into()],
            display_name: "edge".into(),
            meta: crate::session::SessionMeta::default(),
            pending_secret: None,
        };
        let session = crate::session::Session::spawn(config, 24, 80).unwrap();
        app.sessions.push(session);
        app.active_session = Some(0);
        app.mode = AppMode::Connecting;

        // Sanity: app has one tab.
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.mode, AppMode::Connecting);

        // Ctrl+D closes the last tab and returns to Normal.
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.sessions.is_empty());
        assert!(app.active_session.is_none());
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn ctrl_t_duplicates_and_ctrl_w_closes_tab() {
        // Three tabs, all running `true`. Verify Ctrl+T appends, Ctrl+PgUp/Dn
        // cycle, Ctrl+W removes the active tab.
        let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
        let (launcher, _launched) = RecordingLauncher::new();
        let resolver = MockResolver::new(vec![("edge", host("edge"))]);
        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(resolver),
                metadata,
                store: test_store(),
                launcher: Box::new(launcher),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.reload_hosts().unwrap();
        app.terminal_area = ratatui::layout::Rect::new(0, 0, 80, 24);

        let cfg = crate::session::SessionConfig {
            argv: vec!["true".into()],
            display_name: "edge".into(),
            meta: crate::session::SessionMeta::default(),
            pending_secret: None,
        };
        app.sessions
            .push(crate::session::Session::spawn(cfg, 24, 80).unwrap());
        app.active_session = Some(0);
        app.mode = AppMode::Connecting;

        // Ctrl+T: duplicate to a second tab.
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.sessions.len(), 2);
        assert_eq!(app.active_session, Some(1));

        // Ctrl+T again: third tab.
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.sessions.len(), 3);
        assert_eq!(app.active_session, Some(2));

        // Ctrl+PageUp: cycle backward to tab 1.
        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.active_session, Some(1));

        // Ctrl+PageDown: cycle forward to tab 2.
        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.active_session, Some(2));

        // Ctrl+W: close active (last tab); should stay at the new last.
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.sessions.len(), 2);
        assert_eq!(app.active_session, Some(1));

        // Ctrl+W twice more: empty + return to dashboard.
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.sessions.is_empty());
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn favourite_toggle_updates_metadata() {
        let mut app = test_app(vec![("web", host("web"))]);
        assert!(!app.hosts[0].favorite());

        app.handle_key(key_char('f')).unwrap();
        assert!(app.hosts[0].favorite());

        app.handle_key(key_char('f')).unwrap();
        assert!(!app.hosts[0].favorite());
    }

    #[test]
    fn e_enters_host_detail_with_edit_buffers() {
        let mut app = test_app(vec![("web", host("web"))]);
        legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
        legacy_meta(&mut app.hosts[0]).description = Some("Primary".into());
        legacy_meta(&mut app.hosts[0]).environment = Some("staging".into());

        // HostDetail is the fallback metadata editor (used when an ssh_config
        // alias can't be materialized). Drive it directly.
        app.enter_host_detail().unwrap();
        assert_eq!(app.mode, AppMode::HostDetail);
        let edit = app.detail_edit.as_ref().unwrap();
        assert_eq!(edit.tags, "prod");
        assert_eq!(edit.description, "Primary");
        assert_eq!(edit.environment, "staging");

        app.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.detail_edit.is_none());
    }

    #[test]
    fn host_detail_save_persists_metadata() {
        let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
        let resolver = MockResolver::new(vec![("web", host("web"))]);
        let (launcher, _launched) = RecordingLauncher::new();
        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(resolver),
                metadata: Arc::clone(&metadata),
                store: test_store(),
                launcher: Box::new(launcher),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.reload_hosts().unwrap();

        app.enter_host_detail().unwrap();
        app.handle_key(key_char('p')).unwrap();
        app.handle_key(key_char('r')).unwrap();
        app.handle_key(key_char('o')).unwrap();
        app.handle_key(key_char('d')).unwrap();
        app.handle_key(key(KeyCode::Tab)).unwrap();
        app.handle_key(key_char('n')).unwrap();
        app.handle_key(key_char('o')).unwrap();
        app.handle_key(key_char('t')).unwrap();
        app.handle_key(key_char('e')).unwrap();
        app.handle_key(key(KeyCode::Tab)).unwrap();
        app.handle_key(key_char('d')).unwrap();
        app.handle_key(key_char('e')).unwrap();
        app.handle_key(key_char('v')).unwrap();
        app.handle_key(key(KeyCode::Enter)).unwrap();

        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.hosts[0].tags(), &["prod".to_string()]);
        assert_eq!(app.hosts[0].description(), Some("note"));
        assert_eq!(app.hosts[0].environment(), Some("dev"));

        let stored = metadata.get("web").unwrap().unwrap();
        assert_eq!(stored.tags, vec!["prod".to_string()]);
        assert_eq!(stored.description.as_deref(), Some("note"));
        assert_eq!(stored.environment.as_deref(), Some("dev"));
    }

    #[test]
    fn host_detail_esc_discards_unsaved_edits() {
        let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
        let resolver = MockResolver::new(vec![("web", host("web"))]);
        let (launcher, _launched) = RecordingLauncher::new();
        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(resolver),
                metadata: Arc::clone(&metadata),
                store: test_store(),
                launcher: Box::new(launcher),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.reload_hosts().unwrap();
        legacy_meta(&mut app.hosts[0]).description = Some("saved".into());
        metadata.upsert(legacy_meta(&mut app.hosts[0])).unwrap();

        app.enter_host_detail().unwrap();
        app.handle_key(key_char('x')).unwrap();
        app.handle_key(key(KeyCode::Esc)).unwrap();

        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.hosts[0].description(), Some("saved"));
    }

    #[test]
    fn favourite_toggle_works_in_host_detail() {
        let mut app = test_app(vec![("web", host("web"))]);
        app.enter_host_detail().unwrap();
        app.handle_key(key_char('f')).unwrap();
        assert!(app.hosts[0].favorite());
    }

    #[test]
    fn parse_tags_splits_and_trims() {
        assert_eq!(
            parse_tags(" prod , db , , staging "),
            vec!["prod", "db", "staging"]
        );
    }

    #[test]
    fn tab_toggles_detail_focus() {
        let mut app = test_app(vec![("web", host("web"))]);
        assert!(!app.detail_focus);
        app.handle_key(key(KeyCode::Tab)).unwrap();
        assert!(app.detail_focus);
        app.handle_key(key(KeyCode::Tab)).unwrap();
        assert!(!app.detail_focus);
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        // With confirmation disabled, q and Ctrl+C quit immediately.
        let mut app = test_app(vec![("web", host("web"))]);
        app.config.appearance.confirm_quit = false;

        app.handle_key(key_char('q')).unwrap();
        assert!(app.should_quit);

        app.should_quit = false;
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn keychain_create_edit_delete_flow() {
        let store = test_store();
        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(MockResolver::new(vec![])),
                metadata: Arc::new(MetadataDb::default()),
                store: Arc::clone(&store),
                launcher: Box::new(RecordingLauncher::new().0),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.active_tab = 2;
        app.reload_identities().unwrap();
        app.handle_key(key_char('a')).unwrap();

        // Single-step model: type straight into the active field, ↓ advances.
        for c in "work-laptop".chars() {
            app.handle_key(key_char(c)).unwrap();
        }
        app.handle_key(key(KeyCode::Down)).unwrap(); // → Username
        for c in "deploy".chars() {
            app.handle_key(key_char(c)).unwrap();
        }
        app.handle_key(key(KeyCode::Down)).unwrap(); // → PrivateKey
        for c in "~/.ssh/id_ed25519".chars() {
            app.handle_key(key_char(c)).unwrap();
        }
        // F2 to save
        app.handle_key(key(KeyCode::F(2))).unwrap();

        assert_eq!(app.mode, AppMode::Normal);
        let created = store
            .get_identity_by_name("work-laptop")
            .unwrap()
            .expect("created in store");
        assert_eq!(created.username.as_deref(), Some("deploy"));
    }

    #[test]
    fn tag_filter_narrows_candidates_before_search() {
        let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
        legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
        legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
        app.tag_filter = Some("prod".into());
        app.rebuild_filter();

        assert_eq!(app.filtered_indices, vec![0]);
    }

    #[test]
    fn hash_enters_tag_filter_and_enter_applies() {
        let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
        legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
        legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
        app.rebuild_filter();

        app.handle_key(key_char('#')).unwrap();
        assert_eq!(app.mode, AppMode::TagFilter);

        app.handle_key(key_char('p')).unwrap();
        app.handle_key(key_char('r')).unwrap();
        app.handle_key(key_char('o')).unwrap();
        app.handle_key(key_char('d')).unwrap();
        app.handle_key(key(KeyCode::Enter)).unwrap();

        // Enter applies the filter and returns to Normal so the list can be
        // navigated while filtered.
        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.tag_filter.as_deref(), Some("prod"));
        assert_eq!(app.filtered_indices, vec![0]);

        // Esc in Normal clears the active tag filter.
        app.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.tag_filter.is_none());
        assert_eq!(app.filtered_indices.len(), 2);
    }

    #[test]
    fn sort_mode_label_orders_by_display_name() {
        let store = test_store();
        let default_id = store.get_identity_by_name("Default").unwrap().unwrap().id;
        store
            .create_host(&NewHost {
                name: "z-host".into(),
                label: Some("Zulu".into()),
                address: "10.0.0.1".into(),
                port: 22,
                group_id: None,
                identity_id: Some(default_id),
                tags: vec![],
                notes: None,
                ..Default::default()
            })
            .unwrap();
        store
            .create_host(&NewHost {
                name: "a-host".into(),
                label: Some("Alpha".into()),
                address: "10.0.0.2".into(),
                port: 22,
                group_id: None,
                identity_id: Some(default_id),
                tags: vec![],
                notes: None,
                ..Default::default()
            })
            .unwrap();

        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(MockResolver::new(vec![])),
                metadata: Arc::new(MetadataDb::default()),
                store,
                launcher: Box::new(RecordingLauncher::new().0),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.reload_hosts().unwrap();
        assert_eq!(
            app.filtered_indices
                .iter()
                .map(|&i| app.hosts[i].name().to_string())
                .collect::<Vec<_>>(),
            vec!["a-host", "z-host"]
        );
    }

    #[test]
    fn reload_hosts_skips_unresolved_and_preserves_selection() {
        struct PartialResolver {
            order: Vec<String>,
        }

        impl HostResolver for PartialResolver {
            fn list_hosts(&self) -> Result<Vec<String>> {
                Ok(self.order.clone())
            }

            fn resolve_host(&self, name: &str) -> Result<SshHost> {
                if name == "bad" {
                    anyhow::bail!("simulated resolve failure");
                }
                Ok(host(name))
            }
        }

        let metadata: Arc<dyn MetadataStore> = Arc::new(MetadataDb::default());
        let (launcher, _launched) = RecordingLauncher::new();
        let mut app = App::new_with_deps(
            AppConfig::default(),
            AppDeps {
                resolver: Box::new(PartialResolver {
                    order: vec!["good".into(), "bad".into(), "also".into()],
                }),
                metadata,
                store: test_store(),
                launcher: Box::new(launcher),
                password_store: Box::new(crate::credentials::NoopPasswordStore),
            },
        );
        app.reload_hosts().unwrap();
        assert_eq!(app.hosts.len(), 2);
        assert!(app.hosts.iter().all(|e| e.name() != "bad"));

        app.handle_key(key(KeyCode::Down)).unwrap();
        assert_eq!(app.selected_entry().unwrap().name(), "good");

        app.reload_hosts().unwrap();
        assert_eq!(app.selected_entry().unwrap().name(), "good");
    }

    #[test]
    fn host_form_up_down_navigate_fields_in_both_directions() {
        let mut app = test_app(vec![]);
        app.enter_host_form(None, false).unwrap();
        assert_eq!(
            app.host_form.as_ref().unwrap().field,
            HostFormField::Address
        );

        app.handle_key(key(KeyCode::Down)).unwrap();
        assert_eq!(
            app.host_form.as_ref().unwrap().field,
            HostFormField::Password
        );

        app.handle_key(key(KeyCode::Down)).unwrap();
        assert_eq!(
            app.host_form.as_ref().unwrap().field,
            HostFormField::Username
        );

        app.handle_key(key(KeyCode::Down)).unwrap();
        assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Label);

        app.handle_key(key(KeyCode::Up)).unwrap();
        assert_eq!(
            app.host_form.as_ref().unwrap().field,
            HostFormField::Username
        );

        app.handle_key(key(KeyCode::Up)).unwrap();
        assert_eq!(
            app.host_form.as_ref().unwrap().field,
            HostFormField::Password
        );

        app.handle_key(key(KeyCode::Up)).unwrap();
        assert_eq!(
            app.host_form.as_ref().unwrap().field,
            HostFormField::Address
        );

        // Navigate to the end (12 downs from Address)
        for _ in 0..12 {
            app.handle_key(key(KeyCode::Down)).unwrap();
        }
        assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::OsIcon);

        app.handle_key(key(KeyCode::Up)).unwrap();
        assert_eq!(
            app.host_form.as_ref().unwrap().field,
            HostFormField::RemoteCommand
        );
    }

    #[test]
    fn host_form_picker_at_boundary_moves_to_adjacent_field() {
        let mut app = test_app(vec![]);
        app.enter_host_form(None, false).unwrap();
        for _ in 0..6 {
            app.handle_key(key(KeyCode::Down)).unwrap();
        }
        assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Group);

        app.handle_key(key(KeyCode::Up)).unwrap();
        assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Port);

        app.handle_key(key(KeyCode::Down)).unwrap();
        assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Group);

        app.handle_key(key(KeyCode::Down)).unwrap();
        assert_eq!(
            app.host_form.as_ref().unwrap().field,
            HostFormField::Identity
        );
    }
}
