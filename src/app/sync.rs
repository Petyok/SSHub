//! The Sync tab (tab 7) and its modals.
//!
//! The UI reads one plain view model, [`SyncView`]. The view types mirror the
//! names of the contract's `DeviceRow`, `DeviceStatus`, `DeviceRole` and
//! `CredMode`, but are the UI's own structs. The store fills them in exactly
//! one place, `App::refresh_sync_view` in `sync_view.rs`, which also resolves
//! pending operations (review) straight in the store.
//!
//! What needs the network or another stage (sync now, enroll a host, unlock)
//! goes out as a [`SyncIntent`] pushed on [`SyncView::intents`]; the worker
//! side drains them. While [`SyncView::worker`] is false nothing drains them,
//! so the keys that would queue one say so instead.

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
/// Keys are ignored this long after the share release confirmation opens.
const RELEASE_KEY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

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
    /// The word a person reads (glossary): the design terms stay in the details view.
    pub fn label(self) -> &'static str {
        match self {
            Self::Operational => "in sync",
            Self::Behind => "missed some changes",
            Self::Stale => "away too long",
            Self::Suspect => "needs a look",
            Self::Compromised => "COMPROMISED",
            Self::Revoked => "removed",
        }
    }

    /// The design term, for the details view.
    pub fn design_term(self) -> &'static str {
        match self {
            Self::Operational => "operational",
            Self::Behind => "behind",
            Self::Stale => "stale",
            Self::Suspect => "suspect",
            Self::Compromised => "compromised",
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
    /// How this device was admitted, from the local enrollment record.
    pub verified_by: Option<crate::sync::oplog::VerifiedBy>,
    /// This machine itself (enrollment records it in the ring).
    pub this: bool,
    /// The phone (not a computer or server).
    pub phone: bool,
}

impl DeviceView {
    /// What kind of device this is, in the glossary's words.
    pub fn role_word(&self) -> &'static str {
        if self.phone {
            return "phone";
        }
        match self.role {
            DeviceRoleView::Holder => "full device",
            DeviceRoleView::Mirror => "view-only copy",
        }
    }

    /// One plain sentence about what a state means and what to do, when it needs one.
    pub fn explain(&self, now: i64) -> Option<String> {
        let days = self.last_seen.map(|t| ((now - t).max(0)) / 86_400);
        match (self.status, days) {
            (DeviceStatusView::Behind | DeviceStatusView::Stale, Some(d)) if !self.this => Some(format!(
                "{} was away {d} days; nothing is blocked. If a round reports 'behind', press Enter to re-enroll it.",
                self.name
            )),
            (DeviceStatusView::Behind | DeviceStatusView::Stale, None) if !self.this => Some(format!(
                "{} has not synced yet; nothing is blocked. Press Enter to sync it first.",
                self.name
            )),
            (DeviceStatusView::Behind | DeviceStatusView::Stale, _) => Some(
                "This device was away or missed some changes; nothing is blocked."
                    .into(),
            ),
            (DeviceStatusView::Suspect, _) => Some(format!(
                "{}'s history looks wrong but is not proven bad: sync it again, and look at it before you approve anything it asks.",
                self.name
            )),
            (DeviceStatusView::Compromised, _) => Some(format!(
                "{} signed two different histories: treat it as stolen. Press Enter for the proof.",
                self.name
            )),
            _ => None,
        }
    }
}

/// The SSH key fingerprint known for `spec` (a launcher host name or `[user@]host[:port]`), if known_hosts has it.
fn host_fingerprint(store: &crate::store::LauncherStore, spec: &str) -> Option<String> {
    let h = crate::sync::runtime::resolve_host(store, spec).ok()?;
    let name = h.hostname.clone().unwrap_or_else(|| spec.to_string());
    let target = match h.port {
        Some(p) if p != 22 => format!("[{name}]:{p}"),
        _ => name,
    };
    crate::known_hosts::fingerprint_for_host(&target)
}

/// What the device-details QR carries: the plain text the phone app scans in place of a typed id.
pub fn device_qr_payload(device_id: &str) -> Vec<u8> {
    format!("sshub:dev:1:{}", device_id.to_lowercase()).into_bytes()
}

/// The lines that need a person's attention above the device table, most urgent first: `(text, urgent)`.
pub fn attention_lines(sync: &SyncView) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    if let Some(row) = &sync.finish_row {
        out.push((row.clone(), false));
    }
    if sync.ring_secret_missing == Some(true) {
        out.push((
            "This computer has no ring secret, so it cannot open saved passwords. Press M, then Import the Vault kit.".into(),
            false,
        ));
    }
    let h = &sync.health;
    if let Some(left) = h.red_expires_in {
        if left <= 86_400 {
            out.push((
                format!(
                    "A big change waiting for approvals is dropped in {}: press R to finish it or propose it again.",
                    crate::sync::health::human_left(left)
                ),
                true,
            ));
        }
    }
    if h.phone_expired {
        out.push((
            "Trust sessions stopped: the phone has not touched any of your computers for 14 days. Open the phone app and scan something from a terminal.".into(),
            true,
        ));
    } else if h.phone_warn {
        out.push((
            format!(
                "The phone has not touched any of your computers for {} days. Trust sessions stop at 14: open the phone app and scan something from a terminal.",
                h.phone_unseen_days.unwrap_or(7)
            ),
            false,
        ));
    }
    if let Some(d) = h.offline_days {
        out.push((
            format!("No other device has been reached for {d} days: check the connection or the other devices."),
            false,
        ));
    }
    out
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

/// Where the Red ceremony of a pending change stands.
#[derive(Debug, Clone, PartialEq)]
pub struct RedView {
    /// Proofs present, one line each.
    pub have: Vec<String>,
    /// Proofs still missing, one line each.
    pub missing: Vec<String>,
    pub expired: bool,
    /// A rule that makes the change impossible (too few live holders ...).
    pub blocked: Option<String>,
    /// A participant rule the proofs break (the same device giving two of them ...): the change
    /// cannot complete as it stands.
    pub rule_violation: Option<String>,
    /// Why a proof that looks available does not count (the emailed code's sender voted ...).
    pub note: Option<String>,
    /// Only one attended holder terminal: the second device is waived, the paper needed.
    pub single_machine: bool,
    /// Who can give each missing proof and since when it is wanted (`ceremony::blocking_view`), one line each.
    pub blockers: Vec<String>,
}

/// One pending operation awaiting review.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingOpView {
    pub device: String,
    /// Id of the device that wrote the op (the name in `device` is not unique).
    pub author: String,
    pub summary: String,
    pub changes: Vec<ChangeView>,
    /// Hash of the op, for approving or rejecting it in the store.
    pub hash: [u8; 32],
    /// Why it waits (`tier`, `unknown-author`, ...), when the store says.
    pub reason: Option<String>,
    /// The ceremony of a Red change; `None` for everything else.
    pub red: Option<RedView>,
}

/// How a share can reach the unlock modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnlockPathKind {
    Peer,
    PhoneQr,
    Clipboard,
    ShortCode,
    Words24,
    /// The master phrase of a Vault ring.
    Phrase,
    /// The microphone listening for the phone's sound.
    Audio,
    /// The camera reading the phone's code.
    Camera,
    /// The phone page connected over Bluetooth.
    Bluetooth,
}

