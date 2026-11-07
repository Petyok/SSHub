package uk.petruha.sshub.phone.ui.screens.enroll

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import androidx.navigation.NavHostController
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.isActive
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import uk.petruha.sshub.phone.platform.AndroidAudio
import uk.petruha.sshub.phone.platform.AudioStatus
import uk.petruha.sshub.phone.platform.BleChannelState
import uk.petruha.sshub.phone.platform.ObservedBleChannel
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.EnrollChannel
import uk.petruha.sshub.phone.core.EnrollDecision
import uk.petruha.sshub.phone.core.EnrollRefusal
import uk.petruha.sshub.phone.core.EnrollStage
import uk.petruha.sshub.phone.core.EnrollTextKind
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.OpenedEnroll
import uk.petruha.sshub.phone.core.OpenedEnrolled
import uk.petruha.sshub.phone.core.Outgoing
import uk.petruha.sshub.phone.core.PhoneCoreApi
import uk.petruha.sshub.phone.core.RefusalKind
import uk.petruha.sshub.phone.core.Platform
import uk.petruha.sshub.phone.core.Routes
import uk.petruha.sshub.phone.core.ScanEvent
import uk.petruha.sshub.phone.core.VaultState

/**
 * One enrollment run across the nine screens. Holds the core and the platform ports, never decides anything: frames go to
 * `core.openFrame`, the user's two decisions go to `core.enrollDecide`, the result is shown as it comes back.
 * No timer here approves, continues or declines (D11); the only timers are the BLE search timeout and the sound progress display.
 */
