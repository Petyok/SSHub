package uk.petruha.sshub.phone.core

/** `Opened.TrustRequest`: a terminal asks this phone to sign a trust certificate (kind 9). */
data class OpenedTrustRequest(val view: TrustRequestView) : Opened

/** `Opened.TrustExtend`: a terminal asks for a budget extension of a certificate (kind 10). */
data class OpenedTrustExtend(val view: TrustExtendView) : Opened

sealed interface TrustDecision {
    /** `typedId` is what the user typed for a requester never met, else null. */
    data class Approve(val typedId: String?) : TrustDecision
    data object Decline : TrustDecision
}

data class TrustDevice(
    /** Null when the phone never met the device. */
    val name: String?,
    /** `7B3E-0D15`. */
    val id: String,
    val met: Boolean,
)

/** One row of the diff box, rendered as is (the key may already contain a count, e.g. `Hosts (5)`). */
data class TrustFieldView(val key: String, val value: String, val danger: Boolean = false)

data class TrustCheck(val text: String, val ok: Boolean)

/** The highlighted epoch range: its own box with the two checks (starts at this phone's epoch, at most one epoch wide). */
data class TrustEpochView(val label: String, val value: String, val checks: List<TrustCheck>)

/** A refusal value. `code` is the refusal index of the doc table (0..8 for trust requests); the text is literal. */
data class TrustRefusal(val code: Int, val title: String, val text: String)

data class TrustRequestView(
    val token: Long,
    val device: TrustDevice,
    /** `laptop-main [7B3E-0D15] asks to open 5 hosts for 30 days`. */
    val headline: String,
    /** Every field, every host, no `N more` (D5). */
    val fields: List<TrustFieldView>,
    val epoch: TrustEpochView,
    /** Full SHA-256 hex of the requester key, present for a requester never met. */
    val fingerprint: String?,
    /** `laptop-main [7B3E-0D15], 5 hosts, 30 days, ends 2026-11-25`, shown after signing. */
    val signedSummary: String,
    val refusal: TrustRefusal?,
)

data class TrustExtendView(
    val token: Long,
    val headline: String,
    /** Device, certificate, hosts, term ends, budget now, requested. */
    val fields: List<TrustFieldView>,
    /** The one changing field: `from` is the old allowance, `to` the new one. */
    val diffKey: String,
    val diffFrom: String,
    val diffTo: String,
    val note: String,
    /** `laptop-main [7B3E-0D15], +10 new hosts for 24 hours`, shown after signing. */
    val signedSummary: String,
    val refusal: TrustRefusal?,
    /** Present for the `again` refusal: when the next extension is possible. */
    val again: TrustAgain?,
)

data class TrustAgain(val remainingSeconds: Long, val windowSeconds: Long)

enum class CertState { Active, Expired, Revoked }

data class CertExtensionInfo(val at: Long, val extraHosts: Int)

data class IssuedCert(
    val certId: String,
    val deviceName: String,
    val deviceId: String,
    val state: CertState,
    val hosts: List<String>,
    val issuedAt: Long,
    val endsAt: Long,
    val termDays: Int,
    val budgetPerDay: Int,
    val extensions: List<CertExtensionInfo>,
)

enum class ShortCodeState { Live, Expired, DeclaredDead }

data class ShortCodeRow(val host: String, val state: ShortCodeState, val issuedAt: Long, val expiresAt: Long)

sealed interface CodeIssuedView {
    /** `code` is the 16 characters without separators; the UI groups them by 4. */
    class Issued(val credential: String, val code: String, val issuedAt: Long, val expiresAt: Long) : CodeIssuedView
    /** A code for this credential is already live: a second one is refused. */
    data class Live(val credential: String, val liveUntil: Long) : CodeIssuedView
}

/** What a hand-off screen shows: the signed reply of a certificate, a revocation or an extension. */
enum class HandoffKind { Certificate, Revocation, Extension }
class TrustHandoff(val kind: HandoffKind, val summary: String, val outgoing: Outgoing)
