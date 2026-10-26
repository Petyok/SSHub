package uk.petruha.sshub.phone.fake

import uk.petruha.sshub.phone.core.HomeApi
import uk.petruha.sshub.phone.core.HomeFakeOpened
import uk.petruha.sshub.phone.core.HomePhoneView
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.PendingItem
import uk.petruha.sshub.phone.core.PendingKind
import uk.petruha.sshub.phone.core.RingDevice
import uk.petruha.sshub.phone.core.RingSync
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
    val phone = HomePhoneView(id = "4F2A-91C7", name = "pixel-8", chainHead = null, liveShortCodes = 0)

    val devices = listOf(
        RingDevice("pixel-8", "4F2A-91C7", "phone holder", null, self = true, stale = false),
        RingDevice("laptop-main", "7B3E-0D15", "terminal holder", "3 min ago", self = false, stale = false),
        RingDevice("workstation", "9C61-E2A8", "terminal holder", "14 min ago", self = false, stale = false),
        RingDevice("nuc-lab", "C5F0-27B9", "machine", "3 h ago", self = false, stale = false),
        RingDevice("laptop-old", "1D88-A340", "terminal holder", "41 days ago", self = false, stale = true),
    )

    val pending = listOf(
        PendingItem(PendingKind.Red, "Revoke device laptop-old", "6 min ago", Routes.Red + "/revoke"),
        PendingItem(PendingKind.Red, "Switch credential mode to Quorum", "2 h ago", Routes.Red + "/mode"),
        PendingItem(PendingKind.Release, "db-staging", "38 s ago", Routes.Release + "/dbs"),
        PendingItem(PendingKind.Release, "bastion", "1 min ago", Routes.Release + "/bastion"),
    )

    val ring = RingView(
        name = "home ring", epoch = 7, hosts = 38, credentials = 12, threshold = "2 of 3", quorum = 9, single = 3,
        lastSync = RingSync(agoSeconds = 180, agoText = "3 min ago", windowSeconds = 3600, via = "laptop-main, camera"),
        devices = devices, pendingRed = 2, pendingRelease = 2,
    )

    val home = HomeState(phone, ring, pending)
    val homeHeadPending = HomeState(phone, ring.copy(headPending = true), pending)
    val homeNoPending = HomeState(phone, ring.copy(pendingRed = 0, pendingRelease = 0), emptyList())
    /** A phone that knows little: no counts, no sync, devices without seen text. */
    val homeSparse = HomeState(
        phone,
        RingView("home ring", 7, null, null, null, null, null, null, listOf(devices[0], devices[1].copy(seen = null)), headPending = false),
        emptyList(),
    )
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
    override fun ring(now: Long): RingView? = if (enrolled) HomeSamples.ring.copy(headPending = headPending) else null
    override fun thisPhone(now: Long): HomePhoneView = HomeSamples.phone
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
    override fun frameFromText(text: String): ByteArray? =
        text.trim().takeIf { it.length >= 8 && it.all { c -> c.isLetterOrDigit() || c == '+' || c == '/' || c == '=' } }?.toByteArray()
    override suspend fun openFrame(frame: ByteArray, now: Long): Opened { opened += frame; return openResult }
}
