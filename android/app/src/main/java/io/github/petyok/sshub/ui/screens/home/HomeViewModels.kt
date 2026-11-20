package io.github.petyok.sshub.ui.screens.home

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import io.github.petyok.sshub.core.CoreException
import io.github.petyok.sshub.core.DeviceIdScanned
import io.github.petyok.sshub.core.HomeApi
import io.github.petyok.sshub.core.IdentityApi
import io.github.petyok.sshub.core.Opened
import io.github.petyok.sshub.core.Platform
import io.github.petyok.sshub.core.CameraPermission
import io.github.petyok.sshub.core.ScanApi
import io.github.petyok.sshub.core.ScanEvent
import io.github.petyok.sshub.core.ScanPurpose
import io.github.petyok.sshub.core.ScanResult
import io.github.petyok.sshub.platform.AndroidCamera
import io.github.petyok.sshub.core.ScanResults

/** The "last sync" age is relative, so it is recomputed once a minute while the screen is alive. Nothing here acts on its own. */
class HomeViewModel(
    private val core: HomeApi,
    private val platform: Platform,
    /** `SettingsStore.updateNeededAfter`: set when a removal was approved, cleared by the reshare adopt. */
    updateNeeded: StateFlow<String?> = MutableStateFlow(null),
) : ViewModel() {
    private val _state = MutableStateFlow<HomeState?>(null)
    /** Null while the core refuses to answer (locked): the shell moves to the Lock screen on the vault state, nothing is drawn. */
    val state: StateFlow<HomeState?> = combine(_state, updateNeeded) { s, u -> s?.copy(updateNeededAfter = u) }
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    init {
        refresh()
        viewModelScope.launch { while (isActive) { delay(60_000); refresh() } }
    }

    /** The core is read off the main thread (the adapter's dispatcher); the state follows when it answers. */
    fun refresh() { viewModelScope.launch { _state.value = read(_state.value) } }

    /** A locked core throws; a refused read keeps what was shown. */
    private suspend fun read(previous: HomeState?): HomeState? {
        val now = platform.clock.nowSeconds()
        return try {
            // `headPending` comes from the status, not from the ring: a restored phone has no ring and is still pending (H5).
            // The real core and the fake are both IdentityApi; a HomeApi alone falls back to the ring's flag.
            val ring = core.ring(now)
            val pending = (core as? IdentityApi)?.status(now)?.headPending ?: ring?.headPending ?: false
            HomeState(core.thisPhone(now), ring, core.pending(now), pending)
        } catch (e: CoreException) {
            previous
        }
    }
}

class ApprovalsViewModel(private val core: HomeApi, private val platform: Platform) : ViewModel() {
    private val _state = MutableStateFlow(read(null))
    val state: StateFlow<ApprovalsState> = _state.asStateFlow()

    fun refresh() { _state.update { read(it.selected) } }
    fun select(i: Int) { _state.update { it.copy(selected = i) } }

    /** First segment that has something, unless the user already chose one. */
    private fun read(selected: Int?): ApprovalsState {
        val items = try { core.pending(platform.clock.nowSeconds()) } catch (e: CoreException) { emptyList() } // locked: the shell shows Lock
        val first = ApprovalTab.entries.indexOfFirst { t -> items.any { it.kind.tab() == t } }.coerceAtLeast(0)
        return ApprovalsState(items, selected ?: first)
    }
}

fun scanCameraOf(permission: CameraPermission, hasCamera: Boolean): ScanCamera = when {
    !hasCamera -> ScanCamera.None
    permission == CameraPermission.Granted -> ScanCamera.Live
    permission == CameraPermission.Denied -> ScanCamera.Denied
    permission == CameraPermission.DeniedForever -> ScanCamera.DeniedForever
    else -> ScanCamera.Rationale
}

fun scanTitle(p: ScanPurpose) = when (p) {
    ScanPurpose.Request -> "Scan a share request"
    ScanPurpose.EnrollStep -> "Scan the terminal code"
    ScanPurpose.Chain -> "Take my chain from a terminal"
    ScanPurpose.Recovery -> "Scan a recovery notice"
    ScanPurpose.Reshare -> "Adopt a reshare"
    ScanPurpose.DeviceId -> "Scan the device id"
}

fun scanHint(p: ScanPurpose) = when (p) {
    ScanPurpose.Request -> "Open the unlock screen on the terminal and point the camera at its QR code."
    ScanPurpose.EnrollStep -> "Point the camera at the QR code the terminal shows."
    ScanPurpose.Chain -> "A terminal shows this phone's own newest ops as a QR code or as text (an ops list)."
    ScanPurpose.Recovery -> "A terminal can show a recovery notice as a QR code."
    ScanPurpose.Reshare -> "Scan the reshare ops a terminal shows for this phone."
    ScanPurpose.DeviceId -> "Open the device details on the terminal and point the camera at its QR code."
}

const val SCAN_NOT_A_DEVICE_ID = "That is not a device id code from SSHub."

