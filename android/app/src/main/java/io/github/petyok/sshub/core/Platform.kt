package io.github.petyok.sshub.core

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow
import io.github.petyok.sshub.platform.BleScanError
import io.github.petyok.sshub.platform.ObservedBleChannel

/*
 * Platform ports: everything Android-specific the screens need. Production adapters live in `platform/` (Keystore, BiometricPrompt, CameraX,
 * BLE GATT central, AudioTrack, ClipboardManager); tests use `FakePlatform` (debug sources). Screens never touch an Android API directly
 * except Compose itself, so every screen runs in a Compose test with no device feature.
 */

interface Platform {
    val clock: Clock
    val clipboard: ClipboardPort
    val keys: KeyVault
    val camera: CameraPort
    val ble: BlePort
    val audio: AudioPort
    val system: SystemPort
}

fun interface Clock { fun nowSeconds(): Long }

/** D19: nothing is copied by default; a copy sets EXTRA_IS_SENSITIVE, shows "Copied, clears in 60 s" and clears after 60 s (or on next resume). */
interface ClipboardPort {
    fun copySensitive(label: String, text: String)
    /** Text on the clipboard right now, read only on an explicit user tap (paste screens). */
    fun readText(): String?
    /** Seconds until the clipboard is cleared, or null when nothing of ours is on it. */
    val clearsInSeconds: StateFlow<Int?>
}

/** Keystore AES-GCM key and BiometricPrompt gate around the 32-byte phone secret (docs section 4). */
interface KeyVault {
    val state: StateFlow<VaultState>
    /** Creates the key and stores `secret` wrapped by it. Fails with `NoSecureLock` when the device has no screen lock. */
    suspend fun store(secret: ByteArray): VaultResult
    /** Shows the system prompt (title = action, subtitle = short hash prefix) and returns the secret; the caller zeroes it. */
    suspend fun unlock(title: String, subtitle: String): VaultResult
    suspend fun erase()
    /** Forgets the unlocked state (ON_STOP, screen off, auto-lock). */
    fun lock()
    /** `strongBox` is shown on About. */
    val strongBox: Boolean
}
enum class VaultState { NoKey, Locked, Unlocked, Invalidated }
sealed interface VaultResult {
    class Secret(val bytes: ByteArray) : VaultResult
    data object Stored : VaultResult
    data object NoSecureLock : VaultResult
    data object Canceled : VaultResult
    data object LockedOut : VaultResult
    data object Invalidated : VaultResult
    data class Failed(val message: String) : VaultResult
}

/** Camera permission and analysis frames. `Preview` draws the live image; frames go to `onLuma` on the analysis thread. */
interface CameraPort {
    val permission: StateFlow<CameraPermission>
    fun requestPermission()
    fun openAppSettings()
    val hasCamera: Boolean
    @Composable fun Preview(modifier: Modifier, onLuma: (width: Int, height: Int, luma: ByteArray) -> Unit)
}
enum class CameraPermission { Unknown, Granted, Denied, DeniedForever }

/** Bluetooth LE, GATT central of contract C20. The sans-IO link state machine is Rust (`BleSession`); this port moves its bytes. */
interface BlePort {
    val availability: StateFlow<BleAvailability>
    fun requestPermissions()
    /** Devices advertising the SSHub service UUID, as `SSHub-<TAG>`. Never accept-all. */
    fun scan(): Flow<List<BleDevice>>
    /** Why the last scan could not start or stopped (the system's scan error as a sentence), null while fine. */
    val scanError: StateFlow<BleScanError?>
    /** Connects, runs HELLO, and exposes whole frames in both directions until closed; the channel says where it is and why it ended. */
    suspend fun connect(device: BleDevice): ObservedBleChannel?
}
enum class BleAvailability { Ready, Off, PermissionNeeded, PermissionDenied, Unsupported }
data class BleDevice(val address: String, val name: String) { val tag: String get() = name.removePrefix("SSHub-") }
interface BleChannel {
    val incoming: Flow<ByteArray>
    suspend fun send(frame: ByteArray): Boolean
    fun close()
}

/** FSK playback only; the app never records (no RECORD_AUDIO). Uses the user-set volume. */
interface AudioPort {
    suspend fun play(pcm: ShortArray, rate: Int)
    fun stop()
    val playing: StateFlow<Boolean>
}

interface SystemPort {
    /** Signing-certificate SHA-256 of this APK, 64 lowercase hex digits (docs section 7). */
    val apkCertSha256: String
    val versionName: String
    /** Bundled licence texts (assets/licenses). */
    fun licenceText(name: String): String
    /** Settings.Global animator scale 0 or "Remove animations". */
    val reducedMotion: Boolean
}
