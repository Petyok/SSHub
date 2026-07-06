mod audit;
mod connect;
mod groups;
mod host_form;
mod hostlist;
mod identities;
mod import;
mod keys;
mod mouse;
mod session;
mod tags;
mod tunnels;

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

/// Base host-name column width (chars) at zoom level 0.
pub const NAME_WIDTH_BASE: usize = 14;
/// Extra name-column width added per zoom level.
pub const NAME_WIDTH_STEP: usize = 8;
/// Maximum UI zoom level.
pub const UI_ZOOM_MAX: usize = 3;

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
    /// Dedicated popup for choosing a group's default identity (`e` on a group).
    GroupIdentityPicker,
    /// Searchable dropdown for choosing the tunnel form's SSH server.
    TunnelHostPicker,
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

    pub(crate) fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub(crate) fn prev(self) -> Self {
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

    pub(crate) fn next(self) -> Self {
        let idx = self as usize;
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub(crate) fn prev(self) -> Self {
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

    pub(crate) fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub(crate) fn prev(self) -> Self {
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

    pub(crate) fn is_picker(self) -> bool {
        matches!(
            self,
            HostFormField::Group | HostFormField::Identity | HostFormField::OsIcon
        )
    }

    pub(crate) fn is_toggle(self) -> bool {
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

/// Dedicated popup to pick a group's default identity ([`AppMode::GroupIdentityPicker`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupIdentityPicker {
    pub group_id: i64,
    pub group_name: String,
    /// Selected row: `0` = "(none)", `1..` = index into `App::identities` + 1.
    pub selected: usize,
}

/// Searchable dropdown for choosing the tunnel form's SSH server
/// ([`AppMode::TunnelHostPicker`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TunnelHostPicker {
    /// Case-insensitive substring filter typed by the user.
    pub query: String,
    /// Index into the current filtered match list.
    pub selected: usize,
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

    pub(crate) fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub(crate) fn prev(self) -> Self {
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
    pub group_identity_picker: Option<GroupIdentityPicker>,
    /// Searchable SSH-server picker for the tunnel form.
    pub tunnel_host_picker: Option<TunnelHostPicker>,
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
            group_identity_picker: None,
            tunnel_host_picker: None,
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

    pub(crate) fn active_field_mut(&mut self) -> &mut String {
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

    pub(crate) fn active_field_mut(&mut self) -> &mut String {
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
    pub(crate) fn clear_pasted_key_marker(&mut self) {
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

    pub(crate) fn active_field_mut(&mut self) -> &mut String {
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

    pub(crate) fn test_store() -> Arc<LauncherStore> {
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

    pub(crate) fn test_app(hosts: Vec<(&str, SshHost)>) -> App {
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

    pub(crate) fn host(name: &str) -> SshHost {
        let mut h = SshHost::new(name);
        h.hostname = Some(format!("{name}.example.com"));
        h
    }

    #[test]
    pub(crate) fn keyevent_to_spec_roundtrips() {
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
    pub(crate) fn keybind_editor_captures_and_persists() {
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
    pub(crate) fn multiline_paste_into_form_stays_in_field() {
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
    pub(crate) fn pasted_key_material_is_written_to_a_file_on_save() {
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
    pub(crate) fn backspace_discards_pasted_key_blob() {
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
    pub(crate) fn identity_grid_navigation_moves_by_row_and_column() {
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
    pub(crate) fn keyless_identity_secret_is_a_login_password() {
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
    pub(crate) fn paste_in_normal_mode_is_ignored() {
        let mut app = test_app(vec![("web", host("web"))]);
        // A stray paste in Normal must not run commands or change mode.
        app.handle_paste("adq#/").unwrap();
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.host_form.is_none());
    }

    #[test]
    pub(crate) fn quit_asks_for_confirmation_by_default() {
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
    pub(crate) fn ctrl_c_confirms_then_forces() {
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
    pub(crate) fn quit_confirmation_can_be_disabled() {
        let mut app = test_app(vec![("web", host("web"))]);
        app.config.appearance.confirm_quit = false;
        app.handle_key(key_char('q')).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    pub(crate) fn rebinding_add_host_action_takes_effect() {
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
    pub(crate) fn parse_keyspec_handles_common_forms() {
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
    pub(crate) fn is_save_key_respects_config() {
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
    pub(crate) fn base64_encode_known_vectors() {
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

    pub(crate) fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    pub(crate) fn key_char(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
    }

    pub(crate) fn legacy_meta(entry: &mut HostEntry) -> &mut crate::metadata::HostMetadata {
        entry.legacy_mut().expect("legacy host").1
    }

    #[test]
    pub(crate) fn reload_hosts_builds_entries_with_metadata_defaults() {
        let app = test_app(vec![("alpha", host("alpha")), ("beta", host("beta"))]);
        assert_eq!(app.hosts.len(), 2);
        assert_eq!(app.filtered_indices, vec![0, 1]);
        assert_eq!(app.hosts[0].name(), "alpha");
        if let HostEntry::Legacy { meta, .. } = &app.hosts[0] {
            assert_eq!(meta.host_name, "alpha");
        }
    }

    #[test]
    pub(crate) fn slash_opens_palette_mode() {
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
    pub(crate) fn typing_char_opens_palette() {
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
    pub(crate) fn esc_exits_search_and_clears_query_and_tag_filter() {
        let mut app = test_app(vec![("alpha", host("alpha"))]);
        app.tag_filters = vec!["prod".into()];
        app.mode = AppMode::Search;
        app.search_query = "al".into();

        app.handle_key(key(KeyCode::Esc)).unwrap();

        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.search_query.is_empty());
        assert!(app.tag_filters.is_empty());
    }

    #[test]
    pub(crate) fn navigation_wraps_around() {
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
    pub(crate) fn j_k_move_selection_in_search_mode() {
        let mut app = test_app(vec![("a", host("a")), ("b", host("b"))]);
        app.mode = AppMode::Search;

        app.handle_key(key_char('j')).unwrap();
        assert_eq!(app.selected, 1);

        app.handle_key(key_char('k')).unwrap();
        assert_eq!(app.selected, 0);
    }

    #[test]
    pub(crate) fn enter_starts_embedded_session() {
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
    pub(crate) fn ctrl_t_duplicates_and_ctrl_w_closes_tab() {
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
    pub(crate) fn favourite_toggle_updates_metadata() {
        let mut app = test_app(vec![("web", host("web"))]);
        assert!(!app.hosts[0].favorite());

        app.handle_key(key_char('f')).unwrap();
        assert!(app.hosts[0].favorite());

        app.handle_key(key_char('f')).unwrap();
        assert!(!app.hosts[0].favorite());
    }

    #[test]
    pub(crate) fn e_enters_host_detail_with_edit_buffers() {
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
    pub(crate) fn host_detail_save_persists_metadata() {
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
    pub(crate) fn host_detail_esc_discards_unsaved_edits() {
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
    pub(crate) fn favourite_toggle_works_in_host_detail() {
        let mut app = test_app(vec![("web", host("web"))]);
        app.enter_host_detail().unwrap();
        app.handle_key(key_char('f')).unwrap();
        assert!(app.hosts[0].favorite());
    }

    #[test]
    pub(crate) fn parse_tags_splits_and_trims() {
        assert_eq!(
            parse_tags(" prod , db , , staging "),
            vec!["prod", "db", "staging"]
        );
    }

    #[test]
    pub(crate) fn tab_toggles_detail_focus() {
        let mut app = test_app(vec![("web", host("web"))]);
        assert!(!app.detail_focus);
        app.handle_key(key(KeyCode::Tab)).unwrap();
        assert!(app.detail_focus);
        app.handle_key(key(KeyCode::Tab)).unwrap();
        assert!(!app.detail_focus);
    }

    #[test]
    pub(crate) fn q_and_ctrl_c_quit() {
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
    pub(crate) fn keychain_create_edit_delete_flow() {
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
    pub(crate) fn tag_filter_narrows_candidates_before_search() {
        let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
        legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
        legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
        app.tag_filters = vec!["prod".into()];
        app.rebuild_filter();

        assert_eq!(app.filtered_indices, vec![0]);
    }

    #[test]
    pub(crate) fn tag_filter_picker_arrow_selects_and_applies_tag() {
        let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
        legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
        legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
        app.rebuild_filter();

        app.handle_key(key_char('#')).unwrap();
        // Rows are ["(all)", "prod", "staging"]; row 0 selected by default.
        assert_eq!(app.tag_filter_rows(), vec!["(all)", "prod", "staging"]);
        assert_eq!(app.tag_filter_selected, 0);

        // Arrow down twice lands on "staging" and Enter toggles + applies it.
        app.handle_key(key(KeyCode::Down)).unwrap();
        app.handle_key(key(KeyCode::Down)).unwrap();
        app.handle_key(key(KeyCode::Enter)).unwrap();

        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.tag_filters, vec!["staging".to_string()]);
        assert_eq!(app.filtered_indices, vec![1]);
    }

    #[test]
    pub(crate) fn tag_filter_picker_space_toggles_multiple_tags_and_ands_them() {
        let mut app = test_app(vec![
            ("web", host("web")),
            ("db", host("db")),
            ("both", host("both")),
        ]);
        legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
        legacy_meta(&mut app.hosts[1]).tags = vec!["eu".into()];
        legacy_meta(&mut app.hosts[2]).tags = vec!["prod".into(), "eu".into()];
        app.rebuild_filter();

        app.handle_key(key_char('#')).unwrap();
        // Rows: ["(all)", "eu", "prod"]. Space toggles a tag and stays open.
        app.handle_key(key(KeyCode::Down)).unwrap(); // → "eu"
        app.handle_key(key_char(' ')).unwrap();
        assert_eq!(app.mode, AppMode::TagFilter, "stays open after Space");
        assert_eq!(app.tag_filters, vec!["eu".to_string()]);

        app.handle_key(key(KeyCode::Down)).unwrap(); // → "prod"
        app.handle_key(key_char(' ')).unwrap();
        assert_eq!(app.tag_filters, vec!["eu".to_string(), "prod".to_string()]);

        // AND semantics: only the host carrying both tags survives.
        app.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.filtered_indices, vec![2]);
    }

    #[test]
    pub(crate) fn tag_filter_picker_enter_after_multiselect_keeps_all_tags() {
        // Regression: Enter must confirm the built-up set, never remove the
        // last-highlighted tag.
        let mut app = test_app(vec![
            ("web", host("web")),
            ("db", host("db")),
            ("both", host("both")),
        ]);
        legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
        legacy_meta(&mut app.hosts[1]).tags = vec!["eu".into()];
        legacy_meta(&mut app.hosts[2]).tags = vec!["prod".into(), "eu".into()];
        app.rebuild_filter();

        app.handle_key(key_char('#')).unwrap();
        app.handle_key(key(KeyCode::Down)).unwrap(); // → "eu"
        app.handle_key(key_char(' ')).unwrap(); // toggle eu on
        app.handle_key(key(KeyCode::Down)).unwrap(); // → "prod"
        app.handle_key(key_char(' ')).unwrap(); // toggle prod on
        // Cursor still on "prod" (active). Enter must NOT toggle it off.
        app.handle_key(key(KeyCode::Enter)).unwrap();

        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.tag_filters, vec!["eu".to_string(), "prod".to_string()]);
        assert_eq!(app.filtered_indices, vec![2]);
    }

    #[test]
    pub(crate) fn tag_filter_picker_space_toggles_tag_off() {
        let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
        legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
        legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
        app.tag_filters = vec!["prod".into()];
        app.rebuild_filter();

        app.handle_key(key_char('#')).unwrap();
        app.handle_key(key(KeyCode::Down)).unwrap(); // → "prod" (already active)
        app.handle_key(key_char(' ')).unwrap(); // toggle off
        assert!(app.tag_filters.is_empty());
        assert_eq!(app.filtered_indices.len(), 2);
    }

    #[test]
    pub(crate) fn tag_filter_picker_all_row_clears_filter() {
        let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
        legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
        legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
        app.tag_filters = vec!["prod".into()];
        app.rebuild_filter();

        app.handle_key(key_char('#')).unwrap();
        // Cursor opens on the "(all)" row.
        assert_eq!(app.tag_filter_selected, 0);

        // Enter on "(all)" clears every active filter and closes.
        app.handle_key(key(KeyCode::Enter)).unwrap();

        assert!(app.tag_filters.is_empty());
        assert_eq!(app.filtered_indices.len(), 2);
    }

    #[test]
    pub(crate) fn tag_filter_picker_esc_keeps_active_filter() {
        let mut app = test_app(vec![("web", host("web")), ("db", host("db"))]);
        legacy_meta(&mut app.hosts[0]).tags = vec!["prod".into()];
        legacy_meta(&mut app.hosts[1]).tags = vec!["staging".into()];
        app.tag_filters = vec!["prod".into()];
        app.rebuild_filter();

        app.handle_key(key_char('#')).unwrap();
        // Esc closes the picker without touching the active filter.
        app.handle_key(key(KeyCode::Esc)).unwrap();

        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.tag_filters, vec!["prod".to_string()]);
        assert_eq!(app.filtered_indices, vec![0]);
    }

    #[test]
    pub(crate) fn hash_enters_tag_filter_and_enter_applies() {
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

        // Enter toggles the highlighted match, applies it and returns to Normal
        // so the list can be navigated while filtered.
        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.tag_filters, vec!["prod".to_string()]);
        assert_eq!(app.filtered_indices, vec![0]);

        // Esc in Normal clears the active tag filter.
        app.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.tag_filters.is_empty());
        assert_eq!(app.filtered_indices.len(), 2);
    }

    #[test]
    pub(crate) fn sort_mode_label_orders_by_display_name() {
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
    pub(crate) fn reload_hosts_skips_unresolved_and_preserves_selection() {
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
    pub(crate) fn host_form_up_down_navigate_fields_in_both_directions() {
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
    pub(crate) fn host_form_picker_at_boundary_moves_to_adjacent_field() {
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
