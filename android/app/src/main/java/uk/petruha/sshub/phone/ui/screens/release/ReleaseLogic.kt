package uk.petruha.sshub.phone.ui.screens.release

import uk.petruha.sshub.phone.core.ChannelKind
import uk.petruha.sshub.phone.core.RedView
import uk.petruha.sshub.phone.core.ReleaseCheck
import uk.petruha.sshub.phone.core.ReleaseOutcome
import uk.petruha.sshub.phone.core.ReleaseView
import uk.petruha.sshub.phone.core.VaultResult
import uk.petruha.sshub.phone.ui.components.hexGroups

/*
 * Pure functions behind the Red, release and refusal screens: no Compose, no clock, no Android. Time is always an explicit `now`
 * (unix seconds). Nothing here approves anything: a timer can only change what is shown (D11).
 */

/** An approval session (reveal, release, sign) must complete within 12 minutes of the first interaction (D10, contract C16). */
const val APPROVAL_WINDOW_SECONDS = 720L

/** Release timer announcements: 120, 60, 30, 10 s and 0 (spec 9). */
val ANNOUNCE_AT = listOf(120, 60, 30, 10, 0)

const val TYPE_ID_FIRST = "type the id first"

fun windowLeft(openedAt: Long, now: Long): Long = (openedAt + APPROVAL_WINDOW_SECONDS - now).coerceIn(0L, APPROVAL_WINDOW_SECONDS)

/** True from exactly 12:00 after the first interaction (D10). */
fun windowClosed(openedAt: Long, now: Long): Boolean = now - openedAt >= APPROVAL_WINDOW_SECONDS

/** True from exactly the lapse time; an expired request cannot be approved (D10). */
fun redLapsed(view: RedView, now: Long): Boolean = now >= view.lapsesAt

fun releaseLeft(view: ReleaseView, now: Long): Long = (view.expiresAt - now).coerceAtLeast(0L)

fun mmss(seconds: Long): String {
    val s = seconds.coerceAtLeast(0L)
    return "%02d:%02d".format(s / 60, s % 60)
}

private fun plural(n: Long, one: String) = if (n == 1L) "$n $one" else "$n ${one}s"

/** "11 minutes 50 seconds left": the spoken form, constant inside each 10 second step so a live region speaks every 10 s only. */
fun spokenWindow(left: Long): String {
    val t = left.coerceAtLeast(0L).let { it - it % 10 }
    val m = t / 60
    val s = t % 60
    val parts = buildList { if (m > 0) add(plural(m, "minute")); if (s > 0 || m == 0L) add(plural(s, "second")) }
    return parts.joinToString(" ") + " left"
}

/** "2 d 23 h", "1 h 12 min", "6 min", "1 min 48 s", "38 s". */
fun durationText(seconds: Long): String {
    val s = seconds.coerceAtLeast(0L)
    val d = s / 86_400
    val h = s % 86_400 / 3_600
    val m = s % 3_600 / 60
    val r = s % 60
    return when {
        d > 0 -> "$d d $h h"
        h > 0 -> if (m > 0) "$h h $m min" else "$h h"
        m > 0 -> if (r > 0) "$m min $r s" else "$m min"
        else -> "$r s"
    }
}

/** "2 days 13 hours" for TalkBack. */
fun spokenDuration(seconds: Long): String {
    val s = seconds.coerceAtLeast(0L)
    val d = s / 86_400
    val h = s % 86_400 / 3_600
    val m = s % 3_600 / 60
    val r = s % 60
    return when {
        d > 0 -> plural(d, "day") + " " + plural(h, "hour")
        h > 0 -> plural(h, "hour") + " " + plural(m, "minute")
        m > 0 -> plural(m, "minute") + " " + plural(r, "second")
        else -> plural(r, "second")
    }
}

/** The smallest announcement step that is still at or above `left`: 82 s is in step 120, 60 s in step 60, 59 s in step 60, 0 s in step 0. */
fun announceStep(left: Long): Int = ANNOUNCE_AT.filter { left <= it }.minOrNull() ?: ANNOUNCE_AT.first()

