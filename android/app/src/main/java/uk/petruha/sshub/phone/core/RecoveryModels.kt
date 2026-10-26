package uk.petruha.sshub.phone.core

/** Where the wrap-key and reshare sequence stands. `Empty`: no reshare is waiting. */
enum class ReshareStage { Empty, Start, Rotated, Adopted, Published }

/** `verified` false is shown as "signature not checked". */
data class ReshareContributor(val name: String, val id: String, val verified: Boolean)

/** A reshare as the core read it. `note` states the contributor rule and its result; `adoptable` is the core's verdict. */
data class ReshareView(
    val epoch: Long,
    val holderNumber: Int,
    val credentialsReady: Int,
    val credentialsTotal: Int,
    val contributors: List<ReshareContributor>,
    val note: String,
    val adoptable: Boolean,
)

/** `reshare` is the adopted reshare once the stage is Adopted or Published, else null. */
data class WrapKeyState(
    val generation: Int,
    val epoch: Long,
    val holderNumber: Int,
    val credentialsReady: Int,
    val credentialsTotal: Int,
    val stage: ReshareStage,
    val reshare: ReshareView?,
)

/** `openFrame` result for a reshare frame; review it with `reshareCandidate(token)`, accept with `decideReshare(token)`. */
class ReshareOpened(val token: Long, val view: ReshareView) : Opened

data class RecoveryNoticeSummary(val token: Long, val startedBy: String, val startedAt: Long, val endsAt: Long)

/** The fields of a recovery notice. `startedBy` is "name [ID]"; `*Utc` are core-formatted UTC stamps; `hash` is 64 hex digits. */
data class RecoveryNoticeView(
    val startedBy: String,
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
)

/** `openFrame` result for a recovery notice frame; leads to the recovery-notice screen for `token`. */
class RecoveryNoticeOpened(val token: Long, val view: RecoveryNoticeView) : Opened

data class ExportOpRow(val seq: Long, val kind: String, val time: String, val hash: String)

/** `outgoing` is null when this phone chain has no signed ops yet. */
class ExportedOps(val rows: List<ExportOpRow>, val total: Int, val outgoing: Outgoing?)
