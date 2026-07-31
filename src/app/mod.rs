pub(crate) mod adhoc;
mod audit;
mod broadcast;
mod connect;
mod field_picker;
mod groups;
mod host_crud;
mod host_detail;
mod host_form;
mod hostlist;
mod identities;
mod import;
mod keygen;
mod keys;
mod local_shell;
mod mouse;
mod push_key;
mod session;
mod session_picker;
mod session_spawn;
mod sftp;
mod tags;
mod theme_picker;
mod tunnels;
mod types;
mod util;

#[cfg(test)]
mod tests;

pub use theme_picker::{ThemePickerState, ThemeRecordSummary, ThemeRow, ThemeRowStatus};
pub use types::*;
pub use util::*;

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::config::{self, AppConfig, KeyAction};
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
use crate::theme::manager::ThemeManager;
use crate::theme::model::{ResolvedTheme, ThemeDiagnostic, ValidationMode};
use crate::theme::registry::ThemeRegistry;
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

pub const OS_ICON_OPTIONS: [&str; 22] = [
    "(none)",
    "arch",
    "ubuntu",
    "debian",
    "alpine",
    "fedora",
    "rocky",
    "rhel",
    "centos",
    "opensuse",
    "linuxmint",
    "manjaro",
    "popos",
    "kali",
    "gentoo",
    "void",
    "nixos",
    "endeavouros",
    "freebsd",
    "macos",
    "windows",
    "linux",
];

/// A one-line, non-fatal summary of the theme start-up diagnostics, or `None`
/// when everything loaded cleanly.
///
/// Both diagnostic lists the manager collected are eligible. The directory-level
/// warnings — an unusable file name, an unreadable `*.toml` path, the 256-file
/// cut — are *not* filtered out: they are exactly what explains a theme missing
/// from the picker, so hiding them would leave the user with no clue at all.
/// Errors are reported first because an unusable active theme is the thing that
/// actually changed what is on screen.
fn theme_startup_notice(manager: &ThemeManager, load_error: Option<&str>) -> Option<String> {
    let mut messages: Vec<&str> = Vec::new();
    if let Some(error) = load_error {
        messages.push(error);
    }
    let diagnostics = manager.startup_diagnostics();
    let (errors, warnings): (Vec<&ThemeDiagnostic>, Vec<&ThemeDiagnostic>) =
        diagnostics.iter().partition(|d| d.is_error());
    messages.extend(
        errors
            .iter()
            .chain(warnings.iter())
            .map(|d| d.message.as_str()),
    );

    let first = messages.first()?;
    Some(match messages.len() {
        1 => format!("Theme: {first}"),
        n => format!("Theme: {first} (+{} more)", n - 1),
    })
}

/// Preserve earlier startup degradations while adding another one-line notice.
fn append_host_notice(host_notice: &mut Option<String>, notice: String) {
    const SEPARATOR: &str = " | ";

    match host_notice {
        Some(existing)
            if existing == &notice
                || existing
                    .strip_suffix(&notice)
                    .is_some_and(|prefix| prefix.ends_with(SEPARATOR)) => {}
        Some(existing) if !existing.is_empty() => {
            existing.push_str(SEPARATOR);
            existing.push_str(&notice);
        }
        _ => *host_notice = Some(notice),
    }
}

/// Injectable dependencies for [`App`].
pub struct AppDeps {
    pub resolver: Box<dyn HostResolver>,
    pub metadata: Arc<dyn MetadataStore>,
    pub store: Arc<LauncherStore>,
    pub password_store: Box<dyn crate::credentials::PasswordStore>,
}

