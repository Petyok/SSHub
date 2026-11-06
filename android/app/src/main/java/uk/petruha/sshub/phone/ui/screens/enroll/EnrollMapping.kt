package uk.petruha.sshub.phone.ui.screens.enroll

import uk.petruha.sshub.phone.core.BleAvailability
import uk.petruha.sshub.phone.core.BleDevice
import uk.petruha.sshub.phone.core.CameraPermission
import uk.petruha.sshub.phone.core.EnrollChannel
import uk.petruha.sshub.phone.core.RefusalKind
import uk.petruha.sshub.phone.core.EnrollTextKind
import uk.petruha.sshub.phone.core.OpenedEnrolled
import uk.petruha.sshub.phone.core.Outgoing

/** Everything the nine screens derive from, in one immutable value (the view model's `ui`). */
data class EnrollFlow(
    val channel: EnrollChannel = EnrollChannel.Qr,
    val name: String = "",
    val nameProblem: String? = null,
    val terminal: String? = null,
    val terminalId: String = "",
    val commit: Outgoing? = null,
    val reveal: Outgoing? = null,
    val confirm: Outgoing? = null,
    val declined: Outgoing? = null,
    val sasWords: List<String> = emptyList(),
    val revealToken: Long? = null,
    val scanning: Boolean = false,
    val have: Int = 0,
    val need: Int = 0,
    val note: String? = null,
    val keys: KeysStatus = KeysStatus.Waiting,
    val committed: String = "",
    val shown: String = "",
    val keysReason: String = "",
    val cmp: ComparePhase = ComparePhase.Compare,
    val conf: ConfirmStatus = ConfirmStatus.Waiting,
    val confReason: String = "",
    val confKind: RefusalKind? = null,
    val enrolled: OpenedEnrolled? = null,
    val pasteN: Int = 1,
    val pasteText: Map<Int, String> = emptyMap(),
    val pasteKind: Map<Int, EnrollTextKind> = emptyMap(),
    val textOpen: Set<Int> = setOf(2),
    val btTag: String = "",
    val btDevices: List<BleDevice> = emptyList(),
    val btSel: Int = -1,
    val btTimedOut: Boolean = false,
    val btConnecting: Boolean = false,
    val btFailed: Boolean = false,
    /** The system's sentence for a scan that could not start; the reason a Bluetooth link ended (BYE, timeout, link lost, ...). */
    val btScanError: String? = null,
    val btLinkEnded: String? = null,
    /** AudioTrack failed (the reason), and the media volume is zero: both shown on the sound screen. */
    val soundProblem: String? = null,
    val volumeZero: Boolean = false,
    val soundProfile: SoundProfile = SoundProfile.Fast,
    val soundPhase: SoundPhase = SoundPhase.Idle,
    val soundElapsed: Int = 0,
    val soundTotal: Int = 0,
    val soundStep: Int = 2,
)

data class EnrollUi(
    val flow: EnrollFlow,
    val camera: CameraPermission,
    val clipboardClearsIn: Int?,
    val ble: BleAvailability,
)

private fun EnrollFlow.who() = terminal ?: "the terminal"

private fun secondsOf(o: Outgoing?, p: SoundProfile): Int? {
    val out = o ?: return null
    val pcm = (if (p == SoundProfile.Fast) out.audioFast else out.audioSlow) ?: return null
    return ((pcm.size + out.audioRate - 1) / out.audioRate).coerceAtLeast(1)
}

private fun EnrollUi.sendOf(o: Outgoing?, step: Int): EnrollSendState {
    val f = flow
    val secs = secondsOf(o, f.soundProfile)
    val playing = f.soundStep == step && f.soundPhase != SoundPhase.Idle
    return EnrollSendState(
        channel = f.channel, terminal = f.who(), text = o?.text.orEmpty(), qr = o?.qr.orEmpty(),
        textOpen = step in f.textOpen, clipboardClearsIn = clipboardClearsIn,
        sound = EnrollSoundState(
            profile = f.soundProfile,
            phase = if (playing) f.soundPhase else SoundPhase.Idle,
            elapsed = if (playing) f.soundElapsed else 0,
            total = if (playing && f.soundTotal > 0) f.soundTotal else secs ?: if (f.soundProfile == SoundProfile.Fast) 8 else 16,
            available = secs != null,
        ),
    )
}

