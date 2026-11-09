package uk.petruha.sshub.phone.core

/** `Opened.TrustRequest`: a terminal asks this phone to sign a trust certificate (kind 9). */
data class OpenedTrustRequest(val view: TrustRequestView) : Opened

/** `Opened.TrustExtend`: a terminal asks for a budget extension of a certificate (kind 10). */
data class OpenedTrustExtend(val view: TrustExtendView) : Opened

/** `Opened.CertRenew`: a terminal asks this phone to renew a trust session it issued (kind 11). */
data class OpenedTrustRenew(val view: TrustRenewView) : Opened

/** The core's `CERT_RENEW_EVIDENCE_MAX_AGE_SECS`: older news from another terminal is stale. */
const val RENEW_EVIDENCE_MAX_AGE_SECS = 6 * 3600L

/** What the phone shows for a renewal (the core's `RenewView` plus its card text). Signing needs `refusal == null`. */
data class TrustRenewView(
    val token: Long,
    val headline: String,
    /** One plain sentence from the core; empty when none. */
    val plain: String,
    val delegate: String,
    val scope: String,
    /** What the renewal adds to the end that counts now, seconds. */
    val termSecs: Long,
    val budget: Int,
    val renewalsLeft: Int,
    /** The new end date, unix seconds. */
    val newEnd: Long,
    /** Names of the hosts the delegate opened that the terminal listed (at most 8, newest first). */
    val opened: List<String>,
    /** How many hosts it opened in all; `opened` empty and 0 here means the terminal sent no list (never "nothing opened"). */
    val openedTotal: Int,
    /** Age of the freshest news from a terminal other than the delegate; null: none, or only the delegate vouches for the list. */
    val evidenceAgeSecs: Long?,
    val refusal: TrustRefusal?,
    val note: String,
    val signedSummary: String,
) {
    /** The row under "Hosts it opened": the names and "and N more (T in all)", or that no list came. */
    val openedText: String get() {
        if (opened.isEmpty() && openedTotal == 0) return "no list sent: the terminal did not say which hosts it opened"
        val more = openedTotal - opened.size
        return opened.joinToString("\n") + if (more > 0) "\nand $more more ($openedTotal in all)" else ""
    }
    val evidenceStale: Boolean get() = evidenceAgeSecs == null || evidenceAgeSecs > RENEW_EVIDENCE_MAX_AGE_SECS
    /** Red: the core refuses, or the evidence is missing, stale or only the delegate's own. */
    val red: Boolean get() = refusal != null || evidenceStale
}

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
) {
    /** `laptop-main [7B3E-0D15]`, or `not met [7B3E-0D15]`: never an empty name. */
    val label: String get() = "${name?.takeIf { met && it.isNotBlank() } ?: "not met"} [$id]"
}

/** One row of the diff box, rendered as is (the key may already contain a count, e.g. `Hosts (5)`). */
data class TrustFieldView(val key: String, val value: String, val danger: Boolean = false)

data class TrustCheck(val text: String, val ok: Boolean)

/** The highlighted epoch range: its own box; `checks` is empty until the core supplies them, and a line is drawn only per present check. */
data class TrustEpochView(val label: String, val value: String, val checks: List<TrustCheck> = emptyList())

/** A refusal value. `code` is the refusal index of the doc table (0..8 for trust requests); the text is literal. */
data class TrustRefusal(val code: Int, val title: String, val text: String)

data class TrustRequestView(
    val token: Long,
    val device: TrustDevice,
    /** `laptop-main [7B3E-0D15] asks to open 5 hosts for 30 days`. */
    val headline: String,
    /** Every field, every host, no `N more` (D5). */
    val fields: List<TrustFieldView>,
    /** Null when the core sent no epoch row. */
    val epoch: TrustEpochView?,
    /** `laptop-main [7B3E-0D15], 5 hosts, 30 days, ends 2026-11-25`, shown after signing. */
    val signedSummary: String,
    val refusal: TrustRefusal?,
    /** One plain sentence from the core (U13-8); empty when none. */
    val plain: String = "",
)

data class TrustExtendView(
    val token: Long,
    val headline: String,
    /** Device, certificate, hosts, term ends, budget now, requested. */
    val fields: List<TrustFieldView>,
    /**
     * The one changing field. With `diffFrom` set, `from` is the old allowance and `to` the new one for the next 24 hours; with
     * `diffFrom` empty it is a plain row (the extra alone). `diffDanger` is the core's flag.
     */
    val diffKey: String,
    val diffFrom: String,
    val diffTo: String,
    val diffDanger: Boolean = true,
    val note: String,
    /** `laptop-main [7B3E-0D15], +10 new hosts for 24 hours`, shown after signing. */
    val signedSummary: String,
    val refusal: TrustRefusal?,
    /** Present for the `again` refusal: when the next extension is possible. */
    val again: TrustAgain?,
    /** One plain sentence from the core (U13-8); empty when none. */
    val plain: String = "",
)

data class TrustAgain(val remainingSeconds: Long, val windowSeconds: Long)

enum class CertState { Active, Expired, Revoked }

data class CertExtensionInfo(val at: Long, val extraHosts: Int)

data class IssuedCert(
    val certId: String,
    /** Null when this phone never met the device: shown as `not met` plus [deviceId]. */
    val deviceName: String?,
    val deviceId: String,
    val state: CertState,
    /** Empty for a group scope. */
    val hosts: List<String>,
    /** The scope group name; when set it is the scope, instead of a host list. */
    val group: String?,
    val issuedAt: Long,
    val endsAt: Long,
    val epochLo: Long,
    val epochHi: Long,
    /** The core's wording of the term, shown as is. */
    val termText: String,
    val budgetPerDay: Int,
    val extensions: List<CertExtensionInfo>,
    /** Chain position of the cert op; null when the core gives none. */
    val lamport: Long?,
    /** Renewals that count (0 to 2); [endsAt] already includes them. */
    val renewals: Int = 0,
) {
    /** The scope line: the group name, else the host count. */
    val scope: String get() = group?.let { "group $it" } ?: "${hosts.size} hosts"
    val deviceLabel: String get() = "${deviceName?.takeIf { it.isNotBlank() } ?: "not met"} [$deviceId]"
}

enum class ShortCodeState { Live, Expired, DeclaredDead }

data class ShortCodeRow(val host: String, val state: ShortCodeState, val issuedAt: Long, val expiresAt: Long)

sealed interface CodeIssuedView {
    /** `code` is the 16 characters without separators; the UI groups them by 4. */
    class Issued(val credential: String, val code: String, val issuedAt: Long, val expiresAt: Long) : CodeIssuedView
    /** A code for this credential is already live: a second one is refused. */
    data class Live(val credential: String, val liveUntil: Long) : CodeIssuedView
}

/** What a hand-off screen shows: the signed reply of a certificate, a revocation or an extension. */
enum class HandoffKind { Certificate, Revocation, Extension, Renewal }
class TrustHandoff(val kind: HandoffKind, val summary: String, val outgoing: Outgoing)
