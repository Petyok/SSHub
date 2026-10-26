package uk.petruha.sshub.phone.core

/*
 * Home group: ring overview, approvals list. Every string a screen shows about the ring (names, ages, "seen" texts, route tokens) comes
 * from the core as data. The real core exposes only what the phone's own state knows (`status(now)`), so `ring` and `pending` may be a
 * subset of this: every field the phone cannot know is nullable and the UI leaves the line out.
 */

/** One device in the ring chain. `seen` is the core's text ("3 min ago"); null when the phone has no sighting of it. */
data class RingDevice(
    val name: String,
    val id: String,
    val role: String?,
    val seen: String?,
    val self: Boolean,
    val stale: Boolean,
)

/** Last sync on a tick scale: `agoSeconds` of `windowSeconds`; `agoText` is the core's wording ("3 min ago"). */
data class RingSync(val agoSeconds: Long, val agoText: String, val windowSeconds: Long, val via: String?)

data class RingView(
    val name: String,
    val epoch: Long,
    val hosts: Int?,
    val credentials: Int?,
    /** "2 of 3" */
    val threshold: String?,
    val quorum: Int?,
    val single: Int?,
    val lastSync: RingSync?,
    val devices: List<RingDevice>,
    val pendingRed: Int = 0,
    val pendingRelease: Int = 0,
    val pendingTrust: Int = 0,
    /** C21: restored phone has not taken its chain; Home shows the red card. */
    val headPending: Boolean = false,
)

/** What Home shows before the phone is enrolled (no ring). `chainHead` null means "no ops yet". */
data class HomePhoneView(val id: String, val name: String, val chainHead: String?, val liveShortCodes: Int)

enum class PendingKind { Red, Release, Trust, Extend, Recovery, Reshare }

/** One waiting request. `route` is a ready navigation route (owned by the screen group that handles `kind`). */
data class PendingItem(val kind: PendingKind, val title: String, val age: String?, val route: String)

interface HomeApi {
    /** Null when the phone is not enrolled. */
    fun ring(now: Long): RingView?
    fun thisPhone(now: Long): HomePhoneView
    fun pending(now: Long): List<PendingItem>
}