/**
 * The device id out of a terminal QR: ASCII `sshub:dev:1:<16 hex>`, upper or lower case, with spaces or dashes inside the hex.
 * Returns the 16 lowercase hex digits or null for anything else.
 */
fun parseDeviceIdPayload(bytes: ByteArray): String? {
    if (bytes.any { it < 0 }) return null
    val text = String(bytes, Charsets.US_ASCII).trim().lowercase()
    val prefix = "sshub:dev:1:"
    if (!text.startsWith(prefix)) return null
    val id = text.removePrefix(prefix).filter { it != ' ' && it != '-' }
    return id.takeIf { it.length == 16 && it.all { c -> c in '0'..'9' || c in 'a'..'f' } }
}

/**
 * Camera frames arrive on the analysis thread (`onLuma`); state is a StateFlow so that is safe. A finished code (camera or paste) ends in
 * the `done` state and waits for the user: only `finish()` hands the result on.
 */
class ScanViewModel(private val core: ScanApi, private val platform: Platform, val purpose: ScanPurpose) : ViewModel() {
    private class Local(
        val have: Int = 0, val need: Int = 0, val ignored: String? = null, val done: Boolean = false,
        val paste: Boolean = false, val pasteText: String = "", val pasteError: String? = null,
    )

    private val local = MutableStateFlow(Local())
    private var opened: Opened? = null
    @Volatile private var busyFlag = false

    /** While a frame is being opened the camera delivers no more frames to the analysis (`AndroidCamera.wantsFrames`). */
    private var busy: Boolean
        get() = busyFlag
        set(v) { busyFlag = v; (platform.camera as? AndroidCamera)?.wantsFrames = !v }

    private fun build(l: Local, p: CameraPermission) = ScanState(
        title = scanTitle(purpose), hint = scanHint(purpose), camera = scanCameraOf(p, platform.camera.hasCamera),
        have = l.have, need = l.need, ignored = l.ignored, done = l.done,
        paste = l.paste, pasteText = l.pasteText, pasteError = l.pasteError,
    )

    val state: StateFlow<ScanState> = combine(local, platform.camera.permission, ::build)
        .stateIn(viewModelScope, SharingStarted.Eagerly, build(local.value, platform.camera.permission.value))

    init { core.resetScan() } // queued on the core's thread, returns at once

    fun onLuma(width: Int, height: Int, luma: ByteArray) {
        if (busy) return
        when (val e = core.scanGray(width, height, luma)) {
            ScanEvent.None -> {}
            ScanEvent.Foreign -> local.update { Local(it.have, it.need, SCAN_FOREIGN_TEXT, false, it.paste, it.pasteText, it.pasteError) }
            is ScanEvent.Progress -> local.update { Local(e.have, e.need, null, false, it.paste, it.pasteText, it.pasteError) }
            is ScanEvent.Frame -> { busy = true; viewModelScope.launch { open(e.bytes) } }
        }
    }

    fun grant() = platform.camera.requestPermission()
    fun openSettings() = platform.camera.openAppSettings()
    fun togglePaste() = local.update { Local(it.have, it.need, it.ignored, it.done, !it.paste, it.pasteText, null) }
    fun pasteText(t: String) = local.update { Local(it.have, it.need, it.ignored, it.done, it.paste, t, null) }
    fun pasteFromClipboard() {
        val t = platform.clipboard.readText() ?: return
        pasteText(t)
    }

    fun usePasted() {
        if (busy) return
        busy = true
        viewModelScope.launch {
            val text = local.value.pasteText
            val bytes = if (purpose == ScanPurpose.DeviceId) text.toByteArray(Charsets.UTF_8) else core.frameFromText(text)
            if (bytes == null) {
                local.update { Local(it.have, it.need, it.ignored, it.done, it.paste, it.pasteText, SCAN_BAD_TEXT) }
                busy = false
            } else open(bytes)
        }
    }

    private suspend fun open(bytes: ByteArray) {
        // A device id is plain text, not a core document: the core is never asked about it (it re-checks the typed id itself).
        val o = if (purpose == ScanPurpose.DeviceId) {
            parseDeviceIdPayload(bytes)?.let { DeviceIdScanned(it) } ?: Opened.Ignored(SCAN_NOT_A_DEVICE_ID)
        } else try {
            core.openFrame(bytes, platform.clock.nowSeconds())
        } catch (e: CoreException) {
            Opened.Ignored(e.message ?: "Code not recognised")
        }
        if (o is Opened.Ignored) {
            core.resetScan()
            local.update { Local(0, 0, o.reason, false, it.paste, it.pasteText, null) }
            busy = false
        } else {
            opened = o
            local.update { Local(it.need, it.need, null, true, it.paste, it.pasteText, null) }
        }
    }

    /** Publishes the opened result for the caller and returns it; null when nothing was read. */
    fun finish(): Opened? {
        val o = opened ?: return null
        ScanResults.publish(ScanResult(purpose, o))
        return o
    }

    override fun onCleared() { core.resetScan(); (platform.camera as? AndroidCamera)?.wantsFrames = true }
}