impl UnlockPathKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Peer => "peer device",
            Self::PhoneQr => "phone (QR)",
            Self::Clipboard => "clipboard",
            Self::ShortCode => "short code",
            Self::Words24 => "24 words",
            Self::Phrase => "master phrase",
            Self::Audio => "microphone",
            Self::Camera => "camera",
            Self::Bluetooth => "bluetooth",
        }
    }
}

/// Set or add the status line of one unlock path.
fn set_path_status(u: &mut UnlockView, kind: UnlockPathKind, status: String) {
    match u.paths.iter_mut().find(|p| p.kind == kind) {
        Some(p) => p.status = status,
        None => u.paths.push(UnlockPathView { kind, status }),
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
    /// The host uid as the phone draws it (groups of 4) and the challenge's four binding words: what the user
    /// compares with the phone's release screen. Empty until the challenge exists.
    pub host_id: String,
    pub words: String,
    /// The short code or words typed so far.
    pub code: String,
}

/// The enrollment SAS screen.
#[derive(Debug, Clone, PartialEq)]
pub struct SasView {
    /// The host the enrollment runs against.
    pub peer: String,
    /// The device name the peer claims.
    pub peer_name: String,
    /// The words both screens must show, e.g. `harbor-globe-lantern-mint`.
    pub words: String,
    /// A human at the other end sees the same words. When false the words prove
    /// nothing and the admission rests on the SSH host key.
    pub peer_has_screen: bool,
    /// The SSH key fingerprint of a peer without a screen, read from known_hosts (`SHA256:...`), when found.
    pub ssh_fingerprint: Option<String>,
}

/// The fork alert: which device equivocated.
#[derive(Debug, Clone, PartialEq)]
pub struct ForkView {
    pub device: String,
    pub detail: String,
    /// What the proof consists of, one line each (hashes, seqs).
    pub evidence: Vec<String>,
}

/// The holder-side share release confirmation. Names the requesting device.
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseView {
    pub host: String,
    pub requester: String,
    pub requester_id: String,
    /// Hash of the pending `ShareRelease` op this answers.
    pub hash: [u8; 32],
    /// How old the request was when it opened.
    pub age_secs: i64,
}

/// What stage 3 stores for Vault: the master phrase and the ring secret.
/// Zeroized on drop; `Debug` never prints them.
#[derive(Clone, PartialEq, Eq)]
pub struct OnboardingSecrets {
    pub phrase: zeroize::Zeroizing<String>,
    pub ring_secret: zeroize::Zeroizing<[u8; 16]>,
}

impl std::fmt::Debug for OnboardingSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OnboardingSecrets(..)")
    }
}

/// Where the Vault first run stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingStep {
    /// The six words, shown once.
    Show,
    /// Two of the words, retyped from the paper.
    Retype,
    /// Choosing where the two sheets go; Enter writes them.
    Kit,
    /// The ring secret on screen, for copying by hand when there is no printer.
    ShowRing,
    /// Both sheets are written.
    Done,
}

/// Vault first run: issue the phrase and ring secret, check the user wrote the
/// phrase down, write the printable kit. The secrets live only here, in
/// zeroizing buffers, until [`SyncIntent::OnboardingDone`] hands them on.
#[derive(Clone, PartialEq)]
pub struct OnboardingView {
    pub step: OnboardingStep,
    secrets: OnboardingSecrets,
    /// Zero-based positions of the two words to retype, ascending.
    pub quiz: [usize; 2],
    pub quiz_done: usize,
    pub typed: String,
    pub phrase_path: String,
    pub ring_path: String,
    /// 0 edits `phrase_path`, 1 edits `ring_path`.
    pub path_field: usize,
    pub notice: Option<String>,
}

impl std::fmt::Debug for OnboardingView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnboardingView")
            .field("step", &self.step)
            .finish_non_exhaustive()
    }
}

impl OnboardingView {
    /// A new phrase (10 CSPRNG bytes as six EFF words) and ring secret.
    pub fn issue() -> Result<Self> {
        use rand_core::{OsRng, RngCore};
        // 10 bytes can exceed 7776^6; draw again until the value is in range.
        let mut entropy = zeroize::Zeroizing::new([0u8; 10]);
        loop {
            OsRng.fill_bytes(&mut entropy[..]);
            if crate::sync::crypto::eff_encode(&entropy).is_ok() {
                break;
            }
        }
        let mut ring = [0u8; 16];
        OsRng.fill_bytes(&mut ring);
        let first = OsRng.next_u32() as usize % 6;
        let second = (first + 1 + OsRng.next_u32() as usize % 5) % 6;
        let home = kit::default_dir();
        let path = |name: &str| home.join(name).to_string_lossy().into_owned();
        Self::with_secrets(
            &entropy,
            ring,
            [first.min(second), first.max(second)],
            (
                path("sshub-vault-phrase.html"),
                path("sshub-vault-ring-secret.html"),
            ),
        )
    }

    /// The same with the secrets, quiz and paths chosen by the caller.
    pub fn with_secrets(
        entropy: &[u8; 10],
        ring_secret: [u8; 16],
        quiz: [usize; 2],
        paths: (String, String),
    ) -> Result<Self> {
        Ok(Self {
            step: OnboardingStep::Show,
            secrets: OnboardingSecrets {
                phrase: zeroize::Zeroizing::new(crate::sync::crypto::eff_encode(entropy)?),
                ring_secret: zeroize::Zeroizing::new(ring_secret),
            },
            quiz,
            quiz_done: 0,
            typed: String::new(),
            phrase_path: paths.0,
            ring_path: paths.1,
            path_field: 0,
            notice: None,
        })
    }

    /// The six words, for the one screen that shows them.
    pub fn words(&self) -> Vec<&str> {
        self.secrets.phrase.split_whitespace().collect()
    }

    pub fn secrets(&self) -> &OnboardingSecrets {
        &self.secrets
    }

    /// The word the user is asked to retype now.
    pub(crate) fn quiz_word(&self) -> &str {
        self.words()[self.quiz[self.quiz_done.min(1)]]
    }
}

/// A request from the UI to the sync worker.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncIntent {
    SyncNow {
        device: String,
    },
    Enroll {
        host: String,
    },
    SasMatches(bool),
    /// The holder's answer to a release request: approve (and grant) or refuse.
    Release {
        approve: bool,
        hash: [u8; 32],
    },
    UnlockCode(String),
    /// Open these synced credentials (store keys) so the action that needs
    /// them can go on; answered by the stage that assembles shares.
    Unlock {
        secrets: Vec<String>,
    },
    /// The Vault first run is finished: the phrase and ring secret for the
    /// sync stage to store as the Vault configuration.
    OnboardingDone(OnboardingSecrets),
    /// Ask the worker which device this is (loads the device key).
    Identify,
    /// An admin job to run on the worker with the device identity.
    Job(crate::sync::runtime::JobBox),
}

/// What runs once credentials are open.
type ResumeFn = Box<dyn FnOnce(&mut App) -> Result<()>>;

