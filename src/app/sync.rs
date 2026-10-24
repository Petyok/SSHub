//! The Sync tab (tab 7) and its modals.
//!
//! The UI reads one plain view model, [`SyncView`], that the sync stages fill:
//! nothing here depends on the op-log, the store or the sync worker. The view
//! types mirror the names of the contract's `DeviceRow`, `DeviceStatus`,
//! `DeviceRole` and `CredMode`, but are the UI's own structs. Building a
//! `SyncView` from store rows happens in exactly one place, a refresh method on
//! `App` that the stage that owns the store adds next to this module; the
//! fields are public so that method just assigns them.
//!
//! What the UI wants done (sync now, enroll a host, approve a change, ...) goes
//! out as a [`SyncIntent`] pushed on [`SyncView::intents`]; the worker side
//! drains them. While [`SyncView::worker`] is false nothing drains them, so the
//! keys that would queue one say so instead.

use ratatui::style::Style;

use super::*;

/// Why a sync key does nothing on a profile without a ring.
const NOT_SET_UP: &str = "sync is not set up on this profile (tab 7 says how)";
/// Why a sync key does nothing while no sync worker runs.
const NO_WORKER: &str = "sync is not available in this build yet";

/// Trust-session terms offered after the scope, in days: a week, a month
/// (the default) and three months (the cap).
pub const TRUST_TTL_DAYS: [u32; 3] = [7, 30, 90];
/// Index of the default term in [`TRUST_TTL_DAYS`].
pub const TRUST_TTL_DEFAULT: usize = 1;

/// Health of a device in the ring (the contract's `DeviceStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatusView {
    Operational,
    Behind,
    Stale,
    Suspect,
    Compromised,
    Revoked,
}

impl DeviceStatusView {
    pub fn label(self) -> &'static str {
        match self {
            Self::Operational => "operational",
            Self::Behind => "behind",
            Self::Stale => "stale",
            Self::Suspect => "suspect",
            Self::Compromised => "COMPROMISED",
            Self::Revoked => "revoked",
        }
    }

    /// The status glyph and the colour role it is drawn in.
    pub fn glyph(self) -> (&'static str, crate::theme::catalog::ColorRole) {
        use crate::theme::catalog::ColorRole::*;
        match self {
            Self::Operational => ("\u{25cf}", StatusSuccess),
            Self::Behind => ("\u{25d0}", StatusWarning),
            Self::Stale => ("\u{25cb}", StatusWarning),
            Self::Suspect => ("\u{25b2}", StatusWarning),
            Self::Compromised => ("\u{2716}", StatusError),
            Self::Revoked => ("\u{25cb}", StatusUnknown),
        }
    }
}

/// What a device may do in the ring (the contract's `DeviceRole`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceRoleView {
    Holder,
    Mirror,
}

/// How credentials are protected (the contract's `CredMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredModeView {
    Vault,
    Quorum,
}

impl CredModeView {
    pub fn label(self) -> &'static str {
        match self {
            Self::Vault => "Vault",
            Self::Quorum => "Quorum",
        }
    }
}

/// One row of the device list (the contract's `DeviceRow`).
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub role: DeviceRoleView,
    /// Unattended devices never vote on Red operations.
    pub unattended: bool,
    pub status: DeviceStatusView,
    /// Credential mode exemption: this device uses the master phrase.
    pub exempt: bool,
    /// Unix seconds of the last contact, `None` when never seen.
    pub last_seen: Option<i64>,
    pub head_seq: u64,
}

/// The active trust session, if one is open.
#[derive(Debug, Clone, PartialEq)]
pub struct TrustView {
    /// Human label of the scope, e.g. `group staging` or `3 hosts`.
    pub scope: String,
    pub term_days: u32,
    pub hosts_opened: u32,
}

/// One field of a pending change.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeView {
    pub field: String,
    pub from: String,
    pub to: String,
    /// Dangerous fields are drawn in the error colour.
    pub dangerous: bool,
}

/// One pending operation awaiting review.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingOpView {
    pub device: String,
    pub summary: String,
    pub changes: Vec<ChangeView>,
}

/// How a share can reach the unlock modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnlockPathKind {
    Peer,
    PhoneQr,
    Clipboard,
    ShortCode,
    Words24,
}

impl UnlockPathKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Peer => "peer device",
            Self::PhoneQr => "phone (QR)",
            Self::Clipboard => "clipboard",
            Self::ShortCode => "short code",
            Self::Words24 => "24 words",
        }
    }
}

/// One unlock path and where it stands, e.g. `(Clipboard, "listening")`.
#[derive(Debug, Clone, PartialEq)]
pub struct UnlockPathView {
    pub kind: UnlockPathKind,
    pub status: String,
}

