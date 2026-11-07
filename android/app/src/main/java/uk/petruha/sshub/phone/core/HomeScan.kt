package uk.petruha.sshub.phone.core

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** What the generic scan screen is for. Route: `Routes.Scan + "/" + purpose.arg` (see `scanRoute`). */
enum class ScanPurpose(val arg: String) {
    Request("request"), EnrollStep("enroll-step"), Chain("chain"), Recovery("recovery"), Reshare("reshare"),
    /** The id of the other device, read from the terminal's QR (`sshub:dev:1:<16 hex>`); the core knows nothing of it. */
    DeviceId("device-id");

    companion object {
        fun fromArg(arg: String?): ScanPurpose = entries.firstOrNull { it.arg == arg } ?: Request
    }
}

fun scanRoute(purpose: ScanPurpose): String = Routes.Scan + "/" + purpose.arg

/**
 * Maps what the core opened to a route, or null when the caller handles it itself. Implemented by the integrator (it knows every
 * `Opened` subtype); passed to `homeGraph`.
 */
fun interface OpenedRouter { fun route(opened: Opened): String? }

/** The result of a `ScanPurpose.DeviceId` scan: the 16 lowercase hex digits. Built by the scan screen, never by the core. */
class DeviceIdScanned(val id: String) : Opened

class ScanResult(val purpose: ScanPurpose, val opened: Opened)

/**
 * The scan screen publishes the `Opened` result here when the user taps Continue, then either navigates to `OpenedRouter.route(opened)`
 * or pops back to the caller. A caller that wants the result reads `latest` after the pop and calls `take(purpose)`.
 */
object ScanResults {
    private val state = MutableStateFlow<ScanResult?>(null)
    val latest: StateFlow<ScanResult?> = state.asStateFlow()
    fun publish(result: ScanResult) { state.value = result }
    /** Returns and clears the result when it is for `purpose`. */
    fun take(purpose: ScanPurpose): ScanResult? {
        val r = state.value
        if (r == null || r.purpose != purpose) return null
        state.value = null
        return r
    }
}
