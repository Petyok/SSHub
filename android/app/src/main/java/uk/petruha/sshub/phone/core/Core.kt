package uk.petruha.sshub.phone.core

import uk.petruha.sshub.phone.ui.components.QrMatrix

/*
 * The Kotlin face of the Rust phone core (docs/host-sync-android.md section 2.1).
 *
 * The real implementation is `UniffiPhoneCore`, a thin adapter over the uniffi-generated `PhoneCore` that maps its records to the
 * types below (added when crates/sshub-phone-core is merged). Screens and view models only ever see these types, so the UI is built
 * and tested against `FakePhoneCore` (debug sources) and does not change when the core lands.
 *
 * Ownership: every screen group owns ONE sub-interface, its model types and its fake, in its own file(s) of this package.
 * `PhoneCoreApi` below only composes them. Rules for all of them:
 *  - Rust never touches the platform: bytes and `now` (unix seconds) go in, bytes and views come out.
 *  - Kotlin never decides security: every "what you are approving" string comes from the core; the UI renders it (never edits, trims or re-orders).
 *  - A decision that signs is `suspend` and returns only after the core produced the bytes; the biometric CryptoObject gate is in the Platform port.
 */

class CoreException(val code: Code, message: String) : Exception(message) {
    enum class Code { WrongSecret, HeadPending, Refused, Malformed, Expired, Internal }
}

/** What the core knows after unlock; drives the home banner and the "head pending" card (C21). */
data class PhoneStatus(
    val deviceId: String,
    val deviceName: String,
    val enrolled: Boolean,
    val headPending: Boolean,
    val epoch: Long,
)

/** Result of `openFrame`: the screen to show for a frame read from any channel. Groups add their own subtypes in their own files. */
sealed interface Opened {
    /** A frame the phone does not act on (wrong kind, wrong ring, damaged). `reason` is literal text. */
    data class Ignored(val reason: String) : Opened
}

/** Progress of the camera path: `scanGray` feeds the QR decoder and the fountain decoder. */
sealed interface ScanEvent {
    data object None : ScanEvent
    /** A QR that is not one of ours. */
    data object Foreign : ScanEvent
    data class Progress(val have: Int, val need: Int) : ScanEvent
    /** A complete frame: hand it to `openFrame`. */
    class Frame(val bytes: ByteArray) : ScanEvent
}

/** Everything a reply can be shown or sent as. Rust renders the payloads; Kotlin displays and plays. */
class Outgoing(
    val frame: ByteArray,
    /** Base64 of the frame, as the terminal accepts it by paste. */
    val text: String,
    /** Looping fountain QR frames at 4 per second, black on white; empty when the frame is too large for QR. */
    val qr: List<QrMatrix>,
    /** 16-bit mono PCM at `audioRate` for each requested profile; empty when the channel was not asked for. */
    val audioFast: ShortArray? = null,
    val audioSlow: ShortArray? = null,
    val audioRate: Int = 48_000,
)

interface ScanApi {
    /** Luma plane (row stride == width), any orientation the camera delivers. Called from the analysis thread. */
    fun scanGray(width: Int, height: Int, luma: ByteArray): ScanEvent
    fun resetScan()
    /** Frame text pasted by the user (base64, whitespace tolerated). */
    fun frameFromText(text: String): ByteArray?
    suspend fun openFrame(frame: ByteArray, now: Long): Opened
}

/** The whole core. Group sub-interfaces are added here, one line each; nothing else in this file changes. */
interface PhoneCoreApi :
    ScanApi,
    IdentityApi,
    HomeApi,
    ReleaseApi,
    TrustApi,
    RecoveryApi,
    EnrollApi