/** Text of the release timer live region. It only changes when `announceStep` changes (120, 60, 30, 10, 0), not every second. */
fun announcement(left: Long, released: Boolean): String {
    val step = announceStep(left)
    return when {
        left <= 0 -> if (released) "Share expired" else "Request expired"
        released -> "Share released, up to $step seconds left"
        else -> "Request open, up to $step seconds left"
    }
}

data class Gate(val enabled: Boolean, val reason: String?)

/** D8: Approve is enabled only when every check passes and the request is inside its limits. Pure, so it is unit-testable. */
fun redGate(checks: List<ReleaseCheck>, lapsed: Boolean, closed: Boolean, busy: Boolean): Gate {
    if (closed) return Gate(false, "window closed")
    if (busy) return Gate(false, "signing")
    if (lapsed) return Gate(false, "the request lapsed after 3 days")
    checks.firstOrNull { !it.ok }?.let { return Gate(false, it.text) }
    return Gate(true, null)
}

fun typedIdMatches(required: String?, typed: String): Boolean = required == null || typed.trim().uppercase() == required.uppercase()

/**
 * D8 + D13 for a share release. A check with `needsTypedId` stops blocking once the typed id equals the requester id; it keeps
 * blocking when the view names no id to type. Every other failing check, an expired challenge or a closed window blocks for good.
 */
fun releaseGate(checks: List<ReleaseCheck>, left: Long, closed: Boolean, mustType: String?, typed: String, busy: Boolean): Gate {
    if (closed) return Gate(false, "window closed")
    if (busy) return Gate(false, "signing")
    if (left <= 0L) return Gate(false, "the request expired")
    checks.firstOrNull { !it.ok && !(it.needsTypedId && mustType != null) }?.let { return Gate(false, it.text) }
    if (!typedIdMatches(mustType, typed)) return Gate(false, TYPE_ID_FIRST)
    return Gate(true, null)
}

/** Null means the vault released the secret and the decision may go on; anything else is the literal text to show. Nothing was signed. */
fun vaultFailure(r: VaultResult): String? = when (r) {
    is VaultResult.Secret -> null
    VaultResult.Canceled -> "The system lock was canceled. Nothing was signed."
    VaultResult.LockedOut -> "The system lock is locked out for now. Nothing was signed. Try again later."
    VaultResult.Invalidated -> "The key was invalidated by a change of the system lock. Nothing was signed. Restore this phone from its 24 words."
    VaultResult.NoSecureLock -> "This device has no screen lock. Nothing was signed."
    VaultResult.Stored -> "The system lock gave no key. Nothing was signed."
    is VaultResult.Failed -> "The system lock failed: ${r.message}. Nothing was signed."
}

/** Subtitle of the system prompt: the short hash prefix, 12 hex digits in 3 groups. */
fun hashPrefix(hash: String): String = hexGroups(hash).take(3).joinToString(" ").uppercase()

enum class ApprovalPhase { Review, Signing, Approved, Declined }

data class RedState(
    val view: RedView,
    val now: Long,
    val openedAt: Long = now,
    val phase: ApprovalPhase = ApprovalPhase.Review,
    val showHash: Boolean = false,
    val outcome: ReleaseOutcome? = null,
    val copied: Boolean = false,
    val clearsIn: Int? = null,
    val error: String? = null,
) {
    val lapsed: Boolean get() = redLapsed(view, now)
    val closed: Boolean get() = phase == ApprovalPhase.Review && windowClosed(openedAt, now)
    val gate: Gate get() = redGate(view.checks, lapsed, closed, phase == ApprovalPhase.Signing)
}

data class ReleaseState(
    val view: ReleaseView,
    val now: Long,
    val openedAt: Long = now,
    val phase: ApprovalPhase = ApprovalPhase.Review,
    val typed: String = "",
    val channelsOn: Set<ChannelKind> = view.channels.map { it.kind }.toSet(),
    val shortCode: Boolean = false,
    val byHand: Boolean = false,
    val outcome: ReleaseOutcome? = null,
    val copied: Boolean = false,
    val clearsIn: Int? = null,
    val error: String? = null,
) {
    val left: Long get() = releaseLeft(view, now)
    val closed: Boolean get() = phase == ApprovalPhase.Review && windowClosed(openedAt, now)
    val gate: Gate get() = releaseGate(view.checks, left, closed, view.mustType, typed, phase == ApprovalPhase.Signing)
}
