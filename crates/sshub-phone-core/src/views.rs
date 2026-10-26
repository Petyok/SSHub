//! The records that cross the uniffi boundary. Everything a person reads is built here from the SIGNED BYTES of the
//! frame that arrived (the screen is the bytes: there is no description field anywhere); Kotlin renders these records
//! and never parses an op, compares a hash or formats "what you are approving".
//!
//! Flow: `PhoneCore::open_frame(frame, now)` -> [`Opened`] (a view and a `token`), the person looks, then
//! `PhoneCore::decide(token, Decision, now)` -> [`Outgoing`] (the bytes to show or send). A refusal is a VALUE
//! ([`Opened::Refused`]), never an error. `CoreError` means the call itself could not run.

/// One line of an "approve this" screen. `danger` rows are highlighted.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Row {
    pub label: String,
    pub value: String,
    pub danger: bool,
}

/// A line of a checks list: a rule that passed (`warn` = passed with a warning to read).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Check {
    pub text: String,
    pub warn: bool,
}

/// A signed op rendered from its bytes: `headline`, then every leaf of the body as a row (nothing summarised away
/// except long base64 blobs, shown with length and prefix).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct OpView {
    pub kind: String,
    pub headline: String,
    /// The device named as requester / author (id).
    pub requester: Option<String>,
    pub rows: Vec<Row>,
    pub danger_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RingEntry {
    pub device_id: String,
    pub name: String,
}

/// What the home screen shows. Returned by `unlock` and `status`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct UnlockInfo {
    pub device_id: String,
    pub enrolled: bool,
    pub head_pending: bool,
    /// The stored state did not open with this key (damaged, or sealed under another key): the phone starts as a
    /// restored one and must take its chain from a terminal.
    pub state_lost: bool,
    /// Seq of the newest op of the phone's own chain, if it has one.
    pub head_seq: Option<u64>,
    pub wrap_epoch: u32,
    pub live_short_codes: u32,
    pub ring: Vec<RingEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AudioProfile {
    /// About 8 s for a 128 byte reply.
    Fast,
    /// About 16 s, for noisy rooms.
    Slow,
}

/// One QR code of a looping animation: `modules` is `size * size` bytes, row major, 1 = dark. No quiet zone: the
/// screen draws a white margin of four modules and whole device pixels per module.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct QrFrame {
    pub size: u32,
    pub modules: Vec<u8>,
}

/// The reply by hand: the share index and its 24 words.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ManualEntry {
    pub index: u32,
    pub words: String,
}

/// A short code issued with a release (valid 7 days, once per credential). Shown once; never stored.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct IssuedCode {
    pub code: String,
    pub valid_until: i64,
    pub argon_mem_mib: u32,
    pub argon_iters: u32,
    /// Below the usual floor because this phone is slow.
    pub weak: bool,
}

/// Bytes to show or send after a decision. `frame` is the whole frame (every channel carries exactly these bytes);
/// the rest is pre-rendered for the channels the person left on.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Outgoing {
    pub kind: u8,
    pub frame: Vec<u8>,
    /// Base64 of `frame` (clipboard, share sheet, paste into a terminal without a camera).
    pub text: String,
    /// Looping reverse QR (fountain frames of the page's size, 25 % parity, 4 fps). Empty when not asked for or when
    /// the frame is too long for an animation (`qr_too_big`: use the text).
    pub qr: Vec<QrFrame>,
    pub qr_too_big: bool,
    /// Bytes for `PhoneCore::audio_burst`: the share alone (a longer reply does not fit a 2 to 16 s burst).
    pub audio_frame: Option<Vec<u8>>,
    pub manual: Option<ManualEntry>,
    pub short_code: Option<IssuedCode>,
    /// A sentence for the screen (what to do with these bytes).
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ShortCodeEntry {
    pub host_uid: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub live: bool,
    pub argon_mem_mib: u32,
    pub argon_iters: u32,
    pub weak: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CertStatus {
    Active,
    Expired,
    Revoked,
}

/// A trust session this phone issued, read back from its own chain.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct IssuedCertView {
    /// Hex of the `CertIssue` op hash.
    pub cert_id: String,
    /// The one line description (device, scope, term, budget, extensions).
    pub summary: String,
    pub status: CertStatus,
    pub can_revoke: bool,
}

// ---------------------------------------------------------------- Opened

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RefusalView {
    pub reason: String,
    pub checks: Vec<Check>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ShortCodeOffer {
    Available,
    /// A short code for this credential is already live: a second one is refused by the phone.
    LiveUntil { expires_at: i64 },
}

/// Kind 1: a terminal asks for this phone's share of a credential.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ShareRequestView {
    pub token: u64,
    pub op: OpView,
    pub host_uid: String,
    pub requester: String,
    /// The four words the terminal shows too: the person compares them.
    pub binding_words: String,
    pub binding_hash: String,
    pub checks: Vec<Check>,
    pub expires_at: i64,
    /// Set for a requester this phone never met: Approve needs this id typed (`Decision::Release.typed_id`).
    pub type_to_approve: Option<String>,
    pub short_code: ShortCodeOffer,
}

