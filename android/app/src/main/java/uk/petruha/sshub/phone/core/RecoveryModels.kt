package uk.petruha.sshub.phone.core

/**
 * Where the wrap-key and reshare sequence stands, in order. `Empty`: no reshare is waiting. `Candidate`: rotated, and a reshare
 * opened by `openFrame` is held for review. `Published`: adopted and published in one signing.
 */
enum class ReshareStage { Empty, Start, Rotated, Candidate, Published }

/**
 * `verified` false is shown as "signature not checked". `known` is whether this phone has met the contributor; `hasSession` whether it
 * holds a session key for it (without one this phone cannot read that contributor's shares).
 */
data class ReshareContributor(val name: String, val id: String, val verified: Boolean, val known: Boolean = true, val hasSession: Boolean = true)

/** One holder of the shares after the reshare. `isThisPhone` marks this phone. */
data class ReshareHolder(val id: String, val name: String, val isThisPhone: Boolean)

/** A reshare as the core read it. `note` states the contributor rule and its result; `adoptable` is the core's verdict. */
data class ReshareView(
    val epoch: Long,
    val holderNumber: Int,
    val credentialsReady: Int,
    val credentialsTotal: Int,
    val contributors: List<ReshareContributor>,
    val note: String,
    val adoptable: Boolean,
    /** Who holds a share once the reshare is adopted, in the core's order. */
    val holdersAfter: List<ReshareHolder> = emptyList(),
    /** A storage key of this epoch is published with the new shares (false: one was already published). */
    val storageKeyNew: Boolean = false,
)

/** `reshare` is the reshare that was adopted and published once the stage is Published, else null. */
data class WrapKeyState(
    val generation: Int,
    val epoch: Long,
    val holderNumber: Int,
    val credentialsReady: Int,
    val credentialsTotal: Int,
    val stage: ReshareStage,
    val reshare: ReshareView?,
)

/** What one `decideReshare` gives back: the state after it, and the signed ops (QR and text) to hand to a terminal. */
class ReshareOutcome(val state: WrapKeyState, val outgoing: Outgoing)

/** `openFrame` result for a reshare frame; review it with `reshareCandidate(token)`, adopt and publish with `decideReshare(token)`. */
class ReshareOpened(val token: Long, val view: ReshareView) : Opened

data class RecoveryNoticeSummary(val token: Long, val startedBy: String, val startedAt: Long, val endsAt: Long)

/**
 * The fields of a recovery notice. `startedBy` is "name [ID]"; `*Utc` are core-formatted UTC stamps; `hash` is 64 hex digits.
 * `phoneName` is empty when the phone has no name for itself; the screen then leaves the line out. `lamport` orders the signed op.
 */
data class RecoveryNoticeView(
    val startedBy: String,
    val lamport: Long,
    val phoneName: String,
    val operation: String,
    val startedAt: Long,
    val startedUtc: String,
    val endsAt: Long,
    val endsUtc: String,
    val hash: String,
    val signatureNote: String,
    /** False when this phone cannot cancel it (already cancelled, timelock over); then `closedReason` says why. */
    val cancellable: Boolean,
    val closedReason: String? = null,
    /** Every leaf of the signed op, with the core's danger flag (signers, paper phrase, third factor, ...). */
    val opRows: List<RedField> = emptyList(),
)

/** `openFrame` result for a recovery notice frame; leads to the recovery-notice screen for `token`. */
class RecoveryNoticeOpened(val token: Long, val view: RecoveryNoticeView) : Opened

data class ExportOpRow(val seq: Long, val kind: String, val time: String, val hash: String, val danger: Boolean = false)

/** `outgoing` is null when this phone chain has no signed ops yet. */
class ExportedOps(val rows: List<ExportOpRow>, val total: Int, val outgoing: Outgoing?)