/// An action held back until credentials it needs are open.
pub(crate) struct UnlockResume {
    refs: Vec<crate::credentials::SecretRef>,
    run: ResumeFn,
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
    /// What each of `trust_scopes` certifies, in the same order.
    pub trust_scope_values: Vec<crate::sync::oplog::CertScope>,
    pub trust_scope_selected: usize,
    /// Hosts marked with Space in the scope step; when any, they are the scope.
    pub trust_marked: Vec<crate::sync::oplog::HostUid>,
    pub trust_ttl_selected: usize,
    /// New hosts a day the trust session may open, set on the confirm screen (1 to 50).
    pub trust_budget: u32,
    pub unlock: Option<UnlockView>,
    pub sas: Option<SasView>,
    /// Set by a fork proof; stays set after the alert is dismissed.
    pub fork: Option<ForkView>,
    pub release: Option<ReleaseView>,
    /// When the release confirmation opened: keys are ignored for its first moments.
    pub release_opened: Option<std::time::Instant>,
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
    /// Store keys the open unlock modal is for, and whether its round runs.
    pub unlock_keys: Vec<String>,
    pub unlock_running: bool,
    /// The unlock dialog is gathering shares for a mode switch (Quorum to Vault): close it when the round ends.
    pub migrating: bool,
    /// This device's id, once the worker has loaded its identity.
    pub this_device: Option<String>,
    /// What needs a look, for the status bar and the tab header.
    pub health: crate::sync::health::Health,
    /// `None` when a stolen full computer can be cut off, else how many more computers are needed.
    pub cut_off_missing: Option<usize>,
    /// A ring is set up but no paper phrase is registered (design 3.1): a standing line offers the wizard.
    pub no_paper: bool,
    /// A ring is set up but no mail command and recipient are configured: a standing line says what that means.
    pub no_mail: bool,
    /// Sticky lines for trust sessions that end soon and can be renewed.
    pub renewals: Vec<String>,
    /// Plain lines about saved passwords that are exposed or not yet shared with a current full device (design 4.3).
    pub exposure: Vec<String>,
    /// Names of the hosts whose saved password is on either list: the host row carries a marker.
    pub exposed_hosts: Vec<String>,
    /// The Vault's ring secret is not in this profile's credential store (checked once, again after a job).
    pub ring_secret_missing: Option<bool>,
    /// A persistent row for a switch or export that was approved and waits to be finished.
    pub finish_row: Option<String>,
    /// A behind peer and the host to re-enroll it through, after a round said so.
    pub reenroll: Option<(String, String)>,
    identify_after: bool,
    /// Release requests this holder already answered (the op may stay pending
    /// until the worker has resolved it).
    pub answered_releases: Vec<[u8; 32]>,
    /// Mode the fork alert interrupted, restored on dismissal.
    fork_return: Option<AppMode>,
    /// The one alert for recoveries open on this terminal (lines already worded), and the ones Esc dismissed.
    pub alert: Option<RecoveryAlert>,
    pub alert_dismissed: Vec<[u8; 32]>,
    alert_return: Option<AppMode>,
}

/// What the recovery alert shows: plain lines, and the recoveries behind them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryAlert {
    pub lines: Vec<String>,
    pub hashes: Vec<[u8; 32]>,
}

impl SyncView {
    pub(crate) fn alert_return_set(&mut self, mode: AppMode) {
        self.alert_return = Some(mode);
    }

    pub(crate) fn alert_return_take(&mut self) -> AppMode {
        self.alert_return.take().unwrap_or(AppMode::Normal)
    }

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
        if sync.alert.is_some() {
            text.push_str(" recovery open");
        }
        if sync.pending > 0 {
            text.push_str(&format!(" \u{23f3}{}", sync.pending));
        }
        let line = crate::sync::health::status_line(&sync.health);
        if let Some(line) = &line {
            text.push_str(&format!("  {line}"));
        }
        let urgent = sync.health.red_expires_in.is_some_and(|l| l <= 86_400)
            || sync.health.phone_expired
            || sync.alert.is_some();
        let role = if sync.compromised() || urgent {
            ColorRole::StatusError
        } else if sync.pending > 0 || line.is_some() {
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
            _ if self.is_action(KeyAction::SyncAdmin, &key) => self.open_sync_admin(),
            _ if self.is_action(KeyAction::SyncPaper, &key) => {
                super::sync_flows::paper_wizard(self)
            }
            _ if self.is_action(KeyAction::Connect, &key) => self.sync_open_selected(),
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
        match self.sync.selected_device().map(|d| (d.id.clone(), d.this)) {
            Some((_, true)) => self.host_notice = Some("that is this device: pick another".into()),
            Some((device, false)) => self.sync.intents.push(SyncIntent::SyncNow { device }),
            None => self.host_notice = Some("no device to sync with".into()),
        }
    }

    /// Enter on a device row: the evidence of a compromised device, or the
    /// guided way back for one that is behind or stale.
    fn sync_open_selected(&mut self) {
        let Some(d) = self.sync.selected_device().cloned() else {
            return;
        };
        match d.status {
            DeviceStatusView::Compromised => {
                let evidence = self
                    .store
                    .sync_fork_evidence(&d.id)
                    .ok()
                    .flatten()
                    .map(|e| super::sync_view::evidence_lines(&e))
                    .unwrap_or_default();
                self.raise_sync_fork_alert(ForkView {
                    device: d.name.clone(),
                    detail: format!("device id {}", d.id),
                    evidence,
                });
            }
            DeviceStatusView::Behind | DeviceStatusView::Stale => {
                if !self.sync_ready() || d.this {
                    return;
                }
                match self.sync.reenroll.clone().filter(|(id, _)| *id == d.id) {
                    // A round already said it is behind: enroll again.
                    Some((_, host)) => {
                        self.sync.reenroll = None;
                        self.sync.intents.push(SyncIntent::Enroll { host });
                    }
                    None => {
                        self.host_notice = Some(format!(
                            "syncing with {} first; if it reports behind again, press Enter once more to re-enroll it",
                            d.name
                        ));
                        self.sync.intents.push(SyncIntent::SyncNow { device: d.id });
                    }
                }
            }
            _ => self.sync_device_details(&d),
        }
    }

    /// Enter on a device that needs no action: what it is, in plain words with the design terms after them,
    /// and its id as a QR the phone app can scan instead of typing it (U13-16).
    fn sync_device_details(&mut self, d: &DeviceView) {
        let verified = match d.verified_by {
            Some(crate::sync::oplog::VerifiedBy::Sas) => "the four check words matched",
            Some(crate::sync::oplog::VerifiedBy::SshHostKey) => "you trusted its SSH host key",
            None => "not recorded",
        };
        let design_role = match d.role {
            DeviceRoleView::Holder => "holder",
            DeviceRoleView::Mirror => "mirror",
        };
        let mut lines = vec![
            format!("{}{}", d.name, if d.this { " (this device)" } else { "" }),
            format!("Kind: {} (design term: {design_role})", d.role_word()),
            format!(
                "State: {} (design term: {})",
                d.status.label(),
                d.status.design_term()
            ),
            format!(
                "Has a person at the keyboard: {}",
                if d.unattended {
                    "no, it runs on its own"
                } else {
                    "yes"
                }
            ),
            format!("Admitted because: {verified}"),
            format!(
                "Its last signed change: number {} (design term: head)",
                d.head_seq
            ),
        ];
        if d.exempt {
            lines.push("Opens passwords with the master phrase (exemption).".into());
        }
        lines.push(format!("Device id: {}", d.id));
        lines.push(String::new());
        lines.push("The QR is its id. In the phone app, scan it instead of typing the id.".into());
        self.sync_info_with(
            "Device details",
            lines,
            vec![device_qr_payload(&d.id)],
            None,
        );
    }

