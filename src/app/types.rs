use super::*;

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

    /// Parse CLI `--sort` values (not TUI display labels).
    pub fn from_cli_str(s: &str) -> Option<Self> {
        match s {
            "label" => Some(Self::Label),
            "last-connected" => Some(Self::LastConnected),
            "favorite" => Some(Self::FavoriteFirst),
            "group" => Some(Self::GroupThenLabel),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

/// An in-progress text selection over a zoomed dashboard panel (issue #18):
/// terminal-cell coordinates of the drag anchor and the current pointer.
#[derive(Debug, Clone, Copy)]
pub struct PanelSel {
    pub anchor: (u16, u16),
    pub cur: (u16, u16),
}

/// Direction for dashboard panel focus movement (issue #18).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDir {
    Left,
    Right,
    Up,
    Down,
}

/// A focusable panel on the hosts dashboard. Focus moves spatially with
/// `Alt+arrows`; `z` zooms the focused panel to the full dashboard body
/// (issue #18). The bento grid is: a left column (`Hosts`, one tall panel),
/// a middle stack (`Detail` / `Agent` / `Latency`), a right stack (`Recent` /
/// `Auth` / `Ping`), and a `SshLog` strip spanning mid+right along the bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelId {
    #[default]
    Hosts,
    Detail,
    Agent,
    Latency,
    Recent,
    Auth,
    Ping,
    SshLog,
    /// Live broadcast run panel, docked bottom-right (issue #3). Only drawn +
    /// focusable while `app.broadcast.is_some()`; the `focus_panel` guard in
    /// `keys.rs` suppresses `neighbor()` hops to it when the run is absent.
    Broadcast,
}

impl PanelId {
    pub fn label(self) -> &'static str {
        match self {
            PanelId::Hosts => "hosts",
            PanelId::Detail => "host detail",
            PanelId::Agent => "agent",
            PanelId::Latency => "latency",
            PanelId::Recent => "recent sessions",
            PanelId::Auth => "auth events",
            PanelId::Ping => "ping",
            PanelId::SshLog => "ssh log",
            PanelId::Broadcast => "broadcast",
        }
    }

    /// The neighboring panel in `dir`, or `None` to keep focus put (e.g. moving
    /// off an edge). Hand-written adjacency over the bento grid.
    pub fn neighbor(self, dir: FocusDir) -> Option<PanelId> {
        use FocusDir::*;
        use PanelId::*;
        match (self, dir) {
            // Left column (one tall panel).
            (Hosts, Right) => Some(Detail),
            (Hosts, _) => None,
            // Middle stack.
            (Detail, Left) => Some(Hosts),
            (Detail, Right) => Some(Recent),
            (Detail, Down) => Some(Agent),
            (Detail, Up) => None,
            (Agent, Left) => Some(Hosts),
            (Agent, Right) => Some(Auth),
            (Agent, Up) => Some(Detail),
            (Agent, Down) => Some(Latency),
            (Latency, Left) => Some(Hosts),
            (Latency, Right) => Some(Ping),
            (Latency, Up) => Some(Agent),
            (Latency, Down) => Some(SshLog),
            // Right stack.
            (Recent, Left) => Some(Detail),
            (Recent, Down) => Some(Auth),
            (Recent, _) => None,
            (Auth, Left) => Some(Agent),
            (Auth, Up) => Some(Recent),
            (Auth, Down) => Some(Ping),
            (Auth, Right) => None,
            (Ping, Left) => Some(Latency),
            (Ping, Up) => Some(Auth),
            (Ping, Down) => Some(SshLog),
            (Ping, Right) => Some(Broadcast),
            // Bottom strip (spans mid+right).
            (SshLog, Up) => Some(Latency),
            (SshLog, Left) => Some(Hosts),
            (SshLog, Right) => Some(Broadcast),
            (SshLog, _) => None,
            // Broadcast docked panel (bottom-right); only live when
            // app.broadcast.is_some() — the orchestrator's focus_panel guard
            // suppresses these when it's absent.
            (Broadcast, Left) => Some(SshLog),
            (Broadcast, Up) => Some(Ping),
            (Broadcast, _) => None,
        }
    }
}

#[cfg(test)]
mod panel_id_tests {
    use super::{FocusDir, PanelId};

