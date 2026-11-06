package uk.petruha.sshub.phone.fake

import uk.petruha.sshub.phone.core.HomeApi
import uk.petruha.sshub.phone.core.HomeFakeOpened
import uk.petruha.sshub.phone.core.HomePhoneView
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.PendingItem
import uk.petruha.sshub.phone.core.PendingKind
import uk.petruha.sshub.phone.core.RingDevice
import uk.petruha.sshub.phone.core.RingView
import uk.petruha.sshub.phone.core.Routes
import uk.petruha.sshub.phone.core.ScanApi
import uk.petruha.sshub.phone.core.ScanEvent
import uk.petruha.sshub.phone.ui.screens.home.ApprovalsState
import uk.petruha.sshub.phone.ui.screens.home.HomeState
import uk.petruha.sshub.phone.ui.screens.home.ScanCamera
import uk.petruha.sshub.phone.ui.screens.home.ScanState
import uk.petruha.sshub.phone.ui.screens.home.scanHint
import uk.petruha.sshub.phone.ui.screens.home.scanTitle
import uk.petruha.sshub.phone.core.ScanPurpose

/** Prototype data.js sample content (fictional names only). Deterministic: ages do not move with `now`. */
object HomeSamples {
    val phone = HomePhoneView(id = "4F2A-91C7", name = "pixel-8", headSeq = null, liveShortCodes = 0)

    val devices = listOf(
        RingDevice("pixel-8", "4F2A-91C7", self = true),
        RingDevice("laptop-main", "7B3E-0D15", self = false),
        RingDevice("workstation", "9C61-E2A8", self = false),
        RingDevice(null, "C5F0-27B9", self = false),
    )

    val pending = listOf(
        PendingItem(PendingKind.Red, "Revoke device laptop-old", "6 min ago", Routes.Red + "/revoke"),
        PendingItem(PendingKind.Red, "Switch credential mode to Quorum", "2 h ago", Routes.Red + "/mode"),
        PendingItem(PendingKind.Release, "db-staging", "38 s ago", Routes.Release + "/dbs"),
        PendingItem(PendingKind.Release, "bastion", "1 min ago", Routes.Release + "/bastion"),
    )

    val ring = RingView(epoch = 7, devices = devices)

    val enrolledPhone = phone.copy(headSeq = 12, liveShortCodes = 2)
    val home = HomeState(enrolledPhone, ring, pending)
    val homeHeadPending = HomeState(enrolledPhone, ring.copy(headPending = true), pending)
    val homeNoPending = HomeState(enrolledPhone, ring, emptyList())
    val homeEmpty = HomeState(phone, null, emptyList())

    val approvals = ApprovalsState(pending, 0)
    val approvalsReleases = ApprovalsState(pending, 1)
    val approvalsTrustEmpty = ApprovalsState(pending, 2)
    val approvalsNone = ApprovalsState(emptyList(), 0)
    val approvalsTrust = ApprovalsState(
        pending + listOf(
            PendingItem(PendingKind.Trust, "Certificate for laptop-main", "1 min ago", Routes.TrustRequest + "/t1"),
            PendingItem(PendingKind.Extend, "Extend budget of workstation", "4 min ago", Routes.TrustExtend + "/e1"),
        ),
        2,
    )

    private fun scan(p: ScanPurpose = ScanPurpose.Request, f: (ScanState) -> ScanState = { it }) = f(ScanState(scanTitle(p), scanHint(p)))
    val scanLive = scan()
    val scanProgress = scan { it.copy(have = 3, need = 10) }
    val scanIgnored = scan { it.copy(ignored = "Ignored a QR code that is not what this step needs.") }
    val scanRationale = scan { it.copy(camera = ScanCamera.Rationale) }
    val scanDenied = scan { it.copy(camera = ScanCamera.Denied) }
    val scanDeniedForever = scan { it.copy(camera = ScanCamera.DeniedForever) }
    val scanNone = scan { it.copy(camera = ScanCamera.None, paste = true) }
    val scanDone = scan { it.copy(have = 10, need = 10, done = true) }
    val scanPaste = scan { it.copy(paste = true, pasteText = "QUJDREVGR0hJSktMTU5PUA==") }
    val scanPasteError = scan { it.copy(paste = true, pasteText = "not base64!", pasteError = "That is not base64 text. Copy the whole text the terminal shows.") }
}

class FakeHomeApi(
    var enrolled: Boolean = true,
    var headPending: Boolean = false,
    var items: List<PendingItem> = HomeSamples.pending,
) : HomeApi {
    override suspend fun ring(now: Long): RingView? = if (enrolled) HomeSamples.ring.copy(headPending = headPending) else null
    override suspend fun thisPhone(now: Long): HomePhoneView = HomeSamples.phone
    override fun pending(now: Long): List<PendingItem> = items
}

/** Scripted ScanApi: `events` are returned one per `scanGray` call (None when empty). */
class HomeScanFake : ScanApi {
    val events = ArrayDeque<ScanEvent>()
    var openResult: Opened = HomeFakeOpened("opened")
    var resets = 0
    val opened = mutableListOf<ByteArray>()
    override fun scanGray(width: Int, height: Int, luma: ByteArray): ScanEvent = events.removeFirstOrNull() ?: ScanEvent.None
    override fun resetScan() { resets++ }
    override suspend fun frameFromText(text: String): ByteArray? =
        text.trim().takeIf { it.length >= 8 && it.all { c -> c.isLetterOrDigit() || c == '+' || c == '/' || c == '=' } }?.toByteArray()
    override suspend fun openFrame(frame: ByteArray, now: Long): Opened { opened += frame; return openResult }
}