    pub(crate) fn sync_open_trust_pub(&mut self) {
        self.sync_open_trust();
    }

    pub(crate) fn sync_open_review_pub(&mut self) {
        self.sync_open_review();
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
            self.sync.trust_scope_selected =
                super::sync_trust::default_scope(&self.store, &self.sync.trust_scopes);
            self.sync.trust_marked.clear();
            self.sync.trust_ttl_selected = TRUST_TTL_DEFAULT;
            self.sync.trust_budget = 10;
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
        let count = match (self.mode, self.sync_ui.as_ref()) {
            (AppMode::SyncUnlock, _) => self.sync.unlock.as_ref().map_or(0, |u| u.frames.len()),
            (AppMode::SyncPrompt, Some(super::sync_ui::SyncUi::Prompt { frames, .. }))
            | (AppMode::SyncInfo, Some(super::sync_ui::SyncUi::Info { frames, .. })) => {
                frames.len()
            }
            _ => return,
        };
        let sync = &mut self.sync;
        crate::tui::widgets::qr_frames::advance(&mut sync.qr_frame, &mut sync.qr_last, now, count);
    }

    pub fn open_sync_sas(&mut self, view: SasView) {
        self.sync.sas = Some(view);
        self.mode = AppMode::SyncSas;
    }

    /// Run `job` on the sync worker with the device identity: the way every
    /// admin operation appends ops. Its returned line becomes the notice.
    pub(crate) fn run_sync_job(
        &mut self,
        job: impl FnOnce(&crate::sync::runtime::JobEnv) -> Result<String> + Send + 'static,
    ) {
        if !self.sync.worker {
            self.host_notice = Some(NO_WORKER.into());
            return;
        }
        self.sync
            .intents
            .push(SyncIntent::Job(crate::sync::runtime::JobBox::new(job)));
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
        // A save that could not be recorded for sync is still saved: say so in the status line.
        if let Some(warning) = self.store.take_sync_warning() {
            self.host_notice = Some(format!("saved, but not recorded for sync: {warning}"));
        }
        self.poll_unlock_resume();
        self.forward_sync_intents();
        super::sync_phone::tick(self);
        super::sync_trust::tick(self);
        while let Some(Ok(event)) = self.sync_worker.as_ref().map(|w| w.try_recv()) {
            self.apply_sync_event(event);
        }
    }