    #[test]
    fn neighbor_moves_across_the_bento_grid() {
        // Columns: hosts ⇄ mid stack ⇄ right stack.
        assert_eq!(
            PanelId::Hosts.neighbor(FocusDir::Right),
            Some(PanelId::Detail)
        );
        assert_eq!(
            PanelId::Detail.neighbor(FocusDir::Left),
            Some(PanelId::Hosts)
        );
        assert_eq!(
            PanelId::Detail.neighbor(FocusDir::Right),
            Some(PanelId::Recent)
        );
        // Vertical within a stack, down into the shared ssh-log strip.
        assert_eq!(
            PanelId::Detail.neighbor(FocusDir::Down),
            Some(PanelId::Agent)
        );
        assert_eq!(
            PanelId::Latency.neighbor(FocusDir::Down),
            Some(PanelId::SshLog)
        );
        assert_eq!(
            PanelId::Ping.neighbor(FocusDir::Down),
            Some(PanelId::SshLog)
        );
        assert_eq!(
            PanelId::SshLog.neighbor(FocusDir::Up),
            Some(PanelId::Latency)
        );
        // Broadcast docks bottom-right: reachable from the ssh-log strip and
        // the ping panel, and hops back left/up into the grid.
        assert_eq!(
            PanelId::SshLog.neighbor(FocusDir::Right),
            Some(PanelId::Broadcast)
        );
        assert_eq!(
            PanelId::Ping.neighbor(FocusDir::Right),
            Some(PanelId::Broadcast)
        );
        assert_eq!(
            PanelId::Broadcast.neighbor(FocusDir::Left),
            Some(PanelId::SshLog)
        );
        assert_eq!(
            PanelId::Broadcast.neighbor(FocusDir::Up),
            Some(PanelId::Ping)
        );
        assert_eq!(PanelId::Broadcast.neighbor(FocusDir::Right), None);
    }

    #[test]
    fn neighbor_returns_none_at_edges() {
        assert_eq!(PanelId::Hosts.neighbor(FocusDir::Left), None);
        assert_eq!(PanelId::Hosts.neighbor(FocusDir::Up), None);
        assert_eq!(PanelId::Detail.neighbor(FocusDir::Up), None);
        assert_eq!(PanelId::Recent.neighbor(FocusDir::Right), None);
        assert_eq!(PanelId::SshLog.neighbor(FocusDir::Down), None);
    }

    #[test]
    fn default_focus_is_hosts() {
        assert_eq!(PanelId::default(), PanelId::Hosts);
    }
}

/// One section in the group tree (real group or virtual ungrouped bucket).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostGroupSection {
    pub group: Option<HostGroup>,
    pub label: String,
    pub host_indices: Vec<usize>,
    /// Whether this section is collapsed (its hosts and descendant sections
    /// are hidden).
    pub collapsed: bool,
    /// Nesting depth: 0 = top-level group (and the ungrouped bucket).
    pub depth: usize,
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
        /// Nesting depth for indentation (0 = top level).
        depth: usize,
    },
    Host {
        host_idx: usize,
        selected: bool,
        /// Indentation depth = the owning section's depth + 1.
        depth: usize,
    },
}

/// Appearance toggles shown in the Settings overlay, in display order. The
/// index maps to [`App::setting_value`] / `toggle_setting`. Each entry is
/// `(label, hint)`.
///
/// Hints must fit the 56-wide popup without ellipsizing (enforced by a test
/// in `tui::screens::settings`) and avoid ambiguous-width chars like the em
/// dash — some terminals draw those 2 cells wide, pushing the tail of the
/// line onto the popup border.
pub const SETTINGS_ITEMS: [(&str, &str); 5] = [
    (
        "Opaque background",
        "fixes unreadable text on transparent terminals",
    ),
    ("Show OS logos", "distro logo in the host card"),
    ("Confirm before quit", "ask before q / Ctrl+C"),
    (
        "Disable startup animation",
        "skip the intro splash (applies next launch)",
    ),
    (
        "Session logging",
        "save PTY output under ~/.local/share/sshub/logs",
    ),
];