class EnrollViewModel(
    private val core: PhoneCoreApi,
    private val platform: Platform,
    private val nav: NavHostController,
    initialName: String,
) : ViewModel() {
    private val flow = MutableStateFlow(EnrollFlow(name = initialName, nameProblem = core.nameProblem(initialName.trim())))

    val ui: StateFlow<EnrollUi> = combine(
        flow, platform.camera.permission, platform.clipboard.clearsInSeconds, platform.ble.availability,
    ) { f, cam, clip, ble -> EnrollUi(f, cam, clip, ble) }.stateIn(
        viewModelScope, SharingStarted.Eagerly,
        EnrollUi(flow.value, platform.camera.permission.value, platform.clipboard.clearsInSeconds.value, platform.ble.availability.value),
    )

    private var lastFrame: ByteArray? = null
    private var link: ObservedBleChannel? = null
    private val audio = platform.audio as? AndroidAudio
    private var linkJob: Job? = null
    private var connectJob: Job? = null
    private var scanJob: Job? = null
    private var soundJob: Job? = null

    init {
        // The core drops its enrollment when it locks; the flow shown here must not outlive it.
        viewModelScope.launch {
            platform.keys.state.collect { if (it != VaultState.Unlocked) runCatching { restart() } }
        }
        // Why a scan could not start, as the system's sentence, shown in the device list.
        viewModelScope.launch { platform.ble.scanError.collect { e -> flow.update { it.copy(btScanError = e?.text) } } }
        audio?.let { a ->
            viewModelScope.launch { a.status.collect { s -> flow.update { it.copy(soundProblem = (s as? AudioStatus.Failed)?.reason) } } }
            viewModelScope.launch { a.mediaVolumeZero.collect { z -> flow.update { it.copy(volumeZero = z) } } }
        }
    }

    private fun now() = platform.clock.nowSeconds()
    private fun note(text: String?) = flow.update { it.copy(note = text) }

    // ---- lifecycle of a run ----------------------------------------------------------------------------------------

    /** Forgets everything of a run except the name and the channel. Entering step 1 always starts a fresh exchange. */
    fun restart() {
        closeLink(); soundStop(); scanJob?.cancel()
        lastFrame = null
        core.resetScan()
        flow.update {
            EnrollFlow(
                channel = it.channel, name = it.name, nameProblem = it.nameProblem, btTag = it.btTag,
                btScanError = it.btScanError, soundProblem = it.soundProblem, volumeZero = it.volumeZero,
            )
        }
    }

    fun startAgain() {
        restart()
        nav.navigate(Routes.Enroll1) { popUpTo(Routes.Enroll1) { inclusive = true } }
    }

    fun finish() {
        restart()
        nav.navigate(Routes.Home) { popUpTo(Routes.Enroll1) { inclusive = true }; launchSingleTop = true }
    }

    fun back() { nav.popBackStack() }
    fun go(route: String) { nav.navigate(route) }

    // ---- step 1 ----------------------------------------------------------------------------------------------------

    fun setName(n: String) = flow.update { it.copy(name = n, nameProblem = core.nameProblem(n.trim())) }
    fun setChannel(c: EnrollChannel) = flow.update { it.copy(channel = c) }

    fun startScan() {
        core.resetScan(); lastFrame = null
        flow.update { it.copy(scanning = true, have = 0, need = 0, note = null) }
    }

    fun cancelScan() = flow.update { it.copy(scanning = false, note = null) }

    fun startPaste() { flow.update { it.copy(pasteN = 1) }; nav.navigate(Routes.EnrollPaste) }

    fun requestCamera() = platform.camera.requestPermission()
    fun openCameraSettings() = platform.camera.openAppSettings()

    /** Analysis thread. Only the QR and fountain decoding happens here; everything else is launched. */
    fun onLuma(w: Int, h: Int, luma: ByteArray) {
        when (val ev = core.scanGray(w, h, luma)) {
            is ScanEvent.Progress -> flow.update { it.copy(have = ev.have, need = ev.need, note = null) }
            ScanEvent.Foreign -> note("That code is not an SSHub code.")
            is ScanEvent.Frame -> {
                val b = ev.bytes
                if (lastFrame?.contentEquals(b) != true) { lastFrame = b; viewModelScope.launch { receive(b) } }
            }
            ScanEvent.None -> {}
        }
    }

    // ---- frames from any channel -----------------------------------------------------------------------------------

    /** `viaLink`: the frame came over Bluetooth; a frame the core refuses is reported to the channel (a hostile peripheral gets cut off). */
    private suspend fun receive(bytes: ByteArray, viaLink: Boolean = false) {
        try {
            val o = core.openFrame(bytes, now())
            handle(o)
            if (viaLink && (o is Opened.Ignored || o is EnrollRefusal)) link?.frameRefused()
        } catch (e: CoreException) {
            note(e.message ?: "The frame was not accepted.")
            if (viaLink) link?.frameRefused()
        }
        core.resetScan()
        flow.update { it.copy(have = 0, need = 0) }
    }

    private suspend fun handle(o: Opened) {
        val f = flow.value
        when (o) {
            is OpenedEnroll -> when {
                o.stage == EnrollStage.Commit && f.commit == null -> onCommit(o)
                o.stage == EnrollStage.Reveal && f.commit != null && f.reveal == null -> onReveal(o)
                else -> note("Not what this step needs: this is the step ${if (o.stage == EnrollStage.Commit) 1 else 3} frame.")
            }
            is OpenedEnrolled -> if (f.confirm != null) onEnrolled(o) else note("Not what this step needs: this is the step 5 frame.")
            is EnrollRefusal -> onRefusal(o)
            is Opened.Ignored -> note(o.reason)
            else -> note("That is not an enrollment frame.")
        }
    }

    private suspend fun onCommit(o: OpenedEnroll) {
        val out = core.enrollDecide(o.token, EnrollDecision.Name(flow.value.name.trim()), now())
        flow.update { it.copy(commit = out, terminal = o.terminalName, terminalId = o.terminalId, scanning = false, note = null, btConnecting = false, soundStep = 2) }
        sendLink(out)
        when (flow.value.channel) {
            EnrollChannel.Paste -> flow.update { it.copy(pasteN = 2) }
            EnrollChannel.Sound -> nav.navigate(Routes.EnrollSound)
            else -> nav.navigate(Routes.Enroll2)
        }
    }

    private suspend fun onReveal(o: OpenedEnroll) {
        flow.update {
            it.copy(
                reveal = o.outgoing, revealToken = o.token, sasWords = o.sasWords, terminal = o.terminalName.ifEmpty { it.terminal ?: "" }.ifEmpty { null },
                terminalId = o.terminalId, keys = KeysStatus.Ok, note = null,
            )
        }
        o.outgoing?.let { sendLink(it) }
        if (flow.value.channel == EnrollChannel.Paste) nav.navigate(Routes.Enroll4)
    }

    /** The core keeps the session after a wrong-order or stray frame; so does the screen: a note, nothing reset (M7). */
    private fun onRefusal(o: EnrollRefusal) {
        if (o.kind == RefusalKind.WrongOrder || o.kind == RefusalKind.StrayFrame) {
            note("That is not the code this step needs. Show the one the terminal shows now.")
            return
        }
        val f = flow.value
        when {
            f.confirm != null -> {
                flow.update { it.copy(conf = ConfirmStatus.Bad, confReason = o.reason, confKind = o.kind) }
                if (f.channel == EnrollChannel.Paste) nav.navigate(Routes.Enroll5)
            }
            f.commit != null -> {
                val mismatch = o.kind == RefusalKind.Mismatch
                flow.update {
                    it.copy(
                        keys = if (mismatch) KeysStatus.Mismatch else KeysStatus.Stopped,
                        committed = o.committed.orEmpty(), shown = o.shown.orEmpty(), keysReason = o.reason,
                    )
                }
                if (f.channel == EnrollChannel.Paste) nav.navigate(Routes.Enroll3)
            }
            else -> note(o.reason)
        }
    }

    private fun onEnrolled(o: OpenedEnrolled) {
        flow.update { it.copy(conf = ConfirmStatus.Ok, enrolled = o, note = null) }
        if (flow.value.channel == EnrollChannel.Paste) nav.navigate(Routes.EnrollDone)
    }

    // ---- step 4 ----------------------------------------------------------------------------------------------------

    fun decideSas(matches: Boolean) {
        val token = flow.value.revealToken ?: return
        viewModelScope.launch {
            try {
                val out = core.enrollDecide(token, EnrollDecision.Sas(matches), now())
                flow.update { if (matches) it.copy(cmp = ComparePhase.Match, confirm = out) else it.copy(cmp = ComparePhase.Differ, declined = out) }
                sendLink(out)
            } catch (e: CoreException) { note(e.message ?: "The decision was not accepted.") }
        }
    }

    fun continueToConfirm() {
        if (flow.value.channel == EnrollChannel.Paste) { flow.update { it.copy(pasteN = 5) }; nav.navigate(Routes.EnrollPaste) }
        else nav.navigate(Routes.Enroll5)
    }

    // ---- shared text and copy --------------------------------------------------------------------------------------

    fun toggleText(step: Int) = flow.update { it.copy(textOpen = if (step in it.textOpen) it.textOpen - step else it.textOpen + step) }

    fun copy(o: Outgoing?) { if (o != null) platform.clipboard.copySensitive("SSHub enrollment text", o.text) }
    fun copyFor(step: Int) = copy(
        when (step) { 2 -> flow.value.commit; 4 -> if (flow.value.cmp == ComparePhase.Differ) flow.value.declined else flow.value.reveal; else -> flow.value.confirm },
    )

    // ---- paste -----------------------------------------------------------------------------------------------------

    private fun setPaste(text: String) {
        val n = flow.value.pasteN
        flow.update { it.copy(pasteText = it.pasteText + (n to text)) }
        // The kind is read from the core's thread; the text shows at once.
        viewModelScope.launch { val kind = core.enrollTextKind(text); flow.update { it.copy(pasteKind = it.pasteKind + (n to kind)) } }
    }

    fun setPasteText(t: String) = setPaste(t)
    fun pasteFromClipboard() = setPaste(platform.clipboard.readText().orEmpty())
    fun pasteClear() = setPaste("")

    fun pasteUse() {
        val n = flow.value.pasteN
        viewModelScope.launch {
            val bytes = core.frameFromText(flow.value.pasteText[n].orEmpty())
            if (bytes == null) flow.update { it.copy(pasteKind = it.pasteKind + (n to EnrollTextKind.NotEnrollment)) } else receive(bytes)
        }
    }

    fun pasteNext() { flow.update { it.copy(pasteN = 3) } }

    fun pasteBack() {
        when (flow.value.pasteN) {
            1 -> nav.popBackStack()
            2 -> { restart(); flow.update { it.copy(pasteN = 1) } }
            3 -> flow.update { it.copy(pasteN = 2) }
            else -> nav.navigate(Routes.Enroll4)
        }
    }

    // ---- Bluetooth -------------------------------------------------------------------------------------------------

    fun btStart() {
        scanJob?.cancel()
        flow.update { it.copy(btDevices = emptyList(), btSel = -1, btTimedOut = false, btFailed = false) }
        scanJob = viewModelScope.launch {
            launch {
                try {
                    platform.ble.scan().collect { list ->
                        flow.update { f ->
                            val keep = f.btDevices.getOrNull(f.btSel)?.address
                            f.copy(btDevices = list, btSel = if (keep == null) -1 else list.indexOfFirst { it.address == keep })
                        }
                    }
                } catch (e: CancellationException) { throw e } catch (e: Exception) { /* scan failures show as "none" after the timeout */ }
            }
            delay(10_000)
            flow.update { it.copy(btTimedOut = true) }
        }
    }

    fun btStop() { scanJob?.cancel() }

    fun btSetTag(t: String) = flow.update { it.copy(btTag = t.uppercase().filter { c -> c in 'A'..'Z' || c in '0'..'9' }.take(4)) }
    fun btSelect(i: Int) = flow.update { it.copy(btSel = i) }
    fun btAllow() = platform.ble.requestPermissions()

    fun btPair() {
        val f = flow.value
        val d = f.btDevices.getOrNull(f.btSel) ?: return
        if (!d.tag.equals(f.btTag, ignoreCase = true)) return
        // Two entries with the tag mean stop (L8): neither is paired.
        if (f.btDevices.count { it.tag.equals(f.btTag, ignoreCase = true) } > 1) return
        scanJob?.cancel()
        flow.update { it.copy(btConnecting = true, btFailed = false, channel = EnrollChannel.Bluetooth) }
        connectJob?.cancel()
        connectJob = viewModelScope.launch {
            val ch = platform.ble.connect(d)
            // Cancelled while connecting: a channel that comes back late is closed, never kept.
            if (!isActive) { ch?.close(); return@launch }
            if (ch == null) { flow.update { it.copy(btConnecting = false, btFailed = true) }; return@launch }
            link = ch
            linkJob = launch {
                ch.incoming.collect { receive(it, viaLink = true) }
                // `incoming` completed: the session is over and the state says why (BYE, timeout, link lost, Bluetooth off, flood).
                val why = (ch.state.value as? BleChannelState.Closed)?.reason ?: "The Bluetooth link ended."
                flow.update { it.copy(btLinkEnded = why, btConnecting = false, note = why) }
            }
        }
    }

    fun btCancel() { closeLink(); flow.update { it.copy(btConnecting = false, btFailed = false) }; nav.popBackStack() }

    private suspend fun sendLink(o: Outgoing) {
        val ch = link ?: return
        val sent = ch.send(o.frame)
        // The terminal's own refusal sentence (busy, too large, ...) wins over a generic one.
        val refused = ch.sendFailure()
        if (refused != null) note(refused) else if (!sent) note("The frame could not be sent over Bluetooth.")
    }

    private fun closeLink() { connectJob?.cancel(); connectJob = null; linkJob?.cancel(); linkJob = null; link?.close(); link = null }

    // ---- sound (this phone only plays) -----------------------------------------------------------------------------

    fun soundProfile(p: SoundProfile) = flow.update { if (it.soundPhase == SoundPhase.Playing) it else it.copy(soundProfile = p, soundPhase = SoundPhase.Idle, soundElapsed = 0) }

    fun soundPlay(step: Int) {
        val f = flow.value
        val o = when (step) { 2 -> f.commit; 4 -> if (f.cmp == ComparePhase.Differ) f.declined else f.reveal; else -> f.confirm } ?: return
        val pcm = (if (f.soundProfile == SoundProfile.Fast) o.audioFast else o.audioSlow) ?: return
        val total = ((pcm.size + o.audioRate - 1) / o.audioRate).coerceAtLeast(1)
        soundJob?.cancel()
        flow.update { it.copy(soundStep = step, soundPhase = SoundPhase.Playing, soundElapsed = 0, soundTotal = total) }
        soundJob = viewModelScope.launch {
            val ticker = launch { while (true) { delay(1000); flow.update { it.copy(soundElapsed = minOf(it.soundElapsed + 1, it.soundTotal)) } } }
            try { platform.audio.play(pcm, o.audioRate) } finally { ticker.cancel() }
            flow.update { it.copy(soundPhase = SoundPhase.Done, soundElapsed = it.soundTotal) }
        }
    }

    /** The sound screen opened: the media volume may have changed since the last look. */
    fun soundOpened() { audio?.refreshVolume() }

    fun soundStop() {
        soundJob?.cancel(); soundJob = null
        platform.audio.stop()
        flow.update { if (it.soundPhase == SoundPhase.Idle) it else it.copy(soundPhase = SoundPhase.Idle, soundElapsed = 0) }
    }

    override fun onCleared() { closeLink(); soundJob?.cancel(); scanJob?.cancel() }
}
