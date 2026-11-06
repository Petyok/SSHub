package uk.petruha.sshub.phone.fake

import uk.petruha.sshub.phone.core.BleDevice
import uk.petruha.sshub.phone.core.CameraPermission
import uk.petruha.sshub.phone.core.EnrollApi
import uk.petruha.sshub.phone.core.EnrollChannel
import uk.petruha.sshub.phone.core.EnrollDecision
import uk.petruha.sshub.phone.core.EnrollRefusal
import uk.petruha.sshub.phone.core.EnrollStage
import uk.petruha.sshub.phone.core.EnrollTextKind
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.OpenedEnroll
import uk.petruha.sshub.phone.core.OpenedEnrolled
import uk.petruha.sshub.phone.core.Outgoing
import uk.petruha.sshub.phone.ui.components.QrMatrix
import uk.petruha.sshub.phone.ui.screens.enroll.BtPhase
import uk.petruha.sshub.phone.ui.screens.enroll.ComparePhase
import uk.petruha.sshub.phone.ui.screens.enroll.ConfirmStatus
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollBtState
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollCompareState
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollConfirmState
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollKeysState
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollPasteState
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollReceiveState
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollScanState
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollSendState
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollShowState
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollSoundScreenState
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollSoundState
import uk.petruha.sshub.phone.ui.screens.enroll.EnrolledState
import uk.petruha.sshub.phone.ui.screens.enroll.KeysStatus
import uk.petruha.sshub.phone.ui.screens.enroll.PasteValidation
import uk.petruha.sshub.phone.ui.screens.enroll.PasteVerdict
import uk.petruha.sshub.phone.ui.screens.enroll.SoundPhase
import uk.petruha.sshub.phone.ui.screens.enroll.SoundProfile

/** Deterministic enrollment core with the prototype's sample data. `FakePhoneCore` delegates `EnrollApi` here and calls `open` from `openFrame`. */
class FakeEnrollApi : EnrollApi {
    var token = 1L
    val decisions = mutableListOf<EnrollDecision>()

    override fun nameProblem(name: String): String? = when {
        name.isBlank() -> "Enter a name of 1 to 40 characters."
        name.toByteArray().size > 64 -> "The name is longer than 64 bytes."
        name.any { it.code < 32 || it.code == 127 } -> "The name has a control character."
        else -> null
    }

    override fun enrollTextKind(text: String): EnrollTextKind = when (val t = text.trim()) {
        "" -> EnrollTextKind.Empty
        EnrollSamples.codeText -> EnrollTextKind.Step1Code
        EnrollSamples.keysText -> EnrollTextKind.Step3Keys
        EnrollSamples.confirmText -> EnrollTextKind.Step5Confirm
        EnrollSamples.commitText, EnrollSamples.revealText, EnrollSamples.confirmOutText -> EnrollTextKind.OwnText
        else -> if (t.isEmpty()) EnrollTextKind.Empty else EnrollTextKind.NotEnrollment
    }

    override suspend fun enrollDecide(token: Long, decision: EnrollDecision, now: Long): Outgoing {
        decisions += decision
        return when (decision) {
            is EnrollDecision.Name -> EnrollSamples.commit
            is EnrollDecision.Sas -> if (decision.matches) EnrollSamples.confirm else EnrollSamples.declined
        }
    }

    /** Frame bytes of the text a paste screen holds; null when it is not one of the terminal's three sample texts. */
    fun sampleFrame(text: String): ByteArray? = when (text.trim()) {
        EnrollSamples.codeText -> EnrollSamples.frameCode
        EnrollSamples.keysText -> EnrollSamples.frameKeys
        EnrollSamples.confirmText -> EnrollSamples.frameConfirm
        else -> null
    }