/// Kind 3 carrying a Red operation that needs this phone's co-signature.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RedNoticeView {
    pub token: u64,
    pub op: OpView,
    pub op_hash: String,
    pub banner: String,
    /// "The signature of the proposer verifies." or why it was not checked.
    pub signature_note: String,
    /// Why Approve is not offered (a certificate written by another device), if so.
    pub blocked: Option<String>,
    /// Set for a proposer this phone never met: Approve needs this id typed.
    pub type_to_approve: Option<String>,
}

/// Kind 3 carrying a `PhoneReenroll`: a recovery that this phone (still alive) can cancel.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ReenrollNoticeView {
    pub token: u64,
    pub op: OpView,
    pub op_hash: String,
    /// When the ring accepts the new phone unless it is cancelled.
    pub ends_at: i64,
    pub signature_note: String,
}

/// Kind 9: a terminal asks this phone to issue a trust session.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CertRequestView {
    pub token: u64,
    pub headline: String,
    pub rows: Vec<Row>,
    pub delegate_known: bool,
    /// Refusals: when not empty Approve is not offered.
    pub problems: Vec<String>,
    pub note: String,
    /// Set for a delegate this phone never met: Approve needs this id typed.
    pub type_to_approve: Option<String>,
}

/// Kind 10: a terminal asks for more new hosts on a certificate this phone issued.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CertExtendView {
    pub token: u64,
    pub headline: String,
    pub rows: Vec<Row>,
    pub problems: Vec<String>,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ContributorView {
    pub device_id: String,
    pub name: String,
    /// The signature of its batch verified under a key this phone enrolled with.
    pub verified: bool,
    pub known: bool,
    /// Its session key for this phone opened.
    pub has_session: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HolderView {
    pub device_id: String,
    pub name: String,
    pub is_this_phone: bool,
}

/// Kind 5 from a terminal carrying a reshare for this phone.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ReshareView {
    pub token: u64,
    pub epoch: u64,
    pub holders: u32,
    pub my_index: u32,
    pub credentials_ready: u32,
    /// Credentials still waiting for a second contributor.
    pub waiting: u32,
    pub contributors: Vec<ContributorView>,
    pub holders_after: Vec<HolderView>,
    /// A storage key of this epoch is published with the new shares (false: one was already published).
    pub storage_key_new: bool,
}

/// Head adoption after a restore (kind 5 list of the phone's own ops). Two lists naming the same newest op are
/// needed (`agreed`); then `decide(token, Decision::Approve{..})` accepts the head.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ChainAdopted {
    pub token: u64,
    pub agreed: bool,
    pub head_seq: u64,
    pub newest_at: i64,
    pub ops: u32,
    /// The seq the earlier, disagreeing list ended at.
    pub previous_seq: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum EnrollStep {
    /// The terminal's commitment (kind 6) is in; the person names this phone (`Decision::EnrollName`).
    Commit,
    /// The terminal revealed its keys (kind 7): compare `sas_words` with the terminal (`Decision::EnrollSas`);
    /// `outgoing` carries this phone's reveal to show to the terminal.
    Reveal,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EnrollView {
    pub token: u64,
    pub step: EnrollStep,
    pub terminal_id: Option<String>,
    pub terminal_name: Option<String>,
    pub sas_words: Option<String>,
    pub outgoing: Option<Outgoing>,
}

/// Kind 8 from the terminal closed the enrollment: the terminal is in this phone's ring.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EnrolledView {
    pub terminal_id: String,
    pub terminal_name: String,
    pub phone_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Opened {
    Refused { view: RefusalView },
    ShareRequest { view: ShareRequestView },
    RedNotice { view: RedNoticeView },
    ReenrollNotice { view: ReenrollNoticeView },
    CertRequest { view: CertRequestView },
    CertExtend { view: CertExtendView },
    Reshare { view: ReshareView },
    Chain { view: ChainAdopted },
    Enroll { view: EnrollView },
    Enrolled { view: EnrolledView },
}

// ---------------------------------------------------------------- Decision

/// What the person chose on a share request.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ReleaseOptions {
    /// The requester's device id typed in, when the view asked for it.
    pub typed_id: Option<String>,
    pub qr: bool,
    pub clipboard: bool,
    pub sound: bool,
    pub audio_profile: AudioProfile,
    /// Also issue a short code (repeat access for 7 days); ignored when one is live.
    pub issue_short_code: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Decision {
    /// Drop the pending request.
    Refuse,
    /// Approve a share request.
    Release { options: ReleaseOptions },
    /// Approve a Red notice, a recovery cancellation, a trust-session request or extension, a reshare, or accept an
    /// agreed chain head. `typed_id` is the device id typed in when the view asked for it.
    Approve { typed_id: Option<String> },
    /// Name this phone for the ring (enrollment step 1).
    EnrollName { name: String },
    /// The four words are (not) identical (enrollment step 3).
    EnrollSas { matches: bool },
}

// ---------------------------------------------------------------- channels

/// What `scan_gray` saw in a camera frame.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScanEvent {
    Progress { have: u32, total: u32, damaged: u32 },
    /// A completed document (a whole frame of kinds 1 to 10): hand it to `open_frame`.
    Document { bytes: Vec<u8> },
}