/// The unlock modal: shares collected so far, the challenge QR and the code
/// field.
#[derive(Debug, Clone, PartialEq)]
pub struct UnlockView {
    pub host: String,
    pub have: usize,
    pub need: usize,
    pub paths: Vec<UnlockPathView>,
    /// Challenge payloads, one QR frame each, looped at 8 fps. Empty hides the
    /// QR.
    pub frames: Vec<Vec<u8>>,
    /// The short code or words typed so far.
    pub code: String,
}

/// The enrollment SAS screen.
#[derive(Debug, Clone, PartialEq)]
pub struct SasView {
    pub peer: String,
    /// The words both screens must show, e.g. `harbor-globe-lantern-mint`.
    pub words: String,
}

/// The fork alert: which device equivocated.
#[derive(Debug, Clone, PartialEq)]
pub struct ForkView {
    pub device: String,
    pub detail: String,
}

/// The holder-side share release confirmation. Names the requesting device.
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseView {
    pub host: String,
    pub requester: String,
    pub requester_id: String,
}

/// Vault first run: the six words.
#[derive(Debug, Clone, PartialEq)]
pub struct OnboardingView {
    pub words: Vec<String>,
}

/// A request from the UI to the sync worker.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncIntent {
    SyncNow { device: String },
    Enroll { host: String },
    Review { approve: bool },
    SasMatches(bool),
    Trust { scope: usize, term_days: u32 },
    Release { approve: bool },
    UnlockCode(String),
    OnboardingDone,
}

/// Everything the Sync UI draws. `Default` is the unconfigured profile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncView {
    /// A ring exists on this profile. False renders the "not set up" state and
    /// hides the tab-bar badge.
    pub configured: bool,
    /// The sync worker is attached and drains [`SyncView::intents`].
    pub worker: bool,
    pub devices: Vec<DeviceView>,
    /// Highlighted device row.
    pub selected: usize,
    /// Operations waiting for review.
    pub pending: usize,
    pub cred_mode: Option<CredModeView>,
    pub trust: Option<TrustView>,
    /// The pending operations the review screen lists.
    pub review: Vec<PendingOpView>,
    pub review_selected: usize,
    /// Scope choices for a new trust session (groups, host lists).
    pub trust_scopes: Vec<String>,
    pub trust_scope_selected: usize,
    pub trust_ttl_selected: usize,
    pub unlock: Option<UnlockView>,
    pub sas: Option<SasView>,
    /// Set by a fork proof; stays set after the alert is dismissed.
    pub fork: Option<ForkView>,
    pub release: Option<ReleaseView>,
    pub onboarding: Option<OnboardingView>,
    /// Requests for the worker, oldest first.
    pub intents: Vec<SyncIntent>,
    /// Unlock QR frame shown now; advanced from the app tick.
    pub qr_frame: usize,
    qr_last: Option<std::time::Instant>,
    /// A round with a peer is running (set when a request goes to the worker).
    pub syncing: bool,
    /// The worker's latest progress line, while a round runs.
    pub progress: Option<String>,
    /// The last round's failure; cleared when the next request goes out.
    pub last_error: Option<String>,
    /// The host an enrollment was started for (names the SAS peer).
    pub enrolling: Option<String>,
    /// Mode the fork alert interrupted, restored on dismissal.
    fork_return: Option<AppMode>,
}

impl SyncView {
    /// A fork proof, or any device marked compromised.
    pub fn compromised(&self) -> bool {
        self.fork.is_some()
            || self
                .devices
                .iter()
                .any(|d| d.status == DeviceStatusView::Compromised)
    }

    /// Devices currently in sync.
    pub fn reachable(&self) -> usize {
        self.devices
            .iter()
            .filter(|d| d.status == DeviceStatusView::Operational)
            .count()
    }

    fn selected_device(&self) -> Option<&DeviceView> {
        self.devices.get(self.selected)
    }
}

impl App {
    pub(crate) fn switch_to_sync_tab(&mut self) {
        self.active_tab = 6;
    }

    /// The tab-bar badge, `⇅N ⏳Nops`, with the colour it is drawn in. `None`
    /// while no ring is configured.
    pub fn sync_badge(&self) -> Option<(String, Style)> {
        use crate::theme::catalog::ColorRole;
        let sync = &self.sync;
        if !sync.configured {
            return None;
        }
        let mut text = format!("\u{21c5}{}", sync.reachable());
        if sync.pending > 0 {
            text.push_str(&format!(" \u{23f3}{}ops", sync.pending));
        }
        let role = if sync.compromised() {
            ColorRole::StatusError
        } else if sync.pending > 0 {
            ColorRole::StatusWarning
        } else {
            ColorRole::StatusSuccess
        };
        Some((text, Style::default().fg(self.theme().color(role))))
    }

