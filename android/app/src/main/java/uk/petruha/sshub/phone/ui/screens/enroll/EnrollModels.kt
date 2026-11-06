package uk.petruha.sshub.phone.ui.screens.enroll

import uk.petruha.sshub.phone.core.BleDevice
import uk.petruha.sshub.phone.core.CameraPermission
import uk.petruha.sshub.phone.core.EnrollChannel
import uk.petruha.sshub.phone.ui.components.QrMatrix

/** State and action types of the nine enrollment screens. Screens are stateless: `XScreen(state, on)`. */

enum class SoundProfile { Fast, Slow }
enum class SoundPhase { Idle, Playing, Done }

/** This phone plays frames as sound and never records. `total` is the length of the chosen profile in whole seconds. */
data class EnrollSoundState(
    val profile: SoundProfile = SoundProfile.Fast,
    val phase: SoundPhase = SoundPhase.Idle,
    val elapsed: Int = 0,
    val total: Int = 8,
    /** False when the core produced no sound version of this reply. */
    val available: Boolean = true,
)

/** What this phone shows or plays to the terminal at one step, for the chosen channel. */
data class EnrollSendState(
    val channel: EnrollChannel,
    val terminal: String,
    /** Base64 of the frame: the same bytes as the QR code. */
    val text: String,
    val qr: List<QrMatrix> = emptyList(),
    val textOpen: Boolean = false,
    /** Seconds until the clipboard is cleared, null when nothing of ours is on it (D19). */
    val clipboardClearsIn: Int? = null,
    val sound: EnrollSoundState = EnrollSoundState(),
)

class EnrollSendActions(
    val onToggleText: () -> Unit = {},
    val onCopy: () -> Unit = {},
    val onProfile: (SoundProfile) -> Unit = {},
    val onPlay: () -> Unit = {},
    val onStop: () -> Unit = {},
)

/** What the terminal sends to this phone at one step. Camera for Qr and Sound (the phone does not listen), link for Bluetooth. */
data class EnrollReceiveState(
    val channel: EnrollChannel,
    val terminal: String,
    val tag: String = "",
    /** Frames read of a looping fountain; need = 0 until the first frame is read. */
    val have: Int = 0,
    val need: Int = 0,
    val camera: CameraPermission = CameraPermission.Granted,
    /** A line from the core such as "That code is not an SSHub code". */
    val note: String? = null,
    /** "Scan the terminal's short code", "Scan the terminal's keys", ... */
    val prompt: String = "",
)

class EnrollReceiveActions(
    val onLuma: (Int, Int, ByteArray) -> Unit = { _, _, _ -> },
    val onRequestCamera: () -> Unit = {},
    val onOpenSettings: () -> Unit = {},
)

// ---- 1 Scan -------------------------------------------------------------------------------------------------------

data class EnrollScanState(
    val name: String,
    /** Null when the core accepts the name. */
    val nameProblem: String?,
    val channel: EnrollChannel,
    val terminal: String = "the terminal",
    val scanning: Boolean = false,
    val receive: EnrollReceiveState = EnrollReceiveState(EnrollChannel.Qr, "the terminal"),
)

class EnrollScanActions(
    val onBack: () -> Unit = {},
    val onName: (String) -> Unit = {},
    val onChannel: (EnrollChannel) -> Unit = {},
    val onScan: () -> Unit = {},
    val onCancelScan: () -> Unit = {},
    val onFindTerminal: () -> Unit = {},
    val onPasteText: () -> Unit = {},
    val receive: EnrollReceiveActions = EnrollReceiveActions(),
)

// ---- 2 Show -------------------------------------------------------------------------------------------------------

data class EnrollShowState(val terminal: String, val send: EnrollSendState)

class EnrollShowActions(
    val onBack: () -> Unit = {},
    val onNext: () -> Unit = {},
    val send: EnrollSendActions = EnrollSendActions(),
)

// ---- 3 Keys -------------------------------------------------------------------------------------------------------

/** `Mismatch`: the keys do not open the first code. `Stopped`: another refusal that ended the session (titled with the core's sentence). */
enum class KeysStatus { Waiting, Ok, Mismatch, Stopped }