/// Application state and input handling (TUI loop wired in F9).
pub struct App {
    pub hosts: Vec<HostEntry>,
    pub filtered_indices: Vec<usize>,
    /// Char offsets the live search matched inside each filtered host's
    /// *display name*, keyed by its `hosts` index. Computed once per filter
    /// rebuild rather than per frame, and empty whenever no query is active.
    pub search_matches: std::collections::HashMap<usize, Vec<u32>>,
    pub selected: usize,
    pub search_query: String,
    pub mode: AppMode,
    pub config: AppConfig,
    /// The active runtime theme and the registry it came from. `App` owns it —
    /// there is no global mutable theme state, so renderers take `app.theme()`
    /// (or an explicit `&ResolvedTheme`) and tests stay deterministic.
    ///
    /// **Private on purpose.** Every change to what is painted has to invalidate
    /// the buffer snapshots captured under the old theme, and a public field
    /// would put `ThemeManager::activate_resolved` — and a wholesale
    /// replacement — within reach of any caller, skipping that entirely. The
    /// two mutation paths are [`App::activate_resolved_theme`] and
    /// [`App::replace_theme_manager`]; reading goes through the accessors
    /// below.
    theme_manager: ThemeManager,
    /// Live theme-picker state; `Some` exactly while `mode` is
    /// [`AppMode::ThemePicker`].
    pub theme_picker: Option<ThemePickerState>,
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
    pub keygen_form: Option<KeygenFormEdit>,
    pub keygen_notice: Option<String>,
    pub groups: Vec<HostGroup>,
    /// The reserved Favorites group, kept out of `groups` so it never appears in
    /// the group-manage list or the host-form group selector. Used only when
    /// building the host tree (favourited hosts show under it).
    pub favorites_group: Option<HostGroup>,
    pub host_form: Option<HostFormEdit>,
    pub field_picker: Option<FieldPicker>,
    pub group_form: Option<GroupFormEdit>,
    /// Dedicated default-identity picker for a group (opened with `e`).
    pub group_field_picker: Option<GroupFieldPicker>,
    /// Searchable SSH-server picker for the tunnel form.
    pub tunnel_host_picker: Option<TunnelHostPicker>,
    /// Searchable host picker for a new embedded session tab.
    pub session_picker: Option<SessionPicker>,
    pub push_key_host_picker: Option<PushKeyHostPicker>,
    pub push_key_identity_picker: Option<PushKeyIdentityPicker>,
    pub import_prompt: Option<ImportPromptEdit>,
    /// Open SFTP mkdir / rename text prompt, if any.
    pub sftp_prompt: Option<SftpPromptEdit>,
    /// UI zoom level (0 = default). Widens the hosts column in the layout and
    /// the host-name column within it.
    pub ui_zoom: usize,
    /// Currently focused dashboard panel (issue #18: focus + tmux-style zoom).
    pub focused_panel: PanelId,
    /// Whether the focused dashboard panel is zoomed to the full body.
    pub panel_zoomed: bool,
    /// Active zoom morph (#35): interpolates the focused panel between its grid
    /// slot and the full body on `z` / `Alt+Enter`. `None` when at rest or under
    /// reduced motion.
    pub zoom_anim: Option<crate::tui::tween::SlideAnim>,
    /// Scroll offset within the zoomed panel (issue #18). Reset on zoom/focus
    /// change; each zoomed list panel clamps it to its own content via the
    /// `Cell`'s interior mutability during render.
    pub panel_scroll: std::cell::Cell<u16>,
    /// In-progress text selection over the zoomed panel (issue #18).
    pub panel_sel: Option<PanelSel>,
    /// Text under the current panel selection, extracted from the rendered
    /// buffer each frame (interior mutability so the `&App` render pass can fill
    /// it); copied to the clipboard on mouse release.
    pub panel_sel_text: std::cell::RefCell<String>,
    /// `self.hosts` indices for the rows of a zoomed host-list panel (ping /
    /// recent), filled by their render so Enter connects the selected row.
    pub zoomed_host_idx: std::cell::RefCell<Vec<usize>>,
    /// Live broadcast run (issue #3): `Some` while a fleet command runs or its
    /// finished panel is settling. Owns the run's mpsc + cancel flag.
    pub broadcast: Option<BroadcastState>,
    /// Pre-run broadcast wizard state (pick target / command / preview);
    /// `Some` only while an `AppMode::Broadcast*` stage is active.
    pub broadcast_setup: Option<BroadcastSetup>,
    /// Transient error popups (issue #3), newest last; slide in from the right
    /// above the broadcast panel and expire after `TOAST_TTL`.
    pub broadcast_toasts: Vec<BroadcastToast>,
    /// When the docked panel was dismissed, so lingering toasts can animate
    /// *down* into the freed space instead of jumping. `None` while a panel is
    /// present (or before any run).
    pub broadcast_panel_gone_at: Option<std::time::Instant>,
    pub group_manage_selected: usize,
    pub group_notice: Option<String>,
    pub host_notice: Option<String>,
    /// Message shown by the modal `AppMode::Notice` popup (e.g. a connect error).
    pub notice_popup: Option<String>,
    pub sort_mode: SortMode,
    pub pending_delete: Option<PendingDelete>,
    pub pre_help_mode: Option<AppMode>,
    /// Vertical scroll offset (in lines) of the help overlay.
    pub help_scroll: u16,
    /// Type-to-filter query for the help overlay.
    pub help_query: String,
    /// Mode to return to if the quit dialog is cancelled.
    pub pre_quit_mode: Option<AppMode>,
    pub group_sections: Vec<HostGroupSection>,
    /// Selectable rows (group headers + hosts of expanded groups).
    pub nav_rows: Vec<NavRow>,
    /// Group keys ([`HostGroupSection::key`]) that are currently collapsed.
    pub collapsed_groups: std::collections::HashSet<i64>,
    /// Keybind editor state: `(selected action row, capturing next key)`.
    pub keybind_editor: Option<KeybindEditor>,
    /// Highlighted row in the Settings overlay.
    pub settings_selected: usize,
    /// Highlighted row in the tunnel reconnect settings overlay.
    pub tunnel_reconnect_selected: usize,
    pub active_tab: usize,
    /// Tab shown on the previous poll tick, so a change can be detected centrally
    /// (many code paths set `active_tab`) and turned into a slide animation (#35).
    pub anim_prev_tab: usize,
    /// In-flight tab-switch slide, or `None` at rest / under reduced motion.
    pub tab_switch: Option<TabSwitch>,
    /// When the current `mode` was entered, so popups can animate their open
    /// (#35). Updated centrally on any mode change.
    pub mode_entered_at: std::time::Instant,
    /// `mode` on the previous poll tick, to detect a change centrally.
    pub anim_prev_mode: AppMode,
    /// Resting rect of the popup drawn this frame (set by `popup_open_rect`), so
    /// the render pass can snapshot it for the close animation (#35).
    pub last_popup_rect: std::cell::Cell<Option<ratatui::layout::Rect>>,
    /// Captured cells of the last-rendered popup, thrown upward on close (#35).
    pub popup_snapshot:
        std::cell::RefCell<Option<(ratatui::layout::Rect, ratatui::buffer::Buffer)>>,
    /// Full-frame dashboard snapshot captured just before a popup draws, so the
    /// open slide can restore what's behind the popup and let it drop in from off
    /// the top of the screen (#35).
    pub popup_backdrop: std::cell::RefCell<Option<ratatui::buffer::Buffer>>,
    /// When a popup started closing, driving the upward exit of its snapshot.
    pub popup_closing_at: Option<std::time::Instant>,
    /// When a fresh SSH session was launched (mode → Connecting), so the
    /// full-screen session view can slide in from the right (#35). `None` at rest
    /// / under reduced motion.
    pub session_enter_at: Option<std::time::Instant>,
    /// Full-frame snapshot of the last session view and its capture-time PTY
    /// ownership, so later slides protect remote output but still paint SSHub
    /// chrome (#35).
    pub session_snapshot: std::cell::RefCell<Option<SessionSnapshot>>,
    /// Full-frame snapshot of the last dashboard, captured each frame while off
    /// the session view, so the session sliding in has something to slide *over*
    /// (#35). Without it the columns the slide has not reached yet are blank, and
    /// entering a session flashes a black screen before the host arrives.
    pub dashboard_snapshot: std::cell::RefCell<Option<ratatui::buffer::Buffer>>,
    /// When the host view started exiting (session -> dashboard), driving the
    /// slide-out of `session_snapshot`. `None` at rest / under reduced motion.
    pub session_exit_at: Option<std::time::Instant>,
    /// Header counters as drawn right now (total / online / slow / down),
    /// counting toward their real values rather than snapping (#35). `Cell`
    /// because the render pass owns the frame clock.
    pub header_stats_pos: std::cell::Cell<[f32; 4]>,
    /// When the header counters were last advanced. `None` until first drawn.
    pub header_stats_at: std::cell::Cell<Option<std::time::Instant>>,
    /// Whether a counter is still counting, so the loop keeps the frame rate up.
    pub header_stats_moving: std::cell::Cell<bool>,
    /// Smoothed scroll position of the identities grid, in *lines* (not card
    /// rows), chasing the target offset each frame (#35).
    pub keys_scroll_pos: std::cell::Cell<f32>,
    /// When the identities grid was last advanced. `None` until first drawn.
    pub keys_scroll_at: std::cell::Cell<Option<std::time::Instant>>,
    /// Whether the grid is still catching up to its target offset.
    pub keys_scroll_moving: std::cell::Cell<bool>,
    /// When the dashboard first took the screen, so it can fade up out of the
    /// intro animation instead of replacing it between frames (#35). Set by the
    /// terminal loop once, just before its first draw.
    pub dashboard_at: Option<std::time::Instant>,
    /// Audit filter + range as of the previous tick, so a re-filtered table can
    /// be faded in centrally rather than swapping between frames (#35).
    pub anim_prev_audit: (AuditFilter, AuditRange),
    /// When the audit table was last re-filtered.
    pub audit_filter_at: Option<std::time::Instant>,
    /// Working directories of the two SFTP panes as of the previous tick, so a
    /// directory change can be detected centrally whether it came from the
    /// local filesystem or an async remote listing (#35).
    pub anim_prev_cwd: [std::path::PathBuf; 2],
    /// Per-pane directory change: whether it went deeper, and when. Indexed by
    /// [`crate::sftp::model::Side`] (local, remote).
    pub sftp_nav: [Option<(bool, std::time::Instant)>; 2],
    /// SFTP queue length as of the previous tick, so a newly staged transfer
    /// can be detected centrally and flown in (#35).
    pub anim_prev_queue: usize,
    /// When the last transfer was staged, driving its row's fly-in.
    pub sftp_queue_at: Option<std::time::Instant>,
    /// Fraction of the running SFTP queue drawn on the progress bar, chasing
    /// the real figure so the bar sweeps between the worker's updates (#35).
    pub sftp_progress_pos: std::cell::Cell<f32>,
    /// When the progress bar was last advanced. `None` until first drawn.
    pub sftp_progress_at: std::cell::Cell<Option<std::time::Instant>>,
    /// Whether the bar is still catching up to the real figure.
    pub sftp_progress_moving: std::cell::Cell<bool>,
    /// Latest ping sample per host and when it landed, so a fresh reading can
    /// grow into the sparkline instead of appearing at full height (#35).
    pub ping_sample: std::collections::HashMap<String, (u32, std::time::Instant)>,
    /// Ping class per host as of the previous tick, and when it last changed,
    /// so a host going green or red flashes instead of switching silently (#35).
    pub ping_flash: std::collections::HashMap<String, (crate::ping::PingClass, std::time::Instant)>,
    /// In-flight group fold / unfold (#35). While a *fold* plays its rows are
    /// still live in `nav_rows`; `collapsed_groups` takes the change when the
    /// animation ends (see [`App::flush_pending_fold`]).
    pub fold_anim: Option<FoldAnim>,
    /// `selected` as of the previous poll tick, so a moved cursor can be
    /// detected centrally and its highlight wiped in (#35).
    pub anim_prev_selected: usize,
    /// When the host-list cursor last moved, driving the highlight wipe.
    pub selection_at: Option<std::time::Instant>,
    /// Smoothed scroll position of the host list, in visual rows, chasing the
    /// target offset each frame (#35). `Cell` because the render pass advances
    /// it through `&App`.
    pub host_scroll_pos: std::cell::Cell<f32>,
    /// When the scroll position was last advanced, for the frame delta. `None`
    /// until the list has been drawn once.
    pub host_scroll_at: std::cell::Cell<Option<std::time::Instant>>,
    /// Whether the list is still catching up to its target offset, so the loop
    /// knows to keep the frame rate up.
    pub host_scroll_moving: std::cell::Cell<bool>,
    /// `host_notice` as of the previous poll tick, so a fresh one can be
    /// detected centrally (34 code paths set it) and slid in (#35).
    pub anim_prev_notice: Option<String>,
    /// When the current `host_notice` appeared, driving the toast slide-in.
    pub host_notice_at: Option<std::time::Instant>,
    /// In-flight slide between two embedded session tabs (#35). While it plays,
    /// `session_snapshot` is held frozen on the tab being left behind.
    pub session_tab_switch: Option<SessionTabSwitch>,
    pub palette_query: String,
    pub palette_selected: usize,
    pub palette_results: Vec<usize>,
    pub palette_adhoc: Option<crate::app::adhoc::AdhocTarget>,
    pub ping_rx: Option<Receiver<crate::ping::PingResult>>,
    pub ping_data: std::collections::HashMap<String, Vec<u32>>,
    pub sftp: Option<crate::sftp::model::SftpState>,
    pub sftp_tx: Option<std::sync::mpsc::Sender<crate::sftp::SftpCommand>>,
    pub sftp_rx: Option<std::sync::mpsc::Receiver<crate::sftp::SftpEvent>>,
    /// Name of the host the live SFTP session is connected to, so the browser
    /// can open an SSH session back to the same host (completes the round trip).
    pub sftp_host: Option<String>,
    /// Second SFTP worker, driving the left pane when it browses another server
    /// instead of the local filesystem. `None` while the left pane is local.
    pub sftp_tx2: Option<std::sync::mpsc::Sender<crate::sftp::SftpCommand>>,
    pub sftp_rx2: Option<std::sync::mpsc::Receiver<crate::sftp::SftpEvent>>,
    /// Server-to-server transfer in flight, relayed leg by leg through a local
    /// temp file. `None` whenever the queue is a plain local transfer.
    pub sftp_relay: Option<SftpRelay>,
    /// `QueueDone` events still to be swallowed: a worker always finishes its
    /// run with one, even after an error we already acted on, and acting on it
    /// twice would restart the very transfer that just failed.
    pub sftp_swallow_done: usize,
    /// Whether the run in flight has already reported an error, so its leftover
    /// queue is left for the user to retry rather than restarted automatically.
    pub sftp_run_failed: bool,
    /// True while the SFTP picker's host search input is capturing keys.
    pub sftp_picker_searching: bool,
    /// Remembered dotfile visibility for the SFTP panes, restored from
    /// `ui_state` at startup and applied to each new browser.
    pub sftp_show_hidden: bool,
    /// In-flight SFTP tab sub-state slide and when it started (#35). `None` at
    /// rest / under reduced motion.
    pub sftp_anim: Option<(SftpAnim, std::time::Instant)>,
    /// Snapshot of the SFTP tab body captured each resting frame while a session
    /// is live, so leaving it can slide the captured cells away after the state
    /// itself is gone.
    pub sftp_snapshot: std::cell::RefCell<Option<ratatui::buffer::Buffer>>,
    pub probe_rx: Option<Receiver<crate::ssh::probe::SshLogEntry>>,
    pub os_detect_tx: Option<std::sync::mpsc::Sender<crate::osinfo::OsDetectCmd>>,
    pub os_detect_rx: Option<Receiver<crate::osinfo::OsDetectEvent>>,
    /// Host ids with an in-flight OS detection probe, to avoid re-probing.
    pub os_detect_inflight: std::collections::HashSet<i64>,
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
    /// One-shot startup hook for `auto_connect` tunnels.
    tunnels_auto_started: bool,
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
    password_store: Box<dyn crate::credentials::PasswordStore>,
    search: HostSearch,
}