    /// Everything the tab bar shows besides the tabs and the version.
    pub fn tab_badges(&self) -> crate::tui::widgets::tab_bar::TabBadges {
        crate::tui::widgets::tab_bar::TabBadges {
            update: self.update_badge(),
            sync: self.sync_badge(),
        }
    }

    pub(crate) fn handle_key_sync(&mut self, key: KeyEvent) -> Result<()> {
        self.host_notice = None;
        if self.try_tab_switch(&key)? {
            return Ok(());
        }
        match key.code {
            _ if self.is_action(KeyAction::Quit, &key) => self.request_quit(),
            _ if self.is_action(KeyAction::Help, &key) => self.open_help(),
            _ if self.is_action(KeyAction::MoveDown, &key) => {
                let last = self.sync.devices.len().saturating_sub(1);
                self.sync.selected = (self.sync.selected + 1).min(last);
            }
            _ if self.is_action(KeyAction::MoveUp, &key) => {
                self.sync.selected = self.sync.selected.saturating_sub(1);
            }
            _ if self.is_action(KeyAction::SyncNow, &key) => self.sync_now(),
            _ if self.is_action(KeyAction::SyncReview, &key) => self.sync_open_review(),
            _ if self.is_action(KeyAction::SyncTrust, &key) => self.sync_open_trust(),
            _ => {}
        }
        Ok(())
    }

    /// The tab's gate: a ring and a worker, or a notice saying which is
    /// missing.
    fn sync_ready(&mut self) -> bool {
        let notice = if !self.sync.configured {
            NOT_SET_UP
        } else if !self.sync.worker {
            NO_WORKER
        } else {
            return true;
        };
        self.host_notice = Some(notice.into());
        false
    }

    fn sync_now(&mut self) {
        if !self.sync_ready() {
            return;
        }
        match self.sync.selected_device().map(|d| d.id.clone()) {
            Some(device) => self.sync.intents.push(SyncIntent::SyncNow { device }),
            None => self.host_notice = Some("no device to sync with".into()),
        }
    }

    fn sync_open_review(&mut self) {
        if !self.sync.configured {
            self.host_notice = Some(NOT_SET_UP.into());
        } else if self.sync.review.is_empty() {
            self.host_notice = Some("nothing waiting for review".into());
        } else {
            self.sync.review_selected = 0;
            self.mode = AppMode::SyncReview;
        }
    }

    fn sync_open_trust(&mut self) {
        if !self.sync.configured {
            self.host_notice = Some(NOT_SET_UP.into());
        } else if self.sync.trust_scopes.is_empty() {
            self.host_notice = Some("no group or host list to scope a trust session to".into());
        } else {
            self.sync.trust_scope_selected = 0;
            self.sync.trust_ttl_selected = TRUST_TTL_DEFAULT;
            self.mode = AppMode::TrustSessionScope;
        }
    }

    /// `SyncEnroll` on the hosts tab: make the selected host one of my devices.
    pub(crate) fn sync_enroll_selected_host(&mut self) {
        let Some(host) = self
            .selected_host_index()
            .and_then(|i| self.hosts.get(i))
            .map(|h| h.name().to_string())
        else {
            self.host_notice = Some("select a host to enroll as a device".into());
            return;
        };
        if !self.sync.worker {
            self.host_notice = Some(NO_WORKER.into());
            return;
        }
        self.sync.intents.push(SyncIntent::Enroll { host });
    }

    /// Open the unlock modal over whatever is on screen.
    pub fn open_sync_unlock(&mut self, view: UnlockView) {
        self.sync.qr_frame = 0;
        self.sync.qr_last = None;
        self.sync.unlock = Some(view);
        self.mode = AppMode::SyncUnlock;
    }

    /// App-tick hook: step the unlock QR's frame at 8 fps while its modal is up.
    pub(crate) fn advance_sync_qr(&mut self, now: std::time::Instant) {
        if self.mode != AppMode::SyncUnlock {
            return;
        }
        let count = self.sync.unlock.as_ref().map_or(0, |u| u.frames.len());
        let sync = &mut self.sync;
        crate::tui::widgets::qr_frames::advance(&mut sync.qr_frame, &mut sync.qr_last, now, count);
    }

    pub fn open_sync_sas(&mut self, view: SasView) {
        self.sync.sas = Some(view);
        self.mode = AppMode::SyncSas;
    }