data class EnrollKeysState(
    val terminal: String,
    val terminalId: String = "",
    val status: KeysStatus,
    val receive: EnrollReceiveState,
    /** Mismatch only: the two compared values in full, from the core. */
    val committed: String = "",
    val shown: String = "",
    val reason: String = "",
)

class EnrollKeysActions(
    val onBack: () -> Unit = {},
    val onCompare: () -> Unit = {},
    val onStartAgain: () -> Unit = {},
    val receive: EnrollReceiveActions = EnrollReceiveActions(),
)

// ---- 4 Compare ----------------------------------------------------------------------------------------------------

enum class ComparePhase { Compare, Match, Differ }

data class EnrollCompareState(
    val terminal: String,
    val terminalId: String,
    val channel: EnrollChannel,
    val words: List<String>,
    val phase: ComparePhase,
    /** Compare and Match: this phone's own keys. Differ: the declined confirmation for the terminal. */
    val send: EnrollSendState,
)

class EnrollCompareActions(
    val onBack: () -> Unit = {},
    val onMatch: () -> Unit = {},
    val onDiffer: () -> Unit = {},
    val onContinue: () -> Unit = {},
    val onStartAgain: () -> Unit = {},
    val send: EnrollSendActions = EnrollSendActions(),
)

// ---- 5 Confirm ----------------------------------------------------------------------------------------------------

enum class ConfirmStatus { Waiting, Bad, Ok }

data class EnrollConfirmState(
    val terminal: String,
    val status: ConfirmStatus,
    val reason: String = "",
    val send: EnrollSendState,
    val receive: EnrollReceiveState,
)

class EnrollConfirmActions(
    val onBack: () -> Unit = {},
    val onFinish: () -> Unit = {},
    val onStartAgain: () -> Unit = {},
    val send: EnrollSendActions = EnrollSendActions(),
    val receive: EnrollReceiveActions = EnrollReceiveActions(),
)

// ---- done ---------------------------------------------------------------------------------------------------------

data class EnrolledState(val terminal: String, val terminalId: String, val phoneName: String, val phoneId: String)

class EnrolledActions(val onDone: () -> Unit = {})

// ---- Bluetooth chooser --------------------------------------------------------------------------------------------

enum class BtPhase { Searching, Found, None, Denied, Off, Unsupported, Connecting, LinkFailed }

data class EnrollBtState(
    val terminal: String,
    val tag: String,
    val phase: BtPhase,
    val devices: List<BleDevice> = emptyList(),
    /** Index into `devices`, -1 for none. */
    val selected: Int = -1,
)

class EnrollBtActions(
    val onBack: () -> Unit = {},
    val onTag: (String) -> Unit = {},
    val onSelect: (Int) -> Unit = {},
    val onPair: () -> Unit = {},
    val onSearchAgain: () -> Unit = {},
    val onAllow: () -> Unit = {},
    val onOtherChannel: () -> Unit = {},
)

// ---- Paste --------------------------------------------------------------------------------------------------------

enum class PasteVerdict { Empty, Ok, Bad }

data class PasteValidation(val verdict: PasteVerdict, val message: String)

data class EnrollPasteState(
    /** Step 1, 2, 3 or 5. */
    val n: Int,
    val terminal: String,
    val text: String,
    val validation: PasteValidation,
    /** This phone's text for the terminal at steps 2 and 5, null where nothing is sent. */
    val send: EnrollSendState?,
)

class EnrollPasteActions(
    val onBack: () -> Unit = {},
    val onText: (String) -> Unit = {},
    val onPaste: () -> Unit = {},
    val onClear: () -> Unit = {},
    val onUse: () -> Unit = {},
    val onNext: () -> Unit = {},
    val send: EnrollSendActions = EnrollSendActions(),
)

// ---- Sound --------------------------------------------------------------------------------------------------------

data class EnrollSoundScreenState(
    /** The step whose frame is played, normally 2. */
    val step: Int,
    val terminal: String,
    val send: EnrollSendState,
)

class EnrollSoundActions(
    val onBack: () -> Unit = {},
    val onContinue: () -> Unit = {},
    val send: EnrollSendActions = EnrollSendActions(),
)
