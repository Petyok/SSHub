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

/// A line of a checks list: a rule that passed (`warn`: passed with a warning to read) or, with `ok == false`, the rule
/// that refused (always the last line of a refusal).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Check {
    pub text: String,
    pub warn: bool,
    pub ok: bool,
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
    /// Hex of the BLAKE3 of the op's canonical bytes (the op hash the ring and the terminal show): compare it with the
    /// terminal.
    pub hash: String,
    /// One plain sentence for a person to decide on, in the words of docs/host-sync-glossary.md ("Make laptop-2 a full
    /// device: ... Asked by laptop-2."). No op names, lamport or seq: those are in `rows` as technical details.
    pub plain: String,
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
    /// The name this phone gave itself when it enrolled (None before the first enrollment).
    pub device_name: Option<String>,
    /// The highest epoch this phone has seen on its own chain (what a certificate request must start at).
    pub ring_epoch: u64,
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
    /// The trusted device and the name this phone knows it by (escaped).
    pub device_id: String,
    pub device_name: Option<String>,
    /// Scope: either one group (`group` set, `hosts` empty) or a list of hosts (uid hex each).
    pub group: Option<String>,
    pub hosts: Vec<String>,
    /// The signed time of the `CertIssue` op, the expiry it names and the term between them (seconds).
    pub issued_at: i64,
    pub expiry: i64,
    pub term_secs: i64,
    /// `term_secs` in the page's words ("30 days", "36 h 5 min").
    pub term_text: String,
    pub max_new_hosts_per_day: u32,
    pub epoch_lo: u64,
    pub epoch_hi: u64,
    pub revoked: bool,
    pub extensions: Vec<CertExtensionView>,
}

/// One `CertExtend` this phone signed for a certificate: the signed time and the extra new hosts it allowed.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CertExtensionView {
    pub at: i64,
    pub extra_hosts: u32,
}

// ---------------------------------------------------------------- Opened

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RefusalView {
    /// Why the frame was refused, for enrollment frames (`Other` for every other kind of frame): the app picks its
    /// words by this, not by matching `reason`.
    pub kind: RefusalKind,
    /// For `RefusalKind::Mismatch`: the two values that differ (the code the terminal showed first, and the code its
    /// keys open to), each as a label and the hex of its first 16 bytes grouped by four; empty for every other refusal.
    pub compared: Vec<Row>,
    pub reason: String,
    pub checks: Vec<Check>,
}

/// The reason class of an enrollment refusal (`RefusalView.kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RefusalKind {
    /// Not an enrollment refusal.
    Other,
    /// The terminal reveal does not open the commitment it showed first: someone is interfering. Enrollment stopped.
    Mismatch,
    /// The terminal declined the enrollment (its confirmation says no). Enrollment stopped.
    Declined,
    /// An enrollment frame at the wrong step (or the wrong role); the enrollment goes on where it was.
    WrongOrder,
    /// The terminal confirmation signature is bad. Enrollment stopped.
    BadSignature,
    /// The terminal presented keys that cannot be used. Enrollment stopped.
    UnusableKeys,
    /// A document that is not an enrollment frame (damaged, or another kind of QR); the enrollment goes on.
    StrayFrame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ShortCodeOffer {
    Available,
    /// A short code for this credential is already live: a second one is refused by the phone.
    LiveUntil {
        expires_at: i64,
    },
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
    /// The same sentence as `op.plain` ("Give laptop-2 your piece of the saved password of host 0A0A ...").
    pub plain: String,
    pub checks: Vec<Check>,
    pub expires_at: i64,
    /// This phone was restored and has not taken its chain yet: it cannot sign, so Approve is refused (and keeps the
    /// request open) until `take_chain` and `Decision::Approve` accept the head. Show the stop box up front.
    pub head_pending: bool,
    /// Set for a requester this phone never met: Approve needs this id typed (`ReleaseOptions.typed_id`).
    pub type_to_approve: Option<String>,
    /// The requester as this phone knows it: name (escaped), whether it is a terminal this phone enrolled with, and the
    /// Ed25519 public key (hex) it enrolled with, only when the requester's signature verified under that key (the
    /// device id is its first 8 bytes).
    pub requester_name: Option<String>,
    pub requester_known: bool,
    pub requester_key: Option<String>,
    /// What this phone's own share is: `"hybrid"` (the original sharing) or `"envelope"` (after a reshare), the
    /// epoch the share belongs to, and its index in the sharing (all read from the phone material the terminal sent
    /// and this phone opened). The credential's name and mode (single or quorum) are NOT in the signed request: a
    /// terminal does not send them, so the core has nothing to show.
    pub share_kind: String,
    pub share_epoch: u64,
    pub share_index: u32,
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
    /// Everything below comes from the SIGNED op, never from when the phone read the notice. `proposed_at` is the op's
    /// own timestamp, `lamport` and `seq` its own counters, `lapses_at` = `proposed_at` + 7 days: a Red ceremony
    /// lapses then (the page refuses the op after that), whatever the phone clock says when it was scanned.
    pub proposed_at: i64,
    pub lamport: u64,
    pub seq: u64,
    pub lapses_at: i64,
    /// Name this phone knows the proposer by (a terminal it enrolled with, or a name it was given), escaped.
    pub proposer_name: Option<String>,
    pub proposer: String,
}