    /// Hand the worker the protocol package built; the tab's keys then work.
    pub fn attach_sync_worker(&mut self, handle: crate::sync::client::SyncHandle) {
        self.sync.worker = true;
        self.sync_worker = Some(handle);
    }

    /// Per-tick: pass the UI's queued requests to the worker, then fold the
    /// worker's events into the [`SyncView`]. This is the one place worker
    /// events become view state.
    pub fn poll_sync(&mut self) {
        self.forward_sync_intents();
        while let Some(Ok(event)) = self.sync_worker.as_ref().map(|w| w.try_recv()) {
            self.apply_sync_event(event);
        }
    }

    fn forward_sync_intents(&mut self) {
        use crate::sync::client::SyncCmd;
        if self.sync_worker.is_none() || self.sync.intents.is_empty() {
            return;
        }
        let mut kept = Vec::new();
        for intent in std::mem::take(&mut self.sync.intents) {
            let cmd = match &intent {
                SyncIntent::SyncNow { device } => SyncCmd::SyncNow(device.clone()),
                SyncIntent::Enroll { host } => {
                    self.sync.enrolling = Some(host.clone());
                    SyncCmd::EnrollBegin(host.clone())
                }
                SyncIntent::SasMatches(ok) => SyncCmd::EnrollConfirmSas(*ok),
                // Answered by later stages, not by the transport worker.
                _ => {
                    kept.push(intent);
                    continue;
                }
            };
            if !matches!(cmd, SyncCmd::EnrollConfirmSas(_)) {
                self.sync.syncing = true;
                self.sync.progress = None;
                self.sync.last_error = None;
            }
            let sent = self
                .sync_worker
                .as_ref()
                .is_some_and(|w| w.send(cmd).is_ok());
            if !sent {
                self.apply_sync_event(crate::sync::client::SyncEvent::Error(
                    crate::sync::client::WORKER_STOPPED.into(),
                ));
            }
        }
        self.sync.intents = kept;
    }

    fn apply_sync_event(&mut self, event: crate::sync::client::SyncEvent) {
        use crate::sync::client::SyncEvent;
        let sync = &mut self.sync;
        match event {
            SyncEvent::Progress(line) => sync.progress = Some(line),
            SyncEvent::PendingArrived(n) => sync.pending += n,
            SyncEvent::SasPrompt(words) => {
                let peer = sync.enrolling.clone().unwrap_or_else(|| "peer".into());
                self.open_sync_sas(SasView { peer, words });
            }
            SyncEvent::ForkDetected(id) => {
                let device = sync
                    .devices
                    .iter()
                    .find(|d| d.id == id)
                    .map_or_else(|| id.clone(), |d| d.name.clone());
                self.raise_sync_fork_alert(ForkView {
                    device,
                    detail: format!("device id {id}: two different histories under one identity"),
                });
            }
            SyncEvent::BehindPeer(id) => {
                if let Some(d) = sync
                    .devices
                    .iter_mut()
                    .find(|d| d.id == id && d.status == DeviceStatusView::Operational)
                {
                    d.status = DeviceStatusView::Behind;
                }
            }
            SyncEvent::Done => {
                sync.syncing = false;
                sync.progress = None;
                sync.enrolling = None;
            }
            SyncEvent::Error(message) => {
                sync.syncing = false;
                sync.progress = None;
                sync.enrolling = None;
                if message == crate::sync::client::WORKER_STOPPED {
                    // The thread is gone: stop offering the keys that need it.
                    sync.worker = false;
                    self.sync_worker = None;
                }
                self.sync.last_error = Some(message.clone());
                self.host_notice = Some(format!("sync: {message}"));
            }
        }
    }

    pub fn open_sync_release_confirm(&mut self, view: ReleaseView) {
        self.sync.release = Some(view);
        self.mode = AppMode::SyncReleaseConfirm;
    }

    pub fn open_sync_onboarding(&mut self, view: OnboardingView) {
        self.sync.onboarding = Some(view);
        self.mode = AppMode::SyncOnboarding;
    }

    /// A fork proof arrived: take over the whole screen, over a live session
    /// too. Dismissing it restores the mode it interrupted.
    pub fn raise_sync_fork_alert(&mut self, view: ForkView) {
        if self.mode != AppMode::SyncForkAlert {
            self.sync.fork_return = Some(self.mode);
        }
        self.sync.fork = Some(view);
        self.mode = AppMode::SyncForkAlert;
    }

    /// Close a Sync modal and drop back to the dashboard.
    fn sync_close_modal(&mut self) {
        let sync = &mut self.sync;
        sync.unlock = None;
        sync.sas = None;
        sync.release = None;
        sync.onboarding = None;
        self.mode = AppMode::Normal;
    }

