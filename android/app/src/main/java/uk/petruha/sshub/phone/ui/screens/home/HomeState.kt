package uk.petruha.sshub.phone.ui.screens.home

import uk.petruha.sshub.phone.core.HomePhoneView
import uk.petruha.sshub.phone.core.PendingItem
import uk.petruha.sshub.phone.core.PendingKind
import uk.petruha.sshub.phone.core.RingView

/** Home before enrollment is `ring == null`. `headPending` is the status's, so a restored phone without a ring still shows the card. */
data class HomeState(
    val phone: HomePhoneView,
    val ring: RingView?,
    val pending: List<PendingItem>,
    val headPending: Boolean = ring?.headPending ?: false,
    /** Name of the device that was removed while this phone has not yet got the new shares; null when nothing is owed. */
    val updateNeededAfter: String? = null,
) {
    /** Restored phone whose chain was adopted but that has no ring yet: not a fresh phone, so no Enroll offer (N9). */
    val restoredAdopted: Boolean get() = phone.headSeq != null && !headPending && ring == null
}

class HomeActions(
    val onOpenPending: (PendingItem) -> Unit = {},
    val onOpenApprovals: () -> Unit = {},
    val onEnroll: () -> Unit = {},
    val onRestore: () -> Unit = {},
    val onTakeChain: () -> Unit = {},
    val onUpdateShares: () -> Unit = {},
)

/** The three segments of the approvals list. */
enum class ApprovalTab(val label: String) { Red("Red"), Releases("Releases"), Trust("Trust") }

fun PendingKind.tab(): ApprovalTab = when (this) {
    PendingKind.Red -> ApprovalTab.Red
    PendingKind.Release -> ApprovalTab.Releases
    PendingKind.Trust, PendingKind.Extend, PendingKind.Renew, PendingKind.Handover, PendingKind.Recovery, PendingKind.Reshare -> ApprovalTab.Trust
}

fun PendingKind.word(): String = when (this) {
    PendingKind.Red -> "Red"
    PendingKind.Release -> "Release"
    PendingKind.Trust -> "Trust"
    PendingKind.Extend -> "Extend"
    PendingKind.Renew -> "Renew"
    PendingKind.Handover -> "Handover"
    PendingKind.Recovery -> "Recovery"
    PendingKind.Reshare -> "Reshare"
}

data class ApprovalsState(val pending: List<PendingItem>, val selected: Int = 0)

class ApprovalsActions(
    val onSelect: (Int) -> Unit = {},
    val onOpen: (PendingItem) -> Unit = {},
    val onScan: () -> Unit = {},
)

enum class ScanCamera { Rationale, Live, Denied, DeniedForever, None }

/** `need == 0` means the progress is unknown (no frame read yet). `ignored` is the literal notice text of the last QR that was not for this step. */
data class ScanState(
    val title: String,
    val hint: String,
    val camera: ScanCamera = ScanCamera.Live,
    val have: Int = 0,
    val need: Int = 0,
    val ignored: String? = null,
    val done: Boolean = false,
    val paste: Boolean = false,
    val pasteText: String = "",
    val pasteError: String? = null,
)

class ScanActions(
    val onBack: () -> Unit = {},
    val onGrant: () -> Unit = {},
    val onOpenSettings: () -> Unit = {},
    val onTogglePaste: () -> Unit = {},
    val onPasteText: (String) -> Unit = {},
    val onPasteFromClipboard: () -> Unit = {},
    val onUsePasted: () -> Unit = {},
    val onContinue: () -> Unit = {},
    val onLuma: (width: Int, height: Int, luma: ByteArray) -> Unit = { _, _, _ -> },
)
