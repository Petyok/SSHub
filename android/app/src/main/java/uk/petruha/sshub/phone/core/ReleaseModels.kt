package uk.petruha.sshub.phone.core

/*
 * Models of the Release screen group: Red operations, share releases and refusals (spec-A section 12, rules D1-D20).
 * Every string a user decides on (fields, checks, words, hashes, reasons) is produced by the core and rendered as is.
 * Times are unix seconds; the UI derives ages and countdowns from `now`, it never reads a clock of its own.
 */

/**
 * One line of a check list. `ok = false` is shown as a red line with a cross and the words (D3, D8).
 * `needsTypedId` marks the one check that fails only because the requester was never met: it stops blocking Approve once the
 * user typed the requester id (D13). Every other failing check blocks Approve for good.
 */
data class ReleaseCheck(val text: String, val ok: Boolean, val needsTypedId: Boolean = false)

enum class ProofState { YourTurn, Signed, Waiting, Refused }

/** One of the three proofs of a Red operation (D6). `name` and `caption` are the core's words, for example "phone" / "this phone". */
data class RedProofView(val name: String, val caption: String, val state: ProofState)

/** One changing field. `from == null` is a field with no old value (plain row with a red gutter, D2). */
data class RedField(val key: String, val from: String?, val to: String, val mono: Boolean = false)

/**
 * A Red notice as the core describes it. `hash` is the full 64 hex digits of the operation (D7).
 * `proposedAt` and `lapsesAt` bound the 3 day validity (contract C21); the UI shows the age on a tick scale between them.
 * `checks` decide whether Approve is enabled (D8): every one must be ok.
 */
data class RedView(
    val token: Long,
    val kind: String,
    val title: String,
    val proposerName: String,
    val proposerId: String,
    val lamport: Long,
    val proposedAt: Long,
    val lapsesAt: Long,
    val hash: String,
    val fields: List<RedField>,
    val note: String,
    val proofs: List<RedProofView>,
    val checks: List<ReleaseCheck>,
) {
    init { require(proofs.size == 3) { "a Red operation always has three proofs (D6)" } }
}

enum class ChannelKind { Qr, Clipboard, SoundFast, SoundSlow }
data class ReleaseChannel(val kind: ChannelKind, val label: String)

/**
 * A share release request. `expiresAt - signedAt` is the challenge lifetime (120 s). `mustType` is the requester id the user has to type
 * when the requester was never met (D13); null for a requester that was met. `fingerprint` is the requester's full 64 hex digits when known.
 * `shortCodeOffer` is the line under the short code toggle, null when the core offers none.
 */
data class ReleaseView(
    val token: Long,
    val requesterName: String,
    val requesterId: String,
    val met: Boolean,
    val fingerprint: String?,
    val host: String,
    val credential: String,
    val mode: String,
    val signedAt: Long,
    val expiresAt: Long,
    val words: List<String>,
    val checks: List<ReleaseCheck>,
    val channels: List<ReleaseChannel>,
    val mustType: String?,
    val shortCodeOffer: String?,
)

data class RefusalRow(val key: String, val value: String)

/**
 * Why a frame ended on the refusal screen. `label` is the short word in the banner, `title` the sentence, `text` what it means.
 * `compared` are the values the core compared (both shown in full). `mismatch` makes the screen a stop box with Back as the only action (D14).
 */
data class RefusalReason(
    val key: String,
    val label: String,
    val title: String,
    val text: String,
    val compared: List<RefusalRow>,
    val mismatch: Boolean,
)

data class ShareWords(val index: Int, val of: Int, val words: List<String>)

/** The 16 character short code, shown once. No copy button exists for it (D19). */
data class IssuedShortCode(val code: String, val validUntil: Long, val weak: Boolean)

/** Open frame results of this group. Top-level classes: `Opened` is sealed in Core.kt and cannot gain nested members here. */
data class OpenedRed(val view: RedView) : Opened
data class OpenedRelease(val view: ReleaseView) : Opened
/** A refusal is a value, never an error; a refusal screen never has an Approve button. */
data class OpenedRefused(val reason: RefusalReason, val checks: List<ReleaseCheck>) : Opened

data class ReleaseChannels(
    val qr: Boolean = false,
    val clipboard: Boolean = false,
    val soundFast: Boolean = false,
    val soundSlow: Boolean = false,
)

sealed interface ReleaseDecision {
    /** Red: `Approve()` with defaults. Release: channels, optional short code, typed requester id (checked in the core too). */
    data class Approve(
        val channels: ReleaseChannels = ReleaseChannels(),
        val shortCode: Boolean = false,
        val byHand: Boolean = false,
        val typedId: String? = null,
    ) : ReleaseDecision
    data object Decline : ReleaseDecision
}

/** `outgoing` is null for a decline. `byHand` and `shortCode` are present only when requested in the decision. */
class ReleaseOutcome(
    val outgoing: Outgoing?,
    val shortCode: IssuedShortCode? = null,
    val byHand: ShareWords? = null,
    val note: String? = null,
)