/// Global keep-alive reconnect knobs (Tunnels tab, `R`). Row index maps to
/// [`crate::app::App::tunnel_reconnect_field_display`].
pub const TUNNEL_RECONNECT_FIELDS: [(&str, &str); 5] = [
    ("Max attempts", "0 = unlimited retries"),
    ("Initial delay", "first retry wait (seconds)"),
    ("Max delay", "backoff cap (seconds)"),
    ("Stable time", "uptime before a spawn counts as up"),
    ("Jitter", "random spread around each delay"),
];

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
    /// Dropdown over the group form's Parent / Identity field.
    GroupFieldPicker,
    /// Searchable dropdown for choosing the tunnel form's SSH server.
    TunnelHostPicker,
    /// Searchable dropdown for opening a new embedded SSH session tab.
    SessionHostPicker,
    /// Dropdown over the host form's Group/Identity field.
    FieldPicker,
    /// Keybinding editor overlay.
    KeybindEditor,
    /// Settings overlay: checkbox list of appearance toggles.
    Settings,
    /// Keep-alive reconnect backoff settings (Tunnels tab).
    TunnelReconnectSettings,
    /// Quit confirmation dialog.
    ConfirmQuit,
    TunnelForm,
    ConfirmDelete,
    ConfirmDiscard,
    Help,
    Palette,
    ImportPrompt,
    /// Single-field text prompt for an SFTP mkdir / rename.
    SftpPrompt,
    /// Embedded session is spawning; ConnectScreen visible.
    Connecting,
    /// Live embedded SSH session; PTY drives the fullscreen view.
    Session,
    /// Broadcast wizard stage 1: pick a target (group / tag menu).
    BroadcastPickTarget,
    /// Broadcast wizard stage 2: single-line command input.
    BroadcastCommand,
    /// Broadcast wizard stage 3: target preview + [y]/[e]/[N] barrier.
    BroadcastPreview,
}

/// Live background-run state; App holds `broadcast: Option<BroadcastState>`.
///
/// No derive attribute at all — not even `Debug` — because
/// `std::sync::mpsc::Receiver` is not `Debug`, and this type is deliberately
/// neither `Clone` nor `Copy` (it owns the run's channel + cancel flag).
pub struct BroadcastState {
    pub target_label: String, // "#prod" / "group: production"
    pub command: String,
    pub results: Vec<crate::broadcast::HostResult>,
    pub rx: std::sync::mpsc::Receiver<crate::broadcast::BroadcastEvent>,
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub concurrency: usize,
    pub phase: BroadcastPhase,
    pub anim: Option<crate::tui::tween::SlideAnim>, // entry slide; None once settled
    pub audit_written: bool, // guard: log_auth_event fires once at completion
}

/// An in-flight tab-switch slide (#35): the body wipes between the `from` and
/// `to` tabs. Direction is `to > from` (new slides in from the right) vs
/// `to < from` (current slides out to the right, revealing the left tab).
#[derive(Debug, Clone, Copy)]
pub struct TabSwitch {
    pub from: usize,
    pub to: usize,
    pub at: std::time::Instant,
}

/// A transient error popup (issue #3): one failed host's error text, slides in
/// from the right above the broadcast panel and auto-expires. Geometry + slide
/// progress are derived from `born` at render time (no stored anim state).
#[derive(Debug, Clone)]
pub struct BroadcastToast {
    pub host: String,
    pub text: String,
    pub born: std::time::Instant,
}

/// Lifecycle phase of a live broadcast run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BroadcastPhase {
    Running,
    Settling { done_at: std::time::Instant }, // countdown armed
    Paused,                                   // focused/zoomed after completion
    Leaving,                                  // exit slide playing, remove when done
}

/// A pickable broadcast target (menu row).
#[derive(Debug, Clone)]
pub enum BroadcastTarget {
    Group { id: i64, label: String },
    Tag { name: String },
}

/// One resolved target host in the preview (managed hosts only; entries with no
/// managed id are excluded upstream).
#[derive(Debug, Clone)]
pub struct BroadcastCandidate {
    pub host_id: i64,
    pub host_name: String,
    pub argv: Vec<String>,
    /// Stored credential for this host (phase 2), resolved when the target is
    /// picked; threaded into the run so password hosts authenticate via
    /// SSH_ASKPASS. `None` => key/agent only.
    pub secret: Option<crate::session::PendingSecret>,
    pub selected: bool, // toggled in edit-targets
}

/// Pre-run wizard state; App holds `broadcast_setup: Option<BroadcastSetup>`.
/// The active AppMode variant (PickTarget/Command/Preview) names the stage.
///
/// No derive attribute at all — deliberately neither `Clone` nor `Copy`.
pub struct BroadcastSetup {
    pub options: Vec<BroadcastTarget>,
    pub menu_selected: usize,
    pub target_label: String, // filled once a target is chosen
    pub command: String,
    pub cursor: usize,
    pub candidates: Vec<BroadcastCandidate>, // resolved on target pick
    pub preview_selected: usize,             // highlighted row in edit-targets
    pub edit_targets: bool,                  // preview [e] entered per-host deselect
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
    AutoConnect,
}