    /// Keys of every Sync modal. The fork alert is claimed ahead of everything
    /// else in `handle_key`, so a live session never sees them.
    pub(crate) fn handle_key_sync_modal(&mut self, key: KeyEvent) -> Result<()> {
        let cancel = self.is_action(KeyAction::Cancel, &key);
        let confirm = self.is_action(KeyAction::Connect, &key);
        let down = self.is_action(KeyAction::MoveDown, &key);
        let up = self.is_action(KeyAction::MoveUp, &key);
        let plain = |c: char| key.code == KeyCode::Char(c) && key.modifiers.is_empty();
        match self.mode {
            AppMode::SyncForkAlert => {
                if self.is_action(KeyAction::ForceQuit, &key) {
                    self.should_quit = true;
                } else if cancel || confirm {
                    self.mode = self.sync.fork_return.take().unwrap_or(AppMode::Normal);
                }
            }
            AppMode::SyncReview => {
                let last = self.sync.review.len().saturating_sub(1);
                if cancel {
                    self.mode = AppMode::Normal;
                } else if down {
                    self.sync.review_selected = (self.sync.review_selected + 1).min(last);
                } else if up {
                    self.sync.review_selected = self.sync.review_selected.saturating_sub(1);
                } else if plain('a') || plain('r') {
                    self.sync.intents.push(SyncIntent::Review {
                        approve: plain('a'),
                    });
                }
            }
            AppMode::SyncUnlock => {
                if cancel {
                    self.sync_close_modal();
                } else if let Some(unlock) = self.sync.unlock.as_mut() {
                    match key.code {
                        KeyCode::Enter if !unlock.code.is_empty() => {
                            let code = std::mem::take(&mut unlock.code);
                            self.sync.intents.push(SyncIntent::UnlockCode(code));
                        }
                        KeyCode::Backspace => {
                            unlock.code.pop();
                        }
                        KeyCode::Char(c)
                            if !c.is_control()
                                && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            unlock.code.push(c);
                        }
                        _ => {}
                    }
                }
            }
            AppMode::SyncSas => {
                let matches = self.is_action(KeyAction::ConfirmYes, &key);
                if matches || cancel || plain('n') {
                    self.sync.intents.push(SyncIntent::SasMatches(matches));
                    self.sync_close_modal();
                }
            }
            AppMode::TrustSessionScope => {
                let last = self.sync.trust_scopes.len().saturating_sub(1);
                if cancel {
                    self.mode = AppMode::Normal;
                } else if confirm {
                    self.mode = AppMode::TrustSessionTtl;
                } else if down {
                    self.sync.trust_scope_selected = (self.sync.trust_scope_selected + 1).min(last);
                } else if up {
                    self.sync.trust_scope_selected =
                        self.sync.trust_scope_selected.saturating_sub(1);
                }
            }
            AppMode::TrustSessionTtl => {
                if cancel {
                    self.mode = AppMode::TrustSessionScope;
                } else if confirm {
                    self.sync.intents.push(SyncIntent::Trust {
                        scope: self.sync.trust_scope_selected,
                        term_days: TRUST_TTL_DAYS[self.sync.trust_ttl_selected],
                    });
                    self.mode = AppMode::Normal;
                } else if down {
                    self.sync.trust_ttl_selected =
                        (self.sync.trust_ttl_selected + 1).min(TRUST_TTL_DAYS.len() - 1);
                } else if up {
                    self.sync.trust_ttl_selected = self.sync.trust_ttl_selected.saturating_sub(1);
                }
            }
            AppMode::SyncReleaseConfirm => {
                let approve = confirm || self.is_action(KeyAction::ConfirmYes, &key);
                if approve || cancel || plain('n') {
                    self.sync.intents.push(SyncIntent::Release { approve });
                    self.sync_close_modal();
                }
            }
            AppMode::SyncOnboarding if confirm => {
                self.sync.intents.push(SyncIntent::OnboardingDone);
                self.sync_close_modal();
            }
            _ => {}
        }
        Ok(())
    }
}