/// Kind 3 carrying a `PhoneReenroll`: a recovery that this phone (still alive) can cancel.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ReenrollNoticeView {
    pub token: u64,
    pub op: OpView,
    pub op_hash: String,
    /// When the ring accepts the new phone unless it is cancelled.
    pub ends_at: i64,
    /// The signed time and lamport of the `PhoneReenroll` op (the timelock in `ends_at` counts from the `initiated_at`
    /// the op itself names, not from when this phone read the notice).
    pub proposed_at: i64,
    pub lamport: u64,
    pub signature_note: String,
}

/// Kind 9: a terminal asks this phone to issue a trust session.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CertRequestView {
    pub token: u64,
    pub headline: String,
    /// One plain sentence ("Let laptop-2 open 2 hosts without asking until 2030-01-04, up to 2 new hosts a day.").
    pub plain: String,
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
    /// One plain sentence ("Allow 5 more new hosts today for the trust session of laptop-2.").
    pub plain: String,
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
    /// `agreed` came from ONE list: `decide(token, Decision::AcceptSingleSource)` is needed (the user's tick); a plain
    /// `Approve` is refused. The card must show `terminals_count` and the age of `terminals_newest_at`.
    pub single_source: bool,
    /// The sender's own (unsigned, informational) count of the ring's terminals and the author stamp of its newest op.
    pub terminals_count: Option<u32>,
    pub terminals_newest_at: Option<i64>,
    /// The terminal id (hex) the list named as its sender (unsigned: it tells two lists apart, it proves nothing), if it
    /// named one. Two lists from two different ids naming the same head are what `agreed` needs.
    pub sender: Option<String>,
    /// The sentence the page prints under the title: "Newest op in this list: seq N, signed ...Z (n op(s) read)."
    pub summary: String,
    /// What to do next, in the page's words (take another list, or the warning to read before adopting).
    pub note: String,
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
    /// The person chose to read the share to the terminal by hand: only then does `Outgoing.manual` carry the
    /// plaintext share words. False keeps them out of the reply (they would reach the JVM as immutable Strings).
    pub by_hand: bool,
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
    /// Accept a chain head that came from ONE list, with the user's tick on the risk (`ChainAdopted.single_source`).
    AcceptSingleSource,
    /// Name this phone for the ring (enrollment step 1).
    EnrollName { name: String },
    /// The four words are (not) identical (enrollment step 3).
    EnrollSas { matches: bool },
}

// ---------------------------------------------------------------- channels

/// What `scan_gray` saw in a camera frame.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScanEvent {
    Progress {
        have: u32,
        total: u32,
        damaged: u32,
    },
    /// A completed document (a whole frame of kinds 1 to 10): hand it to `open_frame`.
    Document {
        bytes: Vec<u8>,
    },
    /// A completed document that is not an SSHub frame (no `SWR1` magic, or a kind outside 1..=10): the page ignores it
    /// ("not what this step needs"). The collector was reset; nothing is handed to `open_frame`.
    Foreign {
        len: u32,
        /// The document itself when it is small (at most 256 bytes: a plain QR such as `sshub:dev:1:<16 hex>`), else
        /// empty.
        bytes: Vec<u8>,
    },
}