private fun EnrollUi.receiveOf(prompt: String): EnrollReceiveState = EnrollReceiveState(
    channel = flow.channel, terminal = flow.who(), tag = flow.btTag, have = flow.have, need = flow.need,
    camera = camera, note = flow.note, prompt = prompt,
)

fun EnrollUi.toScan() = EnrollScanState(
    name = flow.name, nameProblem = flow.nameProblem, channel = flow.channel, terminal = flow.who(), scanning = flow.scanning,
    receive = receiveOf("Scan the terminal's short code"),
)

fun EnrollUi.toShow() = EnrollShowState(flow.who(), sendOf(flow.commit, 2))

fun EnrollUi.toKeys() = EnrollKeysState(
    terminal = flow.who(), terminalId = flow.terminalId, status = flow.keys, receive = receiveOf("Scan the terminal's keys"),
    committed = flow.committed, shown = flow.shown, reason = flow.keysReason,
)

fun EnrollUi.toCompare() = EnrollCompareState(
    terminal = flow.who(), terminalId = flow.terminalId, channel = flow.channel, words = flow.sasWords, phase = flow.cmp,
    send = if (flow.cmp == ComparePhase.Differ) sendOf(flow.declined, 4) else sendOf(flow.reveal, 4),
)

fun EnrollUi.toConfirm() = EnrollConfirmState(
    terminal = flow.who(), status = flow.conf, reason = flow.confReason, kind = flow.confKind, send = sendOf(flow.confirm, 5),
    receive = receiveOf("Scan the terminal's confirmation"),
)

fun EnrollUi.toEnrolled(): EnrolledState? = flow.enrolled?.let { EnrolledState(it.terminalName, it.terminalId, it.phoneName, it.phoneId) }

fun EnrollUi.toBt(): EnrollBtState {
    val f = flow
    val phase = when {
        ble == BleAvailability.Off -> BtPhase.Off
        ble == BleAvailability.PermissionNeeded || ble == BleAvailability.PermissionDenied -> BtPhase.Denied
        ble == BleAvailability.Unsupported -> BtPhase.Unsupported
        f.btConnecting -> BtPhase.Connecting
        f.btFailed -> BtPhase.LinkFailed
        f.btDevices.isNotEmpty() -> BtPhase.Found
        f.btTimedOut -> BtPhase.None
        else -> BtPhase.Searching
    }
    return EnrollBtState(f.who(), f.btTag, phase, f.btDevices, f.btSel, problem = f.btLinkEnded ?: f.btScanError)
}

internal fun pasteWhat(n: Int): String = when (n) {
    1 -> "the terminal's short code"
    3 -> "the terminal's keys"
    else -> "the terminal's confirmation"
}

internal fun validatePaste(n: Int, kind: EnrollTextKind?): PasteValidation {
    val expected = when (n) { 1 -> EnrollTextKind.Step1Code; 3 -> EnrollTextKind.Step3Keys; else -> EnrollTextKind.Step5Confirm }
    return when {
        kind == null || kind == EnrollTextKind.Empty -> PasteValidation(PasteVerdict.Empty, "Waiting for text. Step $n needs ${pasteWhat(n)}.")
        kind == expected -> PasteValidation(PasteVerdict.Ok, "Looks like step $n frame.")
        else -> PasteValidation(
            PasteVerdict.Bad,
            "Not what this step needs: " + when (kind) {
                EnrollTextKind.Step1Code -> "this looks like the step 1 frame."
                EnrollTextKind.Step3Keys -> "this looks like the step 3 frame."
                EnrollTextKind.Step5Confirm -> "this looks like the step 5 frame."
                EnrollTextKind.OwnText -> "this is the text from this phone."
                else -> "it is not an enrollment frame."
            },
        )
    }
}

fun EnrollUi.toPaste(): EnrollPasteState {
    val n = flow.pasteN
    return EnrollPasteState(
        n = n, terminal = flow.who(), text = flow.pasteText[n].orEmpty(), validation = validatePaste(n, flow.pasteKind[n]), note = flow.note,
        send = when (n) { 2 -> sendOf(flow.commit, 2); 5 -> sendOf(flow.confirm, 5); else -> null },
    )
}

fun EnrollUi.toSound() = EnrollSoundScreenState(
    step = flow.soundStep, terminal = flow.who(),
    send = sendOf(when (flow.soundStep) { 2 -> flow.commit; 4 -> flow.reveal; else -> flow.confirm }, flow.soundStep),
    problem = flow.soundProblem, volumeZero = flow.volumeZero,
)