    /** What `openFrame` returns for the sample frames. */
    fun open(frame: ByteArray): Opened = when {
        frame.contentEquals(EnrollSamples.frameCode) -> OpenedEnroll(token++, EnrollStage.Commit, emptyList(), EnrollSamples.terminalId, EnrollSamples.terminal, null)
        frame.contentEquals(EnrollSamples.frameKeys) -> OpenedEnroll(token++, EnrollStage.Reveal, EnrollSamples.words, EnrollSamples.terminalId, EnrollSamples.terminal, EnrollSamples.reveal)
        frame.contentEquals(EnrollSamples.frameMismatch) -> EnrollRefusal("Nothing was enrolled.", EnrollSamples.committed, EnrollSamples.shown, uk.petruha.sshub.phone.core.RefusalKind.Mismatch)
        frame.contentEquals(EnrollSamples.frameConfirm) -> OpenedEnrolled(EnrollSamples.terminalId, EnrollSamples.terminal, EnrollSamples.phoneName, EnrollSamples.phoneId)
        frame.contentEquals(EnrollSamples.frameBad) -> EnrollRefusal("The signature does not verify.", null, null)
        else -> Opened.Ignored("That is not an enrollment frame.")
    }
}

object EnrollSamples {
    const val terminal = "laptop-main"
    const val terminalId = "7B3E-0D15"
    const val phoneName = "pixel-8"
    const val phoneId = "4F2A-91C0"
    const val btTag = "7K2Q"
    const val committed = "9e41 b07c 22d8 f3a5"
    const val shown = "5a0f c3e1 7d94 08be"
    val words = listOf("triceps", "sanding", "pursuit", "aliens")

    val frameCode = "enroll-code".toByteArray()
    val frameKeys = "enroll-keys".toByteArray()
    val frameConfirm = "enroll-confirm".toByteArray()
    val frameMismatch = "enroll-mismatch".toByteArray()
    val frameBad = "enroll-bad".toByteArray()

    /** Same generator as the prototype: a deterministic base64 looking string. */
    fun b64(seed: Int, n: Int): String {
        var a = seed
        val alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        val sb = StringBuilder()
        repeat(n) { a = a * 1103515245 + 12345; sb.append(alphabet[(a ushr 16) and 63]) }
        return sb.append("==").toString()
    }

    val codeText = b64(11, 140)
    val keysText = b64(33, 176)
    val confirmText = b64(55, 120)
    val commitText = b64(22, 140)
    val revealText = b64(44, 176)
    val confirmOutText = b64(66, 120)

    /** A 25 module square with three finder squares and noisy modules; not decodable, only drawn. */
    fun qr(seed: Int, size: Int = 25): QrMatrix {
        val bits = BooleanArray(size * size)
        var a = seed
        for (i in bits.indices) { a = a * 1103515245 + 12345; bits[i] = ((a ushr 16) and 1) == 1 }
        fun finder(x0: Int, y0: Int) {
            for (y in 0 until 7) for (x in 0 until 7) {
                val edge = x == 0 || y == 0 || x == 6 || y == 6
                val core = x in 2..4 && y in 2..4
                bits[(y0 + y) * size + x0 + x] = edge || core
            }
        }
        finder(0, 0); finder(size - 7, 0); finder(0, size - 7)
        return QrMatrix(size, bits)
    }
    fun qrLoop(seed: Int, frames: Int = 6) = List(frames) { qr(seed * 31 + it) }
    fun pcm(seconds: Int, rate: Int = 8000) = ShortArray(seconds * rate)

    private fun outgoing(seed: Int, text: String) = Outgoing(
        frame = text.toByteArray(), text = text, qr = qrLoop(seed), audioFast = pcm(8), audioSlow = pcm(16), audioRate = 8000,
    )
    val commit = outgoing(2, commitText)
    val reveal = outgoing(4, revealText)
    val confirm = outgoing(5, confirmOutText)
    val declined = outgoing(9, b64(99, 120))

    // ---- sample states for previews and tests ----

    fun send(
        channel: EnrollChannel = EnrollChannel.Qr, o: Outgoing = commit, textOpen: Boolean = false, clipboardClearsIn: Int? = null,
        sound: EnrollSoundState = EnrollSoundState(), qr: Boolean = true,
    ) = EnrollSendState(channel, terminal, o.text, if (qr) o.qr else emptyList(), textOpen, clipboardClearsIn, sound)