impl TunnelFormField {
    const ALL: [TunnelFormField; 7] = [
        TunnelFormField::Host,
        TunnelFormField::Type,
        TunnelFormField::LocalPort,
        TunnelFormField::RemoteHost,
        TunnelFormField::RemotePort,
        TunnelFormField::Label,
        TunnelFormField::AutoConnect,
    ];

    pub(crate) fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub(crate) fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn is_toggle(self) -> bool {
        matches!(self, Self::AutoConnect)
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
    pub auto_connect: bool,
    pub active_field: TunnelFormField,
    pub editing: bool,
    pub edit_snapshot: String,
    pub dirty: bool,
    /// Edit-cursor position (char index) within the active text field.
    pub cursor: usize,
}

impl TunnelFormEdit {
    /// The active field's text buffer, or `None` for the Type / Host fields
    /// (which aren't free-text).
    pub fn active_text_field(&self) -> Option<&str> {
        match self.active_field {
            TunnelFormField::LocalPort => Some(&self.local_port),
            TunnelFormField::RemoteHost => Some(&self.remote_host),
            TunnelFormField::RemotePort => Some(&self.remote_port),
            TunnelFormField::Label => Some(&self.label),
            _ => None,
        }
    }

    pub fn active_text_field_mut(&mut self) -> Option<&mut String> {
        match self.active_field {
            TunnelFormField::LocalPort => Some(&mut self.local_port),
            TunnelFormField::RemoteHost => Some(&mut self.remote_host),
            TunnelFormField::RemotePort => Some(&mut self.remote_port),
            TunnelFormField::Label => Some(&mut self.label),
            _ => None,
        }
    }
}

/// Item pending confirmation before deletion.
#[derive(Debug, Clone)]
pub enum PendingDelete {
    Host {
        id: i64,
        name: String,
    },
    Identity {
        id: i64,
        name: String,
    },
    Group {
        id: i64,
        name: String,
    },
    Tunnel {
        id: i64,
        label: String,
    },
    /// A file/directory in the SFTP browser (remote via the worker, local via
    /// `std::fs`). Directories are removed recursively.
    SftpEntry {
        side: crate::sftp::model::Side,
        path: std::path::PathBuf,
        name: String,
        is_dir: bool,
    },
}

/// Editable metadata field index in [`AppMode::HostDetail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailEditField {
    #[default]
    Tags = 0,
    Description = 1,
    Environment = 2,
    SessionLogging = 3,
}

impl DetailEditField {
    const ALL: [DetailEditField; 4] = [
        DetailEditField::Tags,
        DetailEditField::Description,
        DetailEditField::Environment,
        DetailEditField::SessionLogging,
    ];

    pub(crate) fn next(self) -> Self {
        let idx = self as usize;
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub(crate) fn prev(self) -> Self {
        let idx = self as usize;
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub(crate) fn is_tri_state(self) -> bool {
        matches!(self, Self::SessionLogging)
    }
}

/// In-progress metadata edits while in HostDetail mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostDetailEdit {
    pub tags: String,
    pub description: String,
    pub environment: String,
    pub session_logging: crate::session_log::SessionLoggingOverride,
    pub field: DetailEditField,
    pub cursor: usize,
}

#[allow(clippy::large_enum_variant)]
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

    pub fn session_logging_override(&self) -> crate::session_log::SessionLoggingOverride {
        match self {
            Self::Managed(m) => m.session_logging,
            Self::Legacy { meta, .. } => meta.session_logging,
        }
    }

