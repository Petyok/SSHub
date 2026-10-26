package uk.petruha.sshub.phone.ui.screens.home

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
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.HomeApi
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.Platform
import uk.petruha.sshub.phone.core.CameraPermission
import uk.petruha.sshub.phone.core.ScanApi
import uk.petruha.sshub.phone.core.ScanEvent
import uk.petruha.sshub.phone.core.ScanPurpose
import uk.petruha.sshub.phone.core.ScanResult
import uk.petruha.sshub.phone.core.ScanResults

/** The "last sync" age is relative, so it is recomputed once a minute while the screen is alive. Nothing here acts on its own. */
class HomeViewModel(private val core: HomeApi, private val platform: Platform) : ViewModel() {
    private val _state = MutableStateFlow(read())
    val state: StateFlow<HomeState> = _state.asStateFlow()

    init {
        viewModelScope.launch { while (isActive) { delay(60_000); refresh() } }
    }

    fun refresh() { _state.value = read() }

    private fun read(): HomeState {
        val now = platform.clock.nowSeconds()
        return HomeState(core.thisPhone(now), core.ring(now), core.pending(now))
    }
}

class ApprovalsViewModel(private val core: HomeApi, private val platform: Platform) : ViewModel() {
    private val _state = MutableStateFlow(read(null))
    val state: StateFlow<ApprovalsState> = _state.asStateFlow()

    fun refresh() { _state.update { read(it.selected) } }
    fun select(i: Int) { _state.update { it.copy(selected = i) } }

    /** First segment that has something, unless the user already chose one. */
    private fun read(selected: Int?): ApprovalsState {
        val items = core.pending(platform.clock.nowSeconds())
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
}

fun scanHint(p: ScanPurpose) = when (p) {
    ScanPurpose.Request -> "Open the unlock screen on the terminal and point the camera at its QR code."
    ScanPurpose.EnrollStep -> "Point the camera at the QR code the terminal shows."
    ScanPurpose.Chain -> "A terminal shows this phone's own newest ops as a QR code or as text (an ops list)."
    ScanPurpose.Recovery -> "A terminal can show a recovery notice as a QR code."
    ScanPurpose.Reshare -> "Scan the reshare ops a terminal shows for this phone."
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
    @Volatile private var busy = false

    private fun build(l: Local, p: CameraPermission) = ScanState(
        title = scanTitle(purpose), hint = scanHint(purpose), camera = scanCameraOf(p, platform.camera.hasCamera),
        have = l.have, need = l.need, ignored = l.ignored, done = l.done,
        paste = l.paste, pasteText = l.pasteText, pasteError = l.pasteError,
    )

    val state: StateFlow<ScanState> = combine(local, platform.camera.permission, ::build)
        .stateIn(viewModelScope, SharingStarted.Eagerly, build(local.value, platform.camera.permission.value))

    init { core.resetScan() }

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
        val bytes = core.frameFromText(local.value.pasteText)
        if (bytes == null) { local.update { Local(it.have, it.need, it.ignored, it.done, it.paste, it.pasteText, SCAN_BAD_TEXT) }; return }
        busy = true
        viewModelScope.launch { open(bytes) }
    }

    private suspend fun open(bytes: ByteArray) {
        val o = try { core.openFrame(bytes, platform.clock.nowSeconds()) } catch (e: CoreException) { Opened.Ignored(e.message ?: "Code not recognised") }
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

    override fun onCleared() { core.resetScan() }
}