/// Whether `mode` draws a popup overlay (so its open/close should animate,
/// #35). Excludes the full-screen session modes, which are not popups. The
/// session-host picker counts: it is a popup like any other, drawn over either
/// the dashboard or a live session, both of which snapshot a backdrop for it.
pub(crate) fn is_overlay_mode(mode: AppMode) -> bool {
    !matches!(
        mode,
        AppMode::Normal | AppMode::Connecting | AppMode::Session
    )
}

/// Whether `mode` shows the full-screen embedded session (connecting or live).
pub(crate) fn is_session_mode(mode: AppMode) -> bool {
    matches!(mode, AppMode::Connecting | AppMode::Session)
}

impl App {
    /// Whether UI motion (slides / morphs / fades) should play. Off when the
    /// user set `appearance.disable_animation` (the reduced-motion toggle, also
    /// flipped in Settings). Animation call sites jump straight to the final
    /// state when this is false; [`App::animating`] also returns false so the
    /// render loop never bumps to 60fps for nothing.
    pub(crate) fn motion_enabled(&self) -> bool {
        !self.config.appearance.disable_animation
    }

    /// The secret stored under `key`, or empty when there is none and when the
    /// store refuses to answer (a locked or absent keyring). Used to prefill a
    /// form field, so editing a stored password starts from what is stored
    /// instead of from nothing.
    pub(crate) fn stored_secret(&self, key: &str) -> String {
        self.password_store
            .get(key)
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// Store `secret` under `key`, or remove the entry when it is empty. A form
    /// field cleared on purpose means "there is no secret any more", which used
    /// to be indistinguishable from "left untouched".
    pub(crate) fn put_secret(&self, key: &str, secret: &str) -> anyhow::Result<()> {
        if secret.is_empty() {
            self.password_store.delete(key)
        } else {
            self.password_store.set(key, secret)
        }
    }

    /// Notice a tab change (from any of the many code paths that set
    /// `active_tab`) and arm the body slide (#35). Called once per poll tick.
    pub(crate) fn detect_tab_switch(&mut self) {
        if self.active_tab != self.anim_prev_tab {
            if self.motion_enabled() {
                self.tab_switch = Some(TabSwitch {
                    from: self.anim_prev_tab,
                    to: self.active_tab,
                    at: std::time::Instant::now(),
                });
            }
            self.anim_prev_tab = self.active_tab;
        }
        // Drive popup open/close animations off mode changes (#35). Only a
        // *fresh* open over the dashboard (non-overlay -> overlay) restarts the
        // drop-in, and only a full close (overlay -> non-overlay) throws the
        // snapshot up. Overlay -> overlay (e.g. a form bouncing to its
        // discard-confirm and back on held Esc) does neither, so it never jumps.
        if self.mode != self.anim_prev_mode {
            let now_overlay = is_overlay_mode(self.mode);
            let prev_overlay = is_overlay_mode(self.anim_prev_mode);
            if now_overlay && !prev_overlay {
                self.mode_entered_at = std::time::Instant::now();
                self.popup_closing_at = None;
            } else if prev_overlay && !now_overlay {
                self.popup_closing_at = Some(std::time::Instant::now());
            }
            // Leaving the full-screen host view back to the dashboard slides the
            // captured session snapshot off to the right (#35). Only to Normal —
            // opening the session-host picker over a live session isn't an exit.
            if is_session_mode(self.anim_prev_mode) && self.mode == AppMode::Normal {
                // Leaving supersedes any tab slide still in flight, and releases
                // the snapshot it was holding frozen.
                self.session_tab_switch = None;
                if self.motion_enabled() {
                    self.session_exit_at = Some(std::time::Instant::now());
                } else {
                    *self.session_snapshot.borrow_mut() = None;
                }
            }
            self.anim_prev_mode = self.mode;
        }
        // Retire a finished session-exit slide so its snapshot buffer is freed.
        if self
            .session_exit_at
            .is_some_and(|at| at.elapsed() >= crate::tui::SESSION_ANIM)
        {
            self.session_exit_at = None;
            *self.session_snapshot.borrow_mut() = None;
        }
        // The audit table was re-filtered: fade the new rows in (#35).
        if (self.audit_filter, self.audit_range) != self.anim_prev_audit {
            self.anim_prev_audit = (self.audit_filter, self.audit_range);
            self.audit_filter_at = Some(std::time::Instant::now());
        }
        self.detect_sftp_navigation();
        // A transfer staged since the last tick flies its row in (#35).
        let queued = self.sftp.as_ref().map(|s| s.queue.len()).unwrap_or(0);
        if queued != self.anim_prev_queue {
            // Only a fresh entry animates; removing one is the user's own key
            // press and reads fine as an immediate change.
            self.sftp_queue_at = (queued > self.anim_prev_queue).then(std::time::Instant::now);
            self.anim_prev_queue = queued;
        }
        self.detect_ping_changes();
        // Retire a finished fold, releasing the rows it was replaying.
        if self
            .fold_anim
            .as_ref()
            .is_some_and(|f| f.at.elapsed() >= crate::tui::FOLD_ANIM)
        {
            self.fold_anim = None;
        }
        // The cursor moved since the last tick: wipe its highlight in (#35).
        if self.selected != self.anim_prev_selected {
            self.anim_prev_selected = self.selected;
            self.selection_at = Some(std::time::Instant::now());
        }
        // A notice that changed since the last tick is a fresh one: stamp it so
        // the zoom toast can slide in from the right edge (#35).
        if self.host_notice != self.anim_prev_notice {
            self.anim_prev_notice = self.host_notice.clone();
            self.host_notice_at = self.host_notice.as_ref().map(|_| std::time::Instant::now());
        }
        // Retire a finished session-tab slide, releasing the frozen snapshot of
        // the tab that was left behind.
        if self
            .session_tab_switch
            .is_some_and(|sw| sw.at.elapsed() >= crate::tui::TAB_ANIM)
        {
            self.session_tab_switch = None;
        }
        // Retire a finished SFTP sub-state slide. Its snapshot is refreshed every
        // resting frame while a session is live, so only free it once there is
        // none left to capture.
        if self
            .sftp_anim
            .is_some_and(|(_, at)| at.elapsed() >= crate::tui::SFTP_ANIM)
        {
            self.sftp_anim = None;
            if self.sftp.is_none() {
                *self.sftp_snapshot.borrow_mut() = None;
            }
        }
        // Retire a finished close slide so its snapshot buffer is freed.
        if self
            .popup_closing_at
            .is_some_and(|at| at.elapsed() >= crate::tui::POPUP_ANIM)
        {
            self.popup_closing_at = None;
            *self.popup_snapshot.borrow_mut() = None;
        }
    }

    /// The theme every renderer paints with.
    pub fn theme(&self) -> &ResolvedTheme {
        self.theme_manager.theme()
    }

    /// Id of the theme currently painting — which during a picker preview is
    /// not the saved one.
    pub fn active_theme_id(&self) -> &str {
        self.theme_manager.active_id()
    }

    /// Id `config.toml` holds, i.e. what the next start would load.
    pub fn saved_theme_id(&self) -> &str {
        self.theme_manager.saved_id()
    }

    /// Every theme that was found, valid or not — what the picker lists.
    pub fn theme_registry(&self) -> &ThemeRegistry {
        self.theme_manager.registry()
    }

    /// The directory user themes were loaded from, or `None` for a manager that
    /// belongs to no directory (tests, or no config directory).
    pub fn themes_dir(&self) -> Option<&Path> {
        self.theme_manager.themes_dir()
    }

    /// Swap the whole theme manager, invalidating what the old theme painted.
    ///
    /// The *other* half of the seam. [`App::activate_resolved_theme`] moves the
    /// active theme within one manager; this replaces the manager itself
    /// (registry, saved id and start-up diagnostics), which activation cannot
    /// express. Both end in [`App::invalidate_theme_visual_state`], and with
    /// `theme_manager` private they are the only two ways the painted theme can
    /// change at all.
    fn replace_theme_manager(&mut self, manager: ThemeManager) {
        self.theme_manager = manager;
        self.invalidate_theme_visual_state();
    }

    /// Drop every buffer snapshot and in-flight slide that was captured under
    /// the theme being replaced.
    ///
    /// The frame pipeline's background painters select cells by their *current*
    /// colour (`CellSelection::Matching(Color::Reset)`) and the slides blit
    /// cells captured on an earlier frame. A snapshot that outlived its theme
    /// would therefore be matched — and composited — against colours no longer
    /// on screen, so it has to go **before** the next frame runs rather than
    /// when its animation happens to expire.
    ///
    /// Only visual leftovers are cleared. State that carries a pending
    /// *decision* (`fold_anim`, whose collapse is applied when it ends) is
    /// deliberately left alone: dropping it would lose a user action, not a
    /// stale colour.
    pub(crate) fn invalidate_theme_visual_state(&mut self) {
        *self.popup_snapshot.borrow_mut() = None;
        *self.popup_backdrop.borrow_mut() = None;
        *self.session_snapshot.borrow_mut() = None;
        *self.sftp_snapshot.borrow_mut() = None;
        self.popup_closing_at = None;
        self.session_enter_at = None;
        self.session_exit_at = None;
        self.session_tab_switch = None;
        self.sftp_anim = None;
        self.tab_switch = None;
        self.zoom_anim = None;
        // A live preview activates on every arrow key while the picker popup is
        // open. Its backdrop has just been dropped, so leaving the mode clock
        // inside `POPUP_ANIM` would re-capture one and replay the drop-in for
        // each keystroke; settling the clock keeps the popup where it is.
        self.mode_entered_at = std::time::Instant::now()
            .checked_sub(crate::tui::POPUP_ANIM)
            .unwrap_or_else(std::time::Instant::now);
    }

    /// Load the user's themes from `themes_dir` and activate
    /// `appearance.active_theme`.
    ///
    /// Deliberately infallible. A `ThemeRegistryError` — an unreadable themes
    /// directory, say — degrades to the embedded built-ins plus a non-fatal
    /// hint rather than propagating, so a broken `themes/` can never stop SSHub
    /// from starting. A missing or invalid theme id likewise falls back to
    /// `default` while `saved_id` keeps the configured value, and `config.toml`
    /// is never rewritten: repairing the theme file is enough to get it back.
    ///
    /// Public because it is the **only** loading entry point: it changes what
    /// is painted, so it goes through [`App::replace_theme_manager`] and runs
    /// the snapshot invalidation itself. Nothing that reaches around that
    /// invalidation is exposed. `App::new` calls it with the installed
    /// directory; an end-to-end test calls it with a temporary one, which is
    /// how a workflow test stays off the real config directory.
    pub fn load_themes_from(&mut self, themes_dir: &Path) {
        let saved_id = self.config.appearance.active_theme.clone();
        let (manager, load_error) =
            match ThemeRegistry::load_installed(themes_dir, ValidationMode::Compatible) {
                Ok(registry) => (
                    ThemeManager::from_registry(registry, themes_dir.to_path_buf(), saved_id),
                    None,
                ),
                Err(e) => (
                    // `builtins_at`, not `builtins`: the degraded manager must
                    // keep pointing at the directory that failed, or a reload
                    // after the user repairs it has nowhere to look.
                    ThemeManager::builtins_at(saved_id, themes_dir.to_path_buf()),
                    Some(format!(
                        "{} could not be read ({e}); using the built-in themes",
                        themes_dir.display()
                    )),
                ),
            };
        self.replace_theme_manager(manager);
        if let Some(notice) = theme_startup_notice(&self.theme_manager, load_error.as_deref()) {
            append_host_notice(&mut self.host_notice, notice);
        }
    }

    /// Build app with default resolver and on-disk metadata db.
    pub fn new(config: AppConfig) -> Result<Self> {
        let data_dir = config::data_dir()?;
        std::fs::create_dir_all(&data_dir)?;

        let launcher_path = data_dir.join("launcher.db");
        let first_run = !launcher_path.exists();

        let db_path = data_dir.join("metadata.db");
        let metadata = Arc::new(MetadataDb::open(db_path)?);
        let store = Arc::new(LauncherStore::open(launcher_path)?);
        let resolver = Box::new(SshConfigResolver::default());
        let keyring_available = crate::credentials::check_keyring_available();
        let password_store: Box<dyn crate::credentials::PasswordStore> = if keyring_available {
            let _ = crate::credentials::migrate_fallback_to_keyring();
            Box::new(crate::credentials::OsKeyring)
        } else {
            Box::new(crate::credentials::FilePasswordStore::new(
                data_dir.join("credentials.json"),
            ))
        };

        let mut app = Self::new_with_deps(
            config,
            AppDeps {
                resolver,
                metadata,
                store,
                password_store,
            },
        );
        if !keyring_available {
            app.host_notice =
                Some("OS keyring unavailable. Using credentials.json fallback.".into());
        }

        // Themes load before the hosts so a start-up theme hint is already in
        // place when the first frame draws. Non-fatal by construction: no
        // branch here may `?`, because `default` is embedded and SSHub must
        // start even with a broken themes directory.
        match config::config_dir() {
            Ok(dir) => app.load_themes_from(&dir.join("themes")),
            Err(e) => append_host_notice(
                &mut app.host_notice,
                format!("Theme: no config directory ({e}); using the built-in themes"),
            ),
        }

        app.reload_hosts()?;
        app.refresh_auth_cache();
        app.start_ping_worker();

        // Spawn the OS auto-detect worker here (the on-disk constructor) rather
        // than in new_with_deps so unit tests that inject deps stay offline and
        // never leak a live ssh-probing thread.
        let (os_tx, os_rx) = crate::osinfo::spawn_os_detect_worker();
        app.os_detect_tx = Some(os_tx);
        app.os_detect_rx = Some(os_rx);

        if first_run && app.hosts.is_empty() {
            app.mode = AppMode::Help;
        }

        Ok(app)
    }

    /// Build app from explicit dependencies (tests inject mocks here).
    pub fn new_with_deps(config: AppConfig, deps: AppDeps) -> Self {
        // Built-ins only: no config directory is read here, so the many tests
        // that inject deps stay offline and `AppDeps` gains no new field.
        // `App::new` swaps this for the installed registry.
        let theme_manager = ThemeManager::builtins(config.appearance.active_theme.clone());
        Self {
            theme_manager,
            theme_picker: None,
            hosts: Vec::new(),
            filtered_indices: Vec::new(),
            search_matches: std::collections::HashMap::new(),
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
            keygen_form: None,
            keygen_notice: None,
            groups: Vec::new(),
            favorites_group: None,
            host_form: None,
            field_picker: None,
            group_form: None,
            group_field_picker: None,
            tunnel_host_picker: None,
            session_picker: None,
            push_key_host_picker: None,
            push_key_identity_picker: None,
            import_prompt: None,
            sftp_prompt: None,
            ui_zoom: 0,
            focused_panel: PanelId::default(),
            panel_zoomed: false,
            zoom_anim: None,
            panel_scroll: std::cell::Cell::new(0),
            panel_sel: None,
            panel_sel_text: std::cell::RefCell::new(String::new()),
            zoomed_host_idx: std::cell::RefCell::new(Vec::new()),
            broadcast: None,
            broadcast_setup: None,
            broadcast_toasts: Vec::new(),
            broadcast_panel_gone_at: None,
            group_manage_selected: 0,
            group_notice: None,
            host_notice: None,
            notice_popup: None,
            sort_mode: SortMode::default(),
            pending_delete: None,
            pre_help_mode: None,
            help_scroll: 0,
            help_query: String::new(),
            pre_quit_mode: None,
            group_sections: Vec::new(),
            nav_rows: Vec::new(),
            collapsed_groups: std::collections::HashSet::new(),
            keybind_editor: None,
            settings_selected: 0,
            tunnel_reconnect_selected: 0,
            active_tab: 0,
            anim_prev_tab: 0,
            tab_switch: None,
            // Start "settled" in the past so the initial mode never counts as
            // just-entered: popups animate only after a real mode change stamps
            // `mode_entered_at` (via detect_tab_switch in the poll loop), so
            // direct-mode-set render tests draw popups at rest, not mid-slide.
            mode_entered_at: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(3600))
                .unwrap_or_else(std::time::Instant::now),
            anim_prev_mode: AppMode::Normal,
            last_popup_rect: std::cell::Cell::new(None),
            popup_snapshot: std::cell::RefCell::new(None),
            popup_backdrop: std::cell::RefCell::new(None),
            popup_closing_at: None,
            session_enter_at: None,
            session_snapshot: std::cell::RefCell::new(None),
            dashboard_snapshot: std::cell::RefCell::new(None),
            session_exit_at: None,
            session_tab_switch: None,
            header_stats_pos: std::cell::Cell::new([0.0; 4]),
            header_stats_at: std::cell::Cell::new(None),
            header_stats_moving: std::cell::Cell::new(false),
            keys_scroll_pos: std::cell::Cell::new(0.0),
            keys_scroll_at: std::cell::Cell::new(None),
            keys_scroll_moving: std::cell::Cell::new(false),
            dashboard_at: None,
            anim_prev_audit: (AuditFilter::default(), AuditRange::default()),
            audit_filter_at: None,
            anim_prev_cwd: [std::path::PathBuf::new(), std::path::PathBuf::new()],
            sftp_nav: [None, None],
            anim_prev_queue: 0,
            sftp_queue_at: None,
            sftp_progress_pos: std::cell::Cell::new(0.0),
            sftp_progress_at: std::cell::Cell::new(None),
            sftp_progress_moving: std::cell::Cell::new(false),
            ping_sample: std::collections::HashMap::new(),
            ping_flash: std::collections::HashMap::new(),
            fold_anim: None,
            anim_prev_selected: 0,
            selection_at: None,
            host_scroll_pos: std::cell::Cell::new(0.0),
            host_scroll_at: std::cell::Cell::new(None),
            host_scroll_moving: std::cell::Cell::new(false),
            anim_prev_notice: None,
            host_notice_at: None,
            palette_query: String::new(),
            palette_selected: 0,
            palette_results: Vec::new(),
            palette_adhoc: None,
            ping_rx: None,
            ping_data: std::collections::HashMap::new(),
            sftp: None,
            sftp_tx: None,
            sftp_rx: None,
            sftp_host: None,
            sftp_tx2: None,
            sftp_rx2: None,
            sftp_relay: None,
            sftp_swallow_done: 0,
            sftp_run_failed: false,
            sftp_picker_searching: false,
            sftp_show_hidden: false,
            sftp_anim: None,
            sftp_snapshot: std::cell::RefCell::new(None),
            probe_rx: None,
            os_detect_tx: None,
            os_detect_rx: None,
            os_detect_inflight: std::collections::HashSet::new(),
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
            tunnels_auto_started: false,
            terminal_area: ratatui::layout::Rect::default(),
            sessions: Vec::new(),
            active_session: None,
            last_click: None,
            resolver: deps.resolver,
            metadata: deps.metadata,
            store: deps.store,
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
            // Respect the audit tab's current filter/range so the periodic
            // refresh doesn't silently wipe the user's filtered view (it used
            // to clobber it with 20 unfiltered rows every 10s).
            let status = self.audit_filter.sql_status();
            let since = self.audit_range.since_timestamp();
            self.auth_events_cache = self
                .store
                .list_auth_events_filtered(status, since, 500)
                .unwrap_or_default();
            self.auth_stats_cache = self.store.auth_event_stats(7).unwrap_or((0, 0));
            // Keep the selection within the refreshed list.
            if self.audit_selected >= self.auth_events_cache.len() {
                self.audit_selected = self.auth_events_cache.len().saturating_sub(1);
            }
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
        if hosts.is_empty() {
            // No hosts to ping — drop any existing worker. Dropping its Receiver
            // makes the old thread exit on its next send instead of leaving it
            // pinging deleted addresses forever.
            self.ping_rx = None;
        } else {
            // Replacing ping_rx drops the previous Receiver, so any prior worker
            // also winds down on its next send.
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

    /// Drop the connect-time SSH debug noise for `host_name` once a session has
    /// authenticated, but keep the launched command line (`$ ssh …`) so the
    /// dashboard still shows how the selected host was connected to.
    pub fn clear_ssh_log_for_host(&mut self, host_name: &str) {
        self.ssh_log
            .retain(|e| e.host_name != host_name || e.line.starts_with("$ "));
    }

    pub fn reload_hosts(&mut self) -> Result<()> {
        let selected_name = self.selected_entry().map(|e| e.name().to_string());
        self.load_collapsed_groups();
        self.load_sftp_hidden();
        self.load_ui_zoom();

        sync_ssh_config_hosts(self.resolver.as_ref(), &self.store)?;

        self.hosts = crate::hosts::load_merged_hosts(
            self.resolver.as_ref(),
            &self.store,
            self.metadata.as_ref(),
        )?;
        self.load_groups()?;
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

    /// Load groups from the store, splitting off the reserved Favorites group so
    /// `self.groups` only ever holds real, user-managed groups.
    pub(crate) fn load_groups(&mut self) -> Result<()> {
        let all = self.store.list_groups()?;
        self.favorites_group = all.iter().find(|g| g.reserved).cloned();
        self.groups = all.into_iter().filter(|g| !g.reserved).collect();
        Ok(())
    }

    /// Groups for building the host tree: real groups plus the Favorites group
    /// prepended so it sorts to the very top (when it has members).
    pub(crate) fn tree_groups(&self) -> Vec<HostGroup> {
        let mut groups = Vec::with_capacity(self.groups.len() + 1);
        if let Some(fav) = &self.favorites_group {
            groups.push(fav.clone());
        }
        groups.extend(self.groups.iter().cloned());
        groups
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