    pub fn session_transport(&self) -> crate::session_transport::SessionTransport {
        match self {
            Self::Managed(m) => m.transport,
            Self::Legacy { meta, .. } => meta.transport,
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

    /// Ids of every group this host belongs to (all memberships, including
    /// Favorites). Legacy hosts have none.
    pub fn group_ids(&self) -> Vec<i64> {
        match self {
            Self::Managed(m) => m.groups.iter().map(|g| g.id).collect(),
            Self::Legacy { .. } => Vec::new(),
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

/// State of the keybinding editor overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeybindEditor {
    /// Index into [`KeyAction::ALL`].
    pub selected: usize,
    /// First visible row in the action list (for scrolling).
    pub scroll: usize,
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
    /// Highlighted row in the Group multi-select dropdown (0-based over
    /// `app.groups` then the "+ New group…" row). Selection state itself lives
    /// in `group_ids`.
    pub group_index: usize,
    /// Ids of every non-reserved group the host is assigned to (multi-select).
    /// Favorites is never listed here — it's toggled with `f`.
    pub group_ids: std::collections::BTreeSet<i64>,
    pub identity_index: usize,
    pub tags: String,
    pub proxy_jump: String,
    pub forward_agent: bool,
    pub remote_command: String,
    pub transport: crate::session_transport::SessionTransport,
    pub session_logging: crate::session_log::SessionLoggingOverride,
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
    Transport = 10,
    SessionLogging = 11,
    OsIcon = 12,
    Password = 13,
    Username = 14,
}

impl HostFormField {
    pub const ALL: [HostFormField; 15] = [
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
        HostFormField::Transport,
        HostFormField::SessionLogging,
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
            HostFormField::Transport => "Transport",
            HostFormField::SessionLogging => "Session log",
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
        matches!(self, HostFormField::ForwardAgent | HostFormField::Transport)
    }

    pub(crate) fn is_tri_state(self) -> bool {
        matches!(self, HostFormField::SessionLogging)
    }
}

/// Focusable field in the group form. `↑/↓` (or Tab) move between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupFormField {
    Name,
    Parent,
    Identity,
}

impl GroupFormField {
    pub const ALL: [GroupFormField; 3] = [
        GroupFormField::Name,
        GroupFormField::Parent,
        GroupFormField::Identity,
    ];
}

/// In-progress group form while in [`AppMode::GroupForm`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupFormEdit {
    pub id: Option<i64>,
    pub name: String,
    pub cursor: usize,
    /// Default identity new hosts in this group inherit. Picked via a dropdown.
    pub default_identity_id: Option<i64>,
    /// Parent group for nesting (`None` = top level). Picked via a dropdown.
    pub parent_id: Option<i64>,
    /// Which field is focused.
    pub field: GroupFormField,
    /// Return to GroupManage after save/cancel (vs Normal when opened from Ctrl+G shortcut).
    pub return_to_manage: bool,
}

/// Dropdown list picker for a group-form field ([`AppMode::GroupFieldPicker`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupFieldPicker {
    /// Which field this dropdown edits (`Parent` or `Identity`).
    pub kind: GroupFormField,
    /// Highlighted row: `0` = the "(none)"/"(top level)" slot, then options.
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

/// Searchable dropdown for opening a new SSH session tab
/// ([`AppMode::SessionHostPicker`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHostPicker {
    /// Case-insensitive substring filter typed by the user.
    pub query: String,
    /// Index into the current filtered match list.
    pub selected: usize,
    /// Mode to restore when the picker is dismissed without connecting.
    pub return_mode: AppMode,
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

/// Which SFTP text prompt is open ([`AppMode::SftpPrompt`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SftpPromptKind {
    Mkdir,
    Rename,
    /// Octal-permission input; `old_path` holds the entry being chmod'd.
    Chmod,
}

/// Single-field text prompt for an SFTP mkdir / rename ([`AppMode::SftpPrompt`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpPromptEdit {
    pub kind: SftpPromptKind,
    pub side: crate::sftp::model::Side,
    /// Directory the name is created/renamed within (the focused pane's cwd).
    pub base: std::path::PathBuf,
    /// For `Rename`: the current path being renamed. `None` for `Mkdir`.
    pub old_path: Option<std::path::PathBuf>,
    pub value: String,
    pub cursor: usize,
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
            HostFormField::ForwardAgent
            | HostFormField::Transport
            | HostFormField::SessionLogging => "",
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
            HostFormField::ForwardAgent
            | HostFormField::Transport
            | HostFormField::SessionLogging => &mut self.address,
            HostFormField::Password => &mut self.password,
        }
    }
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
            DetailEditField::SessionLogging => "",
        }
    }

    pub(crate) fn active_field_mut(&mut self) -> &mut String {
        match self.field {
            DetailEditField::Tags => &mut self.tags,
            DetailEditField::Description => &mut self.description,
            DetailEditField::Environment => &mut self.environment,
            DetailEditField::SessionLogging => &mut self.environment,
        }
    }
}