    fn forward_sync_intents(&mut self) {
        use crate::sync::client::SyncCmd;
        if self.sync_worker.is_none() || self.sync.intents.is_empty() {
            return;
        }
        for intent in std::mem::take(&mut self.sync.intents) {
            let cmd = match &intent {
                SyncIntent::SyncNow { device } => SyncCmd::SyncNow(device.clone()),
                SyncIntent::Enroll { host } => {
                    self.sync.identify_after = true;
                    self.sync.enrolling = Some(host.clone());
                    SyncCmd::EnrollBegin(host.clone())
                }
                SyncIntent::SasMatches(ok) => SyncCmd::EnrollConfirmSas(*ok),
                SyncIntent::Unlock { secrets } => {
                    self.sync.unlock_keys = secrets.clone();
                    self.sync.unlock_running = true;
                    SyncCmd::Unlock(secrets.clone())
                }
                SyncIntent::UnlockCode(code) => {
                    // A failed attempt ended the round: answering again
                    // starts it over with the same credentials.
                    if !self.sync.unlock_running && !self.sync.unlock_keys.is_empty() {
                        self.sync.unlock_running = true;
                        if let Some(w) = &self.sync_worker {
                            let _ = w.send(SyncCmd::Unlock(self.sync.unlock_keys.clone()));
                        }
                    }
                    SyncCmd::UnlockCode(code.clone())
                }
                SyncIntent::Identify => SyncCmd::Identify,
                SyncIntent::Job(job) => SyncCmd::Job(job.clone()),
                SyncIntent::Release { approve, hash } => SyncCmd::Release {
                    approve: *approve,
                    hash: *hash,
                },
                SyncIntent::OnboardingDone(secrets) => {
                    self.sync.identify_after = true;
                    SyncCmd::Onboard(Box::new(secrets.clone()))
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
    }

    fn apply_sync_event(&mut self, event: crate::sync::client::SyncEvent) {
        use crate::sync::client::SyncEvent;
        let sync = &mut self.sync;
        if matches!(event, SyncEvent::Notice(_)) {
            // A job may have imported the kit: look for the ring secret again.
            sync.ring_secret_missing = None;
        }
        match event {
            SyncEvent::Progress(line) => sync.progress = Some(line),
            SyncEvent::PendingArrived(n) => sync.pending += n,
            SyncEvent::SasPrompt(info) => {
                let peer = sync.enrolling.clone().unwrap_or_else(|| "peer".into());
                // A server nobody sits at has no words to compare: the question is whether this is your server,
                // and its SSH key fingerprint is what answers it.
                let ssh_fingerprint = (!info.peer_has_screen)
                    .then(|| host_fingerprint(&self.store, &peer))
                    .flatten();
                self.open_sync_sas(SasView {
                    peer,
                    peer_name: info.peer_name,
                    words: info.words,
                    peer_has_screen: info.peer_has_screen,
                    ssh_fingerprint,
                });
            }
            SyncEvent::Notice(message) if message.is_empty() => {}
            SyncEvent::Notice(message) if message.contains('\n') => {
                let lines = message.lines().map(str::to_string).collect();
                self.sync_info("Done", lines);
            }
            SyncEvent::Notice(message) => self.host_notice = Some(message),
            SyncEvent::ThisDevice(id) => {
                for d in &mut sync.devices {
                    d.this = d.id == id;
                }
                sync.this_device = Some(id);
            }
            SyncEvent::UnlockPrompt(text) => {
                if let Some(u) = sync.unlock.as_mut() {
                    set_path_status(u, UnlockPathKind::Phrase, text);
                }
            }
            SyncEvent::UnlockPath(kind, status) => {
                if let Some(u) = sync.unlock.as_mut() {
                    set_path_status(u, kind, status);
                }
            }
            SyncEvent::UnlockProgress { have, need } => {
                if let Some(u) = sync.unlock.as_mut() {
                    (u.have, u.need) = (have, need);
                }
            }
            SyncEvent::UnlockChallenge {
                frames,
                host_id,
                words,
            } => {
                sync.qr_frame = 0;
                if let Some(u) = sync.unlock.as_mut() {
                    u.frames = frames;
                    (u.host_id, u.words) = (host_id, words);
                }
            }
            SyncEvent::ForkDetected(id) => {
                let device = sync
                    .devices
                    .iter()
                    .find(|d| d.id == id)
                    .map_or_else(|| id.clone(), |d| d.name.clone());
                let evidence = self
                    .store
                    .sync_fork_evidence(&id)
                    .ok()
                    .flatten()
                    .map(|e| super::sync_view::evidence_lines(&e))
                    .unwrap_or_default();
                self.raise_sync_fork_alert(ForkView {
                    device,
                    detail: format!("device id {id}: two different histories under one identity"),
                    evidence,
                });
            }
            SyncEvent::BehindPeer(id) => {
                let name = sync
                    .devices
                    .iter()
                    .find(|d| d.id == id)
                    .map_or_else(|| id.clone(), |d| d.name.clone());
                let host = self
                    .profile
                    .as_ref()
                    .and_then(|p| crate::sync::runtime::Peers::load(p).ok())
                    .and_then(|p| p.host_of(&id).map(str::to_string));
                self.host_notice = Some(match &host {
                    Some(h) => format!(
                        "{name} missed some changes and could not prove it is current, so nothing moved. Select it and press Enter to add it again through {h} (it is approved again; what it may do does not change)"
                    ),
                    None => format!(
                        "{name} missed some changes and no host is known for it: select its host in the hosts tab and press A to add it again"
                    ),
                });
                self.sync.reenroll = host.map(|h| (id, h));
            }
            SyncEvent::Done => {
                sync.syncing = false;
                sync.progress = None;
                sync.enrolling = None;
                sync.unlock_running = false;
                let migrating = std::mem::take(&mut sync.migrating);
                // Whatever the round moved is in the store now.
                self.refresh_sync_view();
                if migrating {
                    self.sync_close_modal();
                }
                // A ring just came into being here: learn which device this is.
                if std::mem::take(&mut self.sync.identify_after) && self.sync.configured {
                    self.sync.intents.push(SyncIntent::Identify);
                }
            }
            SyncEvent::Error(message) => {
                sync.syncing = false;
                sync.progress = None;
                sync.enrolling = None;
                sync.unlock_running = false;
                let migrating = std::mem::take(&mut sync.migrating);
                if message == crate::sync::client::WORKER_STOPPED {
                    // The thread is gone: stop offering the keys that need it.
                    sync.worker = false;
                    self.sync_worker = None;
                }
                self.sync.last_error = Some(message.clone());
                if migrating {
                    self.sync_close_modal();
                }
                self.host_notice = Some(format!("sync: {message}"));
            }
        }
    }

    pub fn open_sync_release_confirm(&mut self, view: ReleaseView) {
        self.sync.release = Some(view);
        self.sync.release_opened = Some(std::time::Instant::now());
        self.mode = AppMode::SyncReleaseConfirm;
    }

    pub fn open_sync_onboarding(&mut self, view: OnboardingView) {
        self.sync.onboarding = Some(view);
        self.mode = AppMode::SyncOnboarding;
    }

    /// Vault first run: issue a master phrase and ring secret and walk the user
    /// through writing them down and printing the kit.
    pub fn start_vault_onboarding(&mut self) -> Result<()> {
        let view = OnboardingView::issue()?;
        self.open_sync_onboarding(view);
        Ok(())
    }

    /// One key of the Vault onboarding: show, retype two words, write the kit,
    /// finish. Nothing leaves this screen except the final intent.
    fn onboarding_key(&mut self, key: KeyEvent) {
        use crate::app::kit;
        enum Next {
            Stay,
            WriteKit,
            Finish,
        }
        let Some(o) = self.sync.onboarding.as_mut() else {
            return;
        };
        let editing = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        let mut next = Next::Stay;
        match o.step {
            OnboardingStep::Show => {
                if key.code == KeyCode::Enter {
                    o.step = OnboardingStep::Retype;
                    o.typed.clear();
                    o.notice = None;
                }
            }
            OnboardingStep::Retype => match key.code {
                KeyCode::Enter => {
                    let (n, want) = (o.quiz[o.quiz_done.min(1)] + 1, o.quiz_word().to_string());
                    if o.typed.trim().eq_ignore_ascii_case(&want) {
                        o.quiz_done += 1;
                        o.notice = None;
                        if o.quiz_done >= 2 {
                            o.step = OnboardingStep::Kit;
                        }
                    } else {
                        o.notice = Some(format!("that is not word {n}: check the phrase on paper"));
                    }
                    o.typed.clear();
                }
                // Back to the words, before anything is printed.
                KeyCode::Esc => {
                    o.step = OnboardingStep::Show;
                    o.quiz_done = 0;
                    o.typed.clear();
                    o.notice = None;
                }
                KeyCode::Backspace => {
                    o.typed.pop();
                }
                KeyCode::Char(c) if editing && !c.is_control() => o.typed.push(c),
                _ => {}
            },
            OnboardingStep::ShowRing => {
                if key.code == KeyCode::Enter {
                    next = Next::Finish;
                } else if key.code == KeyCode::Esc {
                    o.step = OnboardingStep::Kit;
                }
            }
            OnboardingStep::Kit
                if key.code == KeyCode::Char('s') && key.modifiers == KeyModifiers::CONTROL =>
            {
                // No printer: show the ring secret to copy by hand. The master phrase was written already.
                o.notice = None;
                o.step = OnboardingStep::ShowRing;
            }
            OnboardingStep::Kit => {
                let field = if o.path_field == 0 {
                    &mut o.phrase_path
                } else {
                    &mut o.ring_path
                };
                match key.code {
                    KeyCode::Enter => next = Next::WriteKit,
                    KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                        o.path_field ^= 1;
                    }
                    KeyCode::Backspace => {
                        field.pop();
                    }
                    KeyCode::Char(c) if editing && !c.is_control() => field.push(c),
                    _ => {}
                }
            }
            OnboardingStep::Done => {
                if key.code == KeyCode::Enter {
                    next = Next::Finish;
                } else if key.code == KeyCode::Char('d') && key.modifiers.is_empty() {
                    // After printing: take the two files off the disk now.
                    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
                    let expand = |raw: &str| match (raw.trim().strip_prefix("~/"), &home) {
                        (Some(rest), Some(home)) => home.join(rest),
                        _ => std::path::PathBuf::from(raw.trim()),
                    };
                    let failed: Vec<String> = [&o.phrase_path, &o.ring_path]
                        .into_iter()
                        .filter_map(|p| {
                            let p = expand(p);
                            kit::wipe_file(&p)
                                .err()
                                .map(|e| format!("{}: {e}", p.display()))
                        })
                        .collect();
                    o.notice = Some(if failed.is_empty() {
                        "Both files are deleted. Only the paper is left.".to_string()
                    } else {
                        failed.join("; ")
                    });
                }
            }
        }
        match next {
            Next::Stay => {}
            Next::WriteKit => self.write_onboarding_kit(),
            Next::Finish => {
                if let Some(o) = self.sync.onboarding.as_ref() {
                    let secrets = o.secrets().clone();
                    self.sync.intents.push(SyncIntent::OnboardingDone(secrets));
                }
                self.sync_close_modal();
            }
        }
    }

    /// Write the two sheets to the chosen paths, or say why not and write
    /// neither: no overwrite, no shared path, no missing folder.
    fn write_onboarding_kit(&mut self) {
        use crate::app::kit;
        let Some(o) = self.sync.onboarding.as_mut() else {
            return;
        };
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let expand = |raw: &str| {
            let raw = raw.trim();
            match (raw.strip_prefix("~/"), &home) {
                (Some(rest), Some(home)) => home.join(rest),
                _ => std::path::PathBuf::from(raw),
            }
        };
        let (phrase_path, ring_path) = (expand(&o.phrase_path), expand(&o.ring_path));
        let problem = if o.phrase_path.trim().is_empty() || o.ring_path.trim().is_empty() {
            Some("choose a path for both sheets".to_string())
        } else if phrase_path == ring_path {
            Some("the two sheets need two different paths".to_string())
        } else {
            [&phrase_path, &ring_path].into_iter().find_map(|p| {
                if p.exists() {
                    Some(format!(
                        "{} already exists: choose another path",
                        p.display()
                    ))
                } else if p
                    .parent()
                    .is_some_and(|d| !d.as_os_str().is_empty() && !d.is_dir())
                {
                    Some(format!("{}: that folder does not exist", p.display()))
                } else {
                    None
                }
            })
        };
        if let Some(problem) = problem {
            o.notice = Some(problem);
            return;
        }
        let phrase_html = zeroize::Zeroizing::new(kit::phrase_sheet(&o.secrets.phrase));
        let ring_html = zeroize::Zeroizing::new(kit::ring_sheet(&o.secrets.ring_secret));
        if let Err(e) = kit::write_sheet(&phrase_path, &phrase_html) {
            o.notice = Some(format!("{}: {e}", phrase_path.display()));
            return;
        }
        if let Err(e) = kit::write_sheet(&ring_path, &ring_html) {
            // Never leave half a kit behind.
            let _ = std::fs::remove_file(&phrase_path);
            o.notice = Some(format!("{}: {e}", ring_path.display()));
            return;
        }
        o.notice = None;
        o.step = OnboardingStep::Done;
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
        if self.sync.unlock.is_some() && self.sync.unlock_running {
            if let Some(w) = &self.sync_worker {
                let _ = w.send(crate::sync::client::SyncCmd::UnlockCancel);
            }
        }
        let sync = &mut self.sync;
        sync.unlock = None;
        sync.sas = None;
        sync.release = None;
        sync.onboarding = None;
        // Closing the modal abandons the action that was waiting on it.
        self.unlock_resume = None;
        self.mode = AppMode::Normal;
    }

    /// The gate before any path that reads credentials. `true`: nothing it
    /// needs is locked, go ahead. `false`: it is blocked and `resume` will run
    /// by itself once the credentials are open (modal and request to the sync
    /// stage), or the user was told why it cannot (no sync worker to open them).
    pub(crate) fn preflight_unlock(
        &mut self,
        refs: Vec<crate::credentials::SecretRef>,
        label: &str,
        resume: impl FnOnce(&mut App) -> Result<()> + 'static,
    ) -> bool {
        let locked = crate::credentials::locked_refs(self.password_store.as_ref(), &refs);
        if locked.is_empty() {
            return true;
        }
        if !self.sync.worker {
            self.host_notice = Some(format!(
                "{label}: {}",
                crate::credentials::needs_share_message()
            ));
            return false;
        }
        let secrets = locked
            .iter()
            .filter_map(|r| match r {
                crate::credentials::SecretRef::Key(k) => Some(k.clone()),
                crate::credentials::SecretRef::Lookup(_)
                | crate::credentials::SecretRef::Staged(_)
                | crate::credentials::SecretRef::Handle(_) => None,
            })
            .collect();
        self.unlock_resume = Some(UnlockResume {
            refs: locked,
            run: Box::new(resume),
        });
        self.open_sync_unlock(UnlockView {
            host: label.into(),
            have: 0,
            need: 0,
            paths: Vec::new(),
            frames: Vec::new(),
            host_id: String::new(),
            words: String::new(),
            code: String::new(),
        });
        self.sync.intents.push(SyncIntent::Unlock { secrets });
        false
    }

    /// Whether `tunnel` would read a synced credential that is still locked.
    /// Cheap when nothing is synced: it asks the gate before touching the
    /// store. Used by the paths that start tunnels with no one at the keyboard.
    pub(crate) fn tunnel_credentials_locked(&self, tunnel: &crate::store::Tunnel) -> bool {
        if !self.password_store.gate().is_some_and(|g| g.guards_any()) {
            return false;
        }
        let host = tunnel
            .host_id
            .and_then(|id| self.store.get_host(id).ok().flatten());
        let resolved = host
            .as_ref()
            .and_then(|h| self.store.resolve_connection(h).ok());
        let refs = crate::app::connection_secret_refs(
            &self.store,
            &self.hosts,
            host.as_ref(),
            resolved.as_ref().and_then(|r| r.identity.as_ref()),
            resolved.as_ref().and_then(|r| r.proxy_jump.as_deref()),
        );
        !crate::credentials::locked_refs(self.password_store.as_ref(), &refs).is_empty()
    }

    /// Per tick: run the action that waited on an unlock once its credentials
    /// are open.
    pub(crate) fn poll_unlock_resume(&mut self) {
        let ready = self.unlock_resume.as_ref().is_some_and(|r| {
            crate::credentials::locked_refs(self.password_store.as_ref(), &r.refs).is_empty()
        });
        if !ready {
            return;
        }
        let Some(resume) = self.unlock_resume.take() else {
            return;
        };
        if self.mode == AppMode::SyncUnlock {
            self.sync_close_modal();
        }
        if let Err(e) = (resume.run)(self) {
            self.host_notice = Some(format!("{e:#}"));
        }
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
            AppMode::SyncRecoveryAlert => {
                // Esc (or Enter) only closes the notice for the recoveries it names; a new one raises it again.
                if self.is_action(KeyAction::ForceQuit, &key) {
                    self.should_quit = true;
                } else if cancel || confirm {
                    if let Some(a) = &self.sync.alert {
                        let hashes = a.hashes.clone();
                        self.sync.alert_dismissed.extend(hashes);
                    }
                    self.mode = self.sync.alert_return.take().unwrap_or(AppMode::Normal);
                }
            }
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
                } else if key.code == KeyCode::Char('A') {
                    self.sync_approve_group();
                } else if let KeyCode::Char(c @ ('a' | 'r' | 'p' | 'e')) = key.code {
                    if key.modifiers.is_empty() {
                        self.sync_review_key(c);
                    }
                }
            }
            AppMode::SyncUnlock => {
                if cancel {
                    self.sync_close_modal();
                } else if self.is_action(KeyAction::SyncBleDrop, &key) {
                    // Only where a Bluetooth channel is running (its row is in the modal).
                    if self.sync.unlock.as_ref().is_some_and(|u| {
                        u.paths.iter().any(|p| p.kind == UnlockPathKind::Bluetooth)
                    }) {
                        self.sync.intents.push(SyncIntent::UnlockCode(
                            crate::sync::ui_shares::DROP_CENTRAL_ANSWER.to_string(),
                        ));
                    }
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
                } else if plain(' ') {
                    super::sync_trust::toggle_mark(self);
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
                    self.mode = AppMode::Normal;
                    super::sync_trust::begin(
                        self,
                        TRUST_TTL_DAYS[self.sync.trust_ttl_selected],
                        self.sync.trust_budget,
                    );
                } else if down {
                    self.sync.trust_ttl_selected =
                        (self.sync.trust_ttl_selected + 1).min(TRUST_TTL_DAYS.len() - 1);
                } else if up {
                    self.sync.trust_ttl_selected = self.sync.trust_ttl_selected.saturating_sub(1);
                } else if key.code == KeyCode::Right {
                    self.sync.trust_budget = (self.sync.trust_budget + 1)
                        .min(crate::sync::limits::CERT_NEW_HOSTS_PER_DAY);
                } else if key.code == KeyCode::Left {
                    self.sync.trust_budget = self.sync.trust_budget.saturating_sub(1).max(1);
                }
            }
            AppMode::SyncReleaseConfirm => {
                // A modal that opens while the user is pressing Enter must not approve by itself:
                // keys are ignored at first, and only an explicit `y` releases.
                if self
                    .sync
                    .release_opened
                    .is_some_and(|t| t.elapsed() < RELEASE_KEY_DELAY)
                {
                    return Ok(());
                }
                let approve = plain('y');
                if approve || cancel || plain('n') {
                    if let Some(hash) = self.sync.release.as_ref().map(|r| r.hash) {
                        self.sync.answered_releases.push(hash);
                        self.sync
                            .intents
                            .push(SyncIntent::Release { approve, hash });
                    }
                    self.sync_close_modal();
                }
            }
            AppMode::SyncOnboarding => self.onboarding_key(key),
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
        verified_by: Some(crate::sync::oplog::VerifiedBy::Sas),
        this: false,
        phone: false,
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
            hash: [0; 32],
            reason: None,
            red: None,
            author: String::new(),
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
        trust_scope_values: vec![
            crate::sync::oplog::CertScope::Group([1; 16]),
            crate::sync::oplog::CertScope::Group([2; 16]),
        ],
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
            host_id: "A3F9 007C 1B2E 55D0 9C41 7E08 F2A6 3D19".into(),
            words: "harbor-globe-lantern-mint".into(),
            code: "ab-12".into(),
        }),
        sas: Some(SasView {
            peer: "laptop".into(),
            peer_name: "laptop".into(),
            words: "harbor-globe-lantern-mint".into(),
            peer_has_screen: true,
            ssh_fingerprint: None,
        }),
        fork: Some(ForkView {
            device: "server".into(),
            detail: "two different ops at seq 9".into(),
            evidence: vec!["op 1a2b3c4d and op 5e6f7a8b at seq 9".into()],
        }),
        release: Some(ReleaseView {
            host: "web-1".into(),
            requester: "new laptop".into(),
            requester_id: "id-9".into(),
            hash: [7; 32],
            age_secs: 12,
        }),
        onboarding: Some(
            OnboardingView::with_secrets(
                &[7u8; 10],
                [9u8; 16],
                [1, 4],
                ("/tmp/sheet-a.html".into(), "/tmp/sheet-b.html".into()),
            )
            .unwrap(),
        ),
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

    fn unlock_with(paths: &[UnlockPathKind]) -> App {
        let mut app = configured_app();
        app.open_sync_unlock(UnlockView {
            host: "web-1".into(),
            have: 1,
            need: 2,
            paths: paths
                .iter()
                .map(|k| UnlockPathView {
                    kind: *k,
                    status: "x".into(),
                })
                .collect(),
            frames: vec![],
            host_id: String::new(),
            words: String::new(),
            code: String::new(),
        });
        app
    }

    fn ctrl_b() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)
    }

    #[test]
    fn the_drop_phone_key_reaches_the_worker_only_while_a_bluetooth_channel_runs() {
        let mut app = unlock_with(&[UnlockPathKind::PhoneQr, UnlockPathKind::Bluetooth]);
        app.handle_key(ctrl_b()).unwrap();
        assert_eq!(
            app.sync.intents,
            vec![SyncIntent::UnlockCode(
                crate::sync::ui_shares::DROP_CENTRAL_ANSWER.to_string()
            )]
        );
        assert_eq!(app.mode, AppMode::SyncUnlock, "the modal stays open");
        assert!(
            app.sync.unlock.as_ref().unwrap().code.is_empty(),
            "nothing typed"
        );
        // No Bluetooth row (not built in, or not armed): the key does nothing.
        let mut app = unlock_with(&[UnlockPathKind::PhoneQr]);
        app.handle_key(ctrl_b()).unwrap();
        assert!(app.sync.intents.is_empty());
    }

    #[test]
    fn the_drop_phone_key_is_not_a_text_key_in_the_phone_enrollment_prompt() {
        let mut app = configured_app();
        app.sync_prompt("Add my phone", "frame", false, |_, _| {});
        app.handle_key(ctrl_b()).unwrap();
        // The prompt is still there, nothing was typed, and with no Bluetooth channel nothing is dropped.
        assert_eq!(app.mode, AppMode::SyncPrompt);
        assert!(app.host_notice.is_none());
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
        app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT))
            .unwrap();
        assert_eq!(app.host_notice.as_deref(), Some(NO_WORKER));
        app.sync.worker = true;
        app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT))
            .unwrap();
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
        assert_eq!(text, "\u{21c5}1 \u{23f3}2");
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
        // The sample's pending op is not in the store: resolving it says so.
        app.handle_key(key('a')).unwrap();
        assert!(app
            .host_notice
            .as_deref()
            .is_some_and(|n| n.starts_with("sync:")));
        assert_eq!(app.sync.intents.len(), 1, "review never goes to the worker");
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
        // The term is chosen: the budget is asked next (the sample's scopes are
        // labels only, so the flow stops there with a notice).
        assert!(app.mode == AppMode::Normal || app.mode == AppMode::SyncPrompt);
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
            host_id: String::new(),
            words: String::new(),
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
            evidence: Vec::new(),
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
            peer_name: "laptop".into(),
            words: "harbor-globe-lantern-mint".into(),
            peer_has_screen: true,
            ssh_fingerprint: None,
        });
        app.handle_key(press(KeyCode::Enter)).unwrap();
        app.open_sync_release_confirm(ReleaseView {
            host: "web-1".into(),
            requester: "new laptop".into(),
            requester_id: "id-9".into(),
            hash: [7; 32],
            age_secs: 3,
        });
        age_the_release_modal(&mut app);
        app.handle_key(press(KeyCode::Esc)).unwrap();
        assert_eq!(
            app.sync.intents,
            vec![
                SyncIntent::SasMatches(true),
                SyncIntent::Release {
                    approve: false,
                    hash: [7; 32]
                },
            ]
        );
        assert_eq!(app.mode, AppMode::Normal);
    }

    fn age_the_release_modal(app: &mut App) {
        app.sync.release_opened =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(2));
    }

    fn release_modal() -> App {
        let mut app = configured_app();
        app.open_sync_release_confirm(ReleaseView {
            host: "web-1".into(),
            requester: "new laptop".into(),
            requester_id: "id-9".into(),
            hash: [7; 32],
            age_secs: 3,
        });
        app
    }

    #[test]
    fn a_release_needs_an_explicit_y_after_the_first_moments() {
        // Keys pressed as the modal opens (even `y`) are ignored.
        let mut app = release_modal();
        app.handle_key(press(KeyCode::Char('y'))).unwrap();
        assert_eq!(app.mode, AppMode::SyncReleaseConfirm);
        assert!(app.sync.intents.is_empty());
        // Enter never approves, even later.
        age_the_release_modal(&mut app);
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(app.mode, AppMode::SyncReleaseConfirm);
        assert!(app.sync.intents.is_empty());
        // An explicit `y` does.
        app.handle_key(press(KeyCode::Char('y'))).unwrap();
        assert_eq!(
            app.sync.intents,
            vec![SyncIntent::Release {
                approve: true,
                hash: [7; 32]
            }]
        );
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
        // The round waits until the assert below has seen "running": without the gate a loaded
        // machine finishes it inside the same poll and the intermediate state is gone.
        let go = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let gate = go.clone();
        let mut app = app_with_worker(Box::new(move |_k, device, ctx| {
            assert_eq!(device, "id-server");
            let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !gate.load(std::sync::atomic::Ordering::SeqCst) && std::time::Instant::now() < end
            {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            ctx.emit(SyncEvent::Progress("talking".into()));
            Ok(RoundSummary {
                pending_arrived: 3,
                ..Default::default()
            })
        }));
        app.sync.intents.push(SyncIntent::SyncNow {
            device: "id-server".into(),
        });
        app.poll_sync();
        assert!(app.sync.syncing, "a request marks the round as running");
        assert!(app.sync.intents.is_empty());
        go.store(true, std::sync::atomic::Ordering::SeqCst);
        pump(&mut app, |a| !a.sync.syncing);
        // The round is done: the count is re-read from the store (empty here).
        assert_eq!(app.sync.pending, 0);
        app.apply_sync_event(crate::sync::client::SyncEvent::PendingArrived(3));
        assert_eq!(app.sync.pending, 3, "arrived ops add to the count");
        assert_eq!(app.sync.progress, None);
        assert_eq!(app.sync.last_error, None);
    }

    #[test]
    fn enrollment_shows_the_sas_naming_the_host_and_the_answer_goes_back() {
        use crate::sync::client::{Round, RoundSummary, SyncCmd, SyncEvent};
        let mut app = app_with_worker(Box::new(|kind, _d, ctx| {
            assert_eq!(kind, Round::Enroll);
            ctx.emit(SyncEvent::SasPrompt(crate::sync::client::SasPromptInfo {
                words: "harbor-globe-lantern-mint".into(),
                peer_name: "laptop".into(),
                peer_has_screen: true,
            }));
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
                (|a: &App| {
                    a.host_notice
                        .as_deref()
                        .is_some_and(|n| n.starts_with("laptop missed some changes"))
                }) as fn(&App) -> bool,
            ),
            ("fork", |a: &App| a.mode == AppMode::SyncForkAlert),
            ("fail", |a: &App| a.sync.last_error.is_some()),
        ] {
            // Each finished round re-reads the (empty) store: put the sample back.
            app.sync.devices = sample_sync_view().devices;
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
    fn unlock_path_events_fill_the_modal_per_path() {
        use crate::sync::client::{RoundSummary, SyncEvent};
        let mut app = app_with_worker(Box::new(|_k, _d, ctx| {
            ctx.emit(SyncEvent::UnlockPrompt("type the words".into()));
            ctx.emit(SyncEvent::UnlockPath(
                UnlockPathKind::Peer,
                "asked laptop".into(),
            ));
            ctx.emit(SyncEvent::UnlockPath(
                UnlockPathKind::Peer,
                "laptop is asking you".into(),
            ));
            ctx.emit(SyncEvent::UnlockProgress { have: 1, need: 2 });
            ctx.emit(SyncEvent::UnlockChallenge {
                frames: vec![vec![1, 2, 3]],
                host_id: "A3F9 007C".into(),
                words: "harbor-globe-lantern-mint".into(),
            });
            Ok(RoundSummary::default())
        }));
        app.open_sync_unlock(UnlockView {
            host: "web".into(),
            have: 0,
            need: 0,
            paths: Vec::new(),
            frames: Vec::new(),
            host_id: String::new(),
            words: String::new(),
            code: String::new(),
        });
        app.sync.intents.push(SyncIntent::SyncNow {
            device: "id-server".into(),
        });
        app.poll_sync();
        pump(&mut app, |a| !a.sync.syncing);
        let u = app.sync.unlock.as_ref().unwrap();
        assert_eq!((u.have, u.need), (1, 2));
        assert_eq!(u.frames, vec![vec![1, 2, 3]]);
        assert_eq!(
            (u.host_id.as_str(), u.words.as_str()),
            ("A3F9 007C", "harbor-globe-lantern-mint")
        );
        let status = |k| {
            u.paths
                .iter()
                .find(|p| p.kind == k)
                .map(|p| p.status.as_str())
        };
        assert_eq!(status(UnlockPathKind::Phrase), Some("type the words"));
        assert_eq!(status(UnlockPathKind::Peer), Some("laptop is asking you"));
        assert_eq!(u.paths.len(), 2, "a path is updated, not repeated");
    }

    #[test]
    fn a_behind_peer_offers_a_guided_re_enroll_through_its_recorded_host() {
        use crate::sync::client::SyncEvent;
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::profile::ProfilePaths {
            data_root: dir.path().to_path_buf(),
            id: "t".into(),
            name: "t".into(),
            root: dir.path().to_path_buf(),
            config_file: dir.path().join("config.toml"),
            ssh_config: dir.path().join("ssh_config"),
            compat: false,
        };
        let mut app = configured_app();
        app.profile = Some(paths.clone());
        app.active_tab = 6;
        app.sync.devices[1].status = DeviceStatusView::Behind;
        app.sync.selected = 1;
        let id = app.sync.devices[1].id.clone();

        // No host recorded: the way out is the hosts tab.
        app.apply_sync_event(SyncEvent::BehindPeer(id.clone()));
        assert!(app.sync.reenroll.is_none());
        assert!(app.host_notice.as_deref().unwrap().contains("hosts tab"));

        crate::sync::runtime::Peers::record(&paths, &id, "hub").unwrap();
        app.apply_sync_event(SyncEvent::BehindPeer(id.clone()));
        assert_eq!(app.sync.reenroll, Some((id.clone(), "hub".into())));
        assert!(app.host_notice.as_deref().unwrap().contains("through hub"));

        app.sync.intents.clear();
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(
            app.sync.intents,
            vec![SyncIntent::Enroll { host: "hub".into() }]
        );

        // Without a round having said so, Enter syncs first.
        app.sync.intents.clear();
        app.sync.reenroll = None;
        app.handle_key(press(KeyCode::Enter)).unwrap();
        assert_eq!(app.sync.intents, vec![SyncIntent::SyncNow { device: id }]);
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
        app.sync.intents.push(SyncIntent::UnlockCode("x".into()));
        app.poll_sync();
        assert_eq!(app.sync.intents, vec![SyncIntent::UnlockCode("x".into())]);
    }
}
