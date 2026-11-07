package uk.petruha.sshub.phone.core

/*
 * Home group: this phone, ring entries, approvals list. Only what the real core can fill lives here (`status(now)` plus the pending
 * requests in the adapter cache); a field the core cannot give is not in the model.
 */

/** One ring entry. `name` null means the phone has not met this device: the screen shows "not met" plus the id. */
data class RingDevice(val name: String?, val id: String, val self: Boolean)

data class RingView(
    /** Highest ring epoch this phone has seen on its own chain. */
    val epoch: Long,
    val devices: List<RingDevice>,
    /** C21: restored phone has not taken its chain; Home shows the red card. */
    val headPending: Boolean = false,
)

/** This phone. `headSeq` null means "no ops yet". */
data class HomePhoneView(val id: String, val name: String, val headSeq: Long?, val liveShortCodes: Int)

enum class PendingKind { Red, Release, Trust, Extend, Recovery, Reshare }

/** One waiting request. `route` is a ready navigation route (owned by the screen group that handles `kind`). */
data class PendingItem(val kind: PendingKind, val title: String, val age: String?, val route: String)

interface HomeApi {
    /** Null when the phone is not enrolled. */
    suspend fun ring(now: Long): RingView?
    suspend fun thisPhone(now: Long): HomePhoneView
    fun pending(now: Long): List<PendingItem>
}