/// A configured ring with two devices, two pending ops and one reviewable
/// dangerous change, for render and key tests.
#[cfg(test)]
pub(crate) fn sample_sync_view() -> SyncView {
    let device = |name: &str, status| DeviceView {
        id: format!("id-{name}"),
        name: name.into(),
        role: DeviceRoleView::Holder,
        unattended: false,
        status,
        exempt: false,
        last_seen: Some(1_700_000_000),
        head_seq: 7,
    };
    SyncView {
        configured: true,
        worker: true,
        devices: vec![
            device("laptop", DeviceStatusView::Operational),
            device("server", DeviceStatusView::Stale),
        ],
        pending: 2,
        cred_mode: Some(CredModeView::Vault),
        trust: Some(TrustView {
            scope: "group staging".into(),
            term_days: 30,
            hosts_opened: 3,
        }),
        review: vec![PendingOpView {
            device: "server".into(),
            summary: "edit host web-1".into(),
            changes: vec![ChangeView {
                field: "proxy_command".into(),
                from: "-".into(),
                to: "nc evil 22".into(),
                dangerous: true,
            }],
        }],
        trust_scopes: vec!["group staging".into(), "group prod".into()],
        ..SyncView::default()
    }
}

/// [`sample_sync_view`] with every modal's state filled in.
#[cfg(test)]
pub(crate) fn sample_sync_view_with_modals() -> SyncView {
    SyncView {
        unlock: Some(UnlockView {
            host: "web-1".into(),
            have: 1,
            need: 2,
            paths: vec![
                UnlockPathView {
                    kind: UnlockPathKind::Clipboard,
                    status: "listening".into(),
                },
                UnlockPathView {
                    kind: UnlockPathKind::PhoneQr,
                    status: "show the code to your phone".into(),
                },
            ],
            // A real challenge-sized payload cut for a 100x40 area (version 13).
            frames: crate::sync::fountain::fountain_encode(
                &(0..600u32).map(|b| (b * 7) as u8).collect::<Vec<u8>>(),
                crate::sync::fountain::frame_bytes_for_area(100, 40).unwrap(),
                crate::sync::limits::QR_PARITY_PERCENT,
            )
            .unwrap(),
            code: "ab-12".into(),
        }),
        sas: Some(SasView {
            peer: "laptop".into(),
            words: "harbor-globe-lantern-mint".into(),
        }),
        fork: Some(ForkView {
            device: "server".into(),
            detail: "two different ops at seq 9".into(),
        }),
        release: Some(ReleaseView {
            host: "web-1".into(),
            requester: "new laptop".into(),
            requester_id: "id-9".into(),
        }),
        onboarding: Some(OnboardingView {
            words: ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"]
                .map(String::from)
                .to_vec(),
        }),
        ..sample_sync_view()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{host, key as press, key_char as key, test_app};

    fn configured_app() -> App {
        let mut app = test_app(vec![]);
        app.sync = sample_sync_view();
        app
    }

    #[test]
    fn key_7_opens_the_sync_tab_and_keys_say_it_is_not_set_up() {
        let mut app = test_app(vec![]);
        app.handle_key(key('7')).unwrap();
        assert_eq!(app.active_tab, 6);
        for c in ['y', 't'] {
            app.host_notice = None;
            app.handle_key(key(c)).unwrap();
            assert_eq!(app.host_notice.as_deref(), Some(NOT_SET_UP), "{c}");
        }
        app.host_notice = None;
        app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT))
            .unwrap();
        assert_eq!(app.host_notice.as_deref(), Some(NOT_SET_UP));
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.sync.intents.is_empty());
        assert!(app.sync_badge().is_none(), "no badge without a ring");
    }

    #[test]
    fn enroll_key_on_the_hosts_tab_needs_a_worker() {
        let mut app = test_app(vec![("web-1", host("web-1"))]);
        app.selected = app
            .nav_rows
            .iter()
            .position(|r| matches!(r, NavRow::Host(_)))
            .unwrap();
        app.handle_key(key('t')).unwrap();
        assert_eq!(app.host_notice.as_deref(), Some(NO_WORKER));
        app.sync.worker = true;
        app.handle_key(key('t')).unwrap();
        assert_eq!(
            app.sync.intents,
            vec![SyncIntent::Enroll {
                host: "web-1".into()
            }]
        );
    }

    #[test]
    fn badge_counts_devices_and_pending_and_turns_red_on_compromise() {
        let mut app = configured_app();
        let (text, _) = app.sync_badge().unwrap();
        assert_eq!(text, "\u{21c5}1 \u{23f3}2ops");
        let red = Style::default().fg(app
            .theme()
            .color(crate::theme::catalog::ColorRole::StatusError));
        assert_ne!(app.sync_badge().unwrap().1, red);
        app.sync.devices[1].status = DeviceStatusView::Compromised;
        assert_eq!(app.sync_badge().unwrap().1, red);
    }

    #[test]
    fn sync_now_targets_the_selected_device_and_review_opens_the_modal() {
        let mut app = configured_app();
        app.active_tab = 6;
        app.handle_key(press(KeyCode::Down)).unwrap();
        app.handle_key(key('y')).unwrap();
        assert_eq!(
            app.sync.intents,
            vec![SyncIntent::SyncNow {
                device: "id-server".into()
            }]
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT))
            .unwrap();
        assert_eq!(app.mode, AppMode::SyncReview);
        app.handle_key(key('a')).unwrap();
        assert_eq!(
            app.sync.intents.last(),
            Some(&SyncIntent::Review { approve: true })
        );
        app.handle_key(press(KeyCode::Esc)).unwrap();
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn trust_session_asks_scope_first_then_term_and_defaults_to_a_month() {
        let mut app = configured_app();
        app.active_tab = 6;
        app.handle_key(key('t')).unwrap();
        assert_eq!(app.mode, AppMode::TrustSessionScope);
        app.handle_key(press(KeyCode::Down)).unwrap();
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(app.mode, AppMode::TrustSessionTtl);
        assert_eq!(TRUST_TTL_DAYS[app.sync.trust_ttl_selected], 30);
        app.handle_key(press(KeyCode::Down)).unwrap();
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(
            app.sync.intents,
            vec![SyncIntent::Trust {
                scope: 1,
                term_days: 90
            }]
        );
    }

    #[test]
    fn unlock_modal_collects_a_code_and_a_paste_reaches_it() {
        let mut app = configured_app();
        app.open_sync_unlock(UnlockView {
            host: "web-1".into(),
            have: 1,
            need: 2,
            paths: vec![UnlockPathView {
                kind: UnlockPathKind::Clipboard,
                status: "listening".into(),
            }],
            frames: Vec::new(),
            code: String::new(),
        });
        app.handle_paste("ab-12\n").unwrap();
        assert_eq!(app.sync.unlock.as_ref().unwrap().code, "ab-12");
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(
            app.sync.intents,
            vec![SyncIntent::UnlockCode("ab-12".into())]
        );
        app.handle_key(press(KeyCode::Esc)).unwrap();
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.sync.unlock.is_none());
    }

    #[test]
    fn fork_alert_preempts_a_live_session_and_restores_it() {
        let mut app = configured_app();
        app.mode = AppMode::Session;
        app.raise_sync_fork_alert(ForkView {
            device: "server".into(),
            detail: "two different ops at seq 9".into(),
        });
        assert_eq!(app.mode, AppMode::SyncForkAlert);
        // A letter would go to the PTY in session mode; the alert eats it.
        app.handle_key(key('x')).unwrap();
        assert_eq!(app.mode, AppMode::SyncForkAlert);
        app.handle_key(press(KeyCode::Esc)).unwrap();
        assert_eq!(app.mode, AppMode::Session);
        assert!(app.sync.compromised(), "dismissing keeps the red badge");
    }

    #[test]
    fn sas_release_and_onboarding_answer_with_intents() {
        let mut app = configured_app();
        app.open_sync_sas(SasView {
            peer: "laptop".into(),
            words: "harbor-globe-lantern-mint".into(),
        });
        app.handle_key(press(KeyCode::Enter)).unwrap();
        app.open_sync_release_confirm(ReleaseView {
            host: "web-1".into(),
            requester: "new laptop".into(),
            requester_id: "id-9".into(),
        });
        app.handle_key(press(KeyCode::Esc)).unwrap();
        app.open_sync_onboarding(OnboardingView {
            words: vec!["a".into(); 6],
        });
        app.handle_key(press(KeyCode::Esc)).unwrap();
        assert_eq!(
            app.mode,
            AppMode::SyncOnboarding,
            "Esc does not skip the words"
        );
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(
            app.sync.intents,
            vec![
                SyncIntent::SasMatches(true),
                SyncIntent::Release { approve: false },
                SyncIntent::OnboardingDone,
            ]
        );
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn the_app_tick_steps_the_unlock_qr_frame_only_while_its_modal_is_up() {
        let mut app = configured_app();
        app.sync = sample_sync_view_with_modals();
        app.mode = AppMode::SyncUnlock;
        let t0 = std::time::Instant::now();
        app.advance_sync_qr(t0);
        app.advance_sync_qr(t0 + std::time::Duration::from_millis(130));
        assert_eq!(app.sync.qr_frame, 1);
        app.mode = AppMode::Normal;
        app.advance_sync_qr(t0 + std::time::Duration::from_secs(2));
        assert_eq!(app.sync.qr_frame, 1, "no stepping behind another screen");
    }

    /// Poll the app until `done` holds; a worker thread needs real time.
    fn pump(app: &mut App, done: impl Fn(&App) -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done(app) {
            assert!(std::time::Instant::now() < end, "condition never held");
            app.poll_sync();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn app_with_worker(round: crate::sync::client::RoundFn) -> App {
        let mut app = configured_app();
        app.sync.worker = false;
        app.attach_sync_worker(crate::sync::client::spawn_sync_thread(round));
        app
    }

    #[test]
    fn a_queued_sync_now_reaches_the_worker_and_its_events_fill_the_view() {
        use crate::sync::client::{RoundSummary, SyncEvent};
        let mut app = app_with_worker(Box::new(|_k, device, ctx| {
            assert_eq!(device, "id-server");
            ctx.emit(SyncEvent::Progress("talking".into()));
            Ok(RoundSummary { pending_arrived: 3 })
        }));
        app.sync.intents.push(SyncIntent::SyncNow {
            device: "id-server".into(),
        });
        app.poll_sync();
        assert!(app.sync.syncing, "a request marks the round as running");
        assert!(app.sync.intents.is_empty());
        pump(&mut app, |a| !a.sync.syncing);
        assert_eq!(app.sync.pending, 2 + 3, "arrived ops add to the count");
        assert_eq!(app.sync.progress, None);
        assert_eq!(app.sync.last_error, None);
    }

    #[test]
    fn enrollment_shows_the_sas_naming_the_host_and_the_answer_goes_back() {
        use crate::sync::client::{Round, RoundSummary, SyncCmd, SyncEvent};
        let mut app = app_with_worker(Box::new(|kind, _d, ctx| {
            assert_eq!(kind, Round::Enroll);
            ctx.emit(SyncEvent::SasPrompt("harbor-globe-lantern-mint".into()));
            match ctx.recv_cmd(std::time::Duration::from_secs(5)) {
                Some(SyncCmd::EnrollConfirmSas(true)) => Ok(RoundSummary::default()),
                other => anyhow::bail!("not confirmed: {other:?}"),
            }
        }));
        app.sync.intents.push(SyncIntent::Enroll {
            host: "web-1".into(),
        });
        pump(&mut app, |a| a.mode == AppMode::SyncSas);
        let sas = app.sync.sas.clone().unwrap();
        assert_eq!(
            (sas.peer.as_str(), sas.words.as_str()),
            ("web-1", "harbor-globe-lantern-mint")
        );
        app.handle_key(press(KeyCode::Enter)).unwrap(); // the words match
        pump(&mut app, |a| !a.sync.syncing && a.sync.enrolling.is_none());
        assert_eq!(app.sync.last_error, None);
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn fork_behind_and_failure_events_become_alert_status_and_notice() {
        use crate::sync::client::{RoundSummary, SyncEvent};
        let mut app = app_with_worker(Box::new(|_k, device, ctx| match device.as_str() {
            "fork" => {
                ctx.emit(SyncEvent::ForkDetected("id-server".into()));
                Ok(RoundSummary::default())
            }
            "behind" => {
                ctx.emit(SyncEvent::BehindPeer("id-laptop".into()));
                Ok(RoundSummary::default())
            }
            _ => anyhow::bail!("peer said no"),
        }));
        for (device, check) in [
            (
                "behind",
                (|a: &App| a.sync.devices[0].status == DeviceStatusView::Behind)
                    as fn(&App) -> bool,
            ),
            ("fork", |a: &App| a.mode == AppMode::SyncForkAlert),
            ("fail", |a: &App| a.sync.last_error.is_some()),
        ] {
            app.sync.intents.push(SyncIntent::SyncNow {
                device: device.into(),
            });
            pump(&mut app, check);
        }
        assert_eq!(app.sync.fork.as_ref().unwrap().device, "server");
        assert_eq!(app.sync.last_error.as_deref(), Some("peer said no"));
        assert_eq!(app.host_notice.as_deref(), Some("sync: peer said no"));
    }

    #[test]
    fn a_stopped_worker_clears_the_worker_flag_and_says_so() {
        use crate::sync::client::{SyncCmd, WORKER_STOPPED};
        let mut app = app_with_worker(Box::new(|_, _, _| Ok(Default::default())));
        app.sync_worker
            .as_ref()
            .unwrap()
            .send(SyncCmd::Shutdown)
            .unwrap();
        pump(&mut app, |a| a.sync_worker.is_none());
        assert!(!app.sync.worker, "the keys that need a worker say so again");
        assert_eq!(app.sync.last_error.as_deref(), Some(WORKER_STOPPED));
    }

    #[test]
    fn without_a_worker_queued_requests_are_kept_not_dropped() {
        let mut app = configured_app();
        app.sync.worker = false;
        app.sync.intents.push(SyncIntent::OnboardingDone);
        app.poll_sync();
        assert_eq!(app.sync.intents, vec![SyncIntent::OnboardingDone]);
    }
}