    fun receive(
        channel: EnrollChannel = EnrollChannel.Qr, have: Int = 2, need: Int = 6, camera: CameraPermission = CameraPermission.Granted,
        note: String? = null, prompt: String = "Scan the terminal's keys",
    ) = EnrollReceiveState(channel, terminal, btTag, have, need, camera, note, prompt)

    fun scan(channel: EnrollChannel = EnrollChannel.Qr) = EnrollScanState(phoneName, null, channel, "the terminal")
    fun scanning() = scan().copy(scanning = true, receive = receive(have = 2, need = 4, prompt = "Scan the terminal's short code"))
    fun scanInvalid() = scan().copy(name = "", nameProblem = FakeEnrollApi().nameProblem(""))

    fun show(channel: EnrollChannel = EnrollChannel.Qr, textOpen: Boolean = true) = EnrollShowState(terminal, send(channel, commit, textOpen))

    fun keys(status: KeysStatus = KeysStatus.Waiting, channel: EnrollChannel = EnrollChannel.Qr) = EnrollKeysState(
        terminal, terminalId, status, receive(channel, have = if (status == KeysStatus.Ok) 6 else 2), committed, shown,
        "Nothing was enrolled.",
    )

    fun compare(phase: ComparePhase = ComparePhase.Compare, channel: EnrollChannel = EnrollChannel.Qr) = EnrollCompareState(
        terminal, terminalId, channel, words, phase,
        send(channel, if (phase == ComparePhase.Differ) declined else reveal),
    )

    fun confirm(status: ConfirmStatus = ConfirmStatus.Waiting, channel: EnrollChannel = EnrollChannel.Qr) = EnrollConfirmState(
        terminal, status, "The signature does not verify.", send(channel, confirm), receive(channel, have = if (status == ConfirmStatus.Ok) 6 else 2, prompt = "Scan the terminal's confirmation"),
    )

    fun enrolled() = EnrolledState(terminal, terminalId, phoneName, phoneId)

    val btDevices = listOf(
        BleDevice("AA:BB:CC:00:00:01", "SSHub-$btTag"), BleDevice("AA:BB:CC:00:00:02", "SSHub-3XD9"),
        BleDevice("AA:BB:CC:00:00:03", "SSHub-9AM4"), BleDevice("AA:BB:CC:00:00:04", "SSHub-B60F"),
    )
    fun bt(phase: BtPhase = BtPhase.Searching, tag: String = btTag, selected: Int = -1) =
        EnrollBtState(terminal, tag, phase, if (phase == BtPhase.Found) btDevices else emptyList(), selected)

    fun paste(n: Int = 1, verdict: PasteVerdict = PasteVerdict.Empty): EnrollPasteState {
        val own = when (n) { 2 -> commit; 5 -> confirm; else -> null }
        val text = when (verdict) {
            PasteVerdict.Empty -> ""
            PasteVerdict.Ok -> when (n) { 1 -> codeText; 3 -> keysText; else -> confirmText }
            PasteVerdict.Bad -> if (n == 1) keysText else codeText
        }
        val message = when (verdict) {
            PasteVerdict.Empty -> "Waiting for text. Step $n needs " + when (n) { 1 -> "the terminal's short code."; 3 -> "the terminal's keys."; else -> "the terminal's confirmation." }
            PasteVerdict.Ok -> "Looks like step $n frame."
            PasteVerdict.Bad -> "Not what this step needs: this looks like the step ${if (n == 1) 3 else 1} frame."
        }
        return EnrollPasteState(n, terminal, text, PasteValidation(verdict, message), own?.let { send(EnrollChannel.Paste, it) })
    }

    fun sound(
        phase: SoundPhase = SoundPhase.Idle, profile: SoundProfile = SoundProfile.Fast, available: Boolean = true,
    ): EnrollSoundScreenState {
        val total = if (profile == SoundProfile.Fast) 8 else 16
        val st = EnrollSoundState(
            profile, phase, when (phase) { SoundPhase.Idle -> 0; SoundPhase.Playing -> total * 2 / 5; SoundPhase.Done -> total }, total, available,
        )
        return EnrollSoundScreenState(2, terminal, send(EnrollChannel.Sound, commit, sound = st))
    }
}
