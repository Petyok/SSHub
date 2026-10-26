package uk.petruha.sshub.phone.fake

import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import java.time.Instant
import java.util.Base64
import uk.petruha.sshub.phone.core.ExportOpRow
import uk.petruha.sshub.phone.core.ExportedOps
import uk.petruha.sshub.phone.core.Outgoing
import uk.petruha.sshub.phone.core.RecoveryApi
import uk.petruha.sshub.phone.core.RecoveryNoticeSummary
import uk.petruha.sshub.phone.core.RecoveryNoticeView
import uk.petruha.sshub.phone.core.ReshareContributor
import uk.petruha.sshub.phone.core.ReshareStage
import uk.petruha.sshub.phone.core.ReshareView
import uk.petruha.sshub.phone.core.WrapKeyState
import uk.petruha.sshub.phone.ui.components.QrMatrix
import uk.petruha.sshub.phone.ui.screens.recovery.ExportActions
import uk.petruha.sshub.phone.ui.screens.recovery.ExportOpsScreen
import uk.petruha.sshub.phone.ui.screens.recovery.ExportState
import uk.petruha.sshub.phone.ui.screens.recovery.NoticeActions
import uk.petruha.sshub.phone.ui.screens.recovery.NoticePhase
import uk.petruha.sshub.phone.ui.screens.recovery.NoticeState
import uk.petruha.sshub.phone.ui.screens.recovery.RecoveryActions
import uk.petruha.sshub.phone.ui.screens.recovery.RecoveryNoticeScreen
import uk.petruha.sshub.phone.ui.screens.recovery.RecoveryScreen
import uk.petruha.sshub.phone.ui.screens.recovery.RecoveryState
import uk.petruha.sshub.phone.ui.screens.recovery.ReshareActions
import uk.petruha.sshub.phone.ui.screens.recovery.ReshareScreen
import uk.petruha.sshub.phone.ui.screens.recovery.ReshareState
import uk.petruha.sshub.phone.ui.theme.SshubTheme

/** Deterministic Recovery core with the prototype's data. `FakePhoneCore` delegates to it. Counters let tests assert what was called. */
class FakeRecoveryApi(stage: ReshareStage = ReshareStage.Start) : RecoveryApi {
    private var wrap = RecoverySamples.wrapKeyState(stage)
    var rotateCalls = 0
    var publishCalls = 0
    val adopted = mutableListOf<Long>()
    val cancelled = mutableListOf<Long>()
    var notices: List<RecoveryNoticeSummary> = listOf(RecoverySamples.summary)

    override suspend fun wrapKeyState(now: Long) = wrap

    override suspend fun rotateWrapKeys(now: Long): Outgoing {
        rotateCalls++
        if (wrap.stage == ReshareStage.Start) wrap = wrap.copy(generation = wrap.generation + 1, stage = ReshareStage.Rotated)
        return RecoverySamples.outgoing(1)
    }

    override suspend fun reshareCandidate(token: Long, now: Long): ReshareView? = if (token == RecoverySamples.TOKEN) RecoverySamples.candidate else null

    override suspend fun decideReshare(token: Long, now: Long): WrapKeyState {
        adopted += token
        if (wrap.stage == ReshareStage.Rotated) wrap = RecoverySamples.wrapKeyState(ReshareStage.Adopted)
        return wrap
    }

    override suspend fun publishNewShares(now: Long): Outgoing {
        publishCalls++
        if (wrap.stage == ReshareStage.Adopted) wrap = RecoverySamples.wrapKeyState(ReshareStage.Published)
        return RecoverySamples.outgoing(2)
    }

    override suspend fun exportOps(now: Long) = ExportedOps(RecoverySamples.exportRows, 41, RecoverySamples.outgoing(3))

    override suspend fun recoveryNotices(now: Long) = notices

    override suspend fun recoveryNotice(token: Long, now: Long): RecoveryNoticeView? = if (token == RecoverySamples.TOKEN) RecoverySamples.noticeView else null

    override suspend fun decideRecoveryCancel(token: Long, now: Long): Outgoing {
        cancelled += token
        notices = emptyList()
        return RecoverySamples.outgoing(4)
    }
}

/** Sample states and data for previews and tests (prototype data.js and A-trust.js). */
object RecoverySamples {
    const val TOKEN = 7L
    val startedAt: Long = Instant.parse("2026-10-25T12:41:00Z").epochSecond
    /** 21 hours after the start: "6 d 3 h" of the 7 day timelock are left. */
    val now: Long = startedAt + 21 * 3_600
    val endsAt: Long = startedAt + 7 * 86_400

    val contributors = listOf(
        ReshareContributor("laptop-main", "7B3E-0D15", true),
        ReshareContributor("workstation", "9C61-E2A8", true),
        ReshareContributor("not met", "E7A4-6B02", false),
    )
    val candidate = ReshareView(
        epoch = 8, holderNumber = 3, credentialsReady = 9, credentialsTotal = 9, contributors = contributors,
        note = "At least 2 contributors must be terminals this phone enrolled with and whose signature verifies: 2 of 2.", adoptable = true,
    )
    val refusedCandidate = candidate.copy(
        contributors = listOf(ReshareContributor("laptop-main", "7B3E-0D15", true), ReshareContributor("not met", "E7A4-6B02", false)),
        note = "At least 2 contributors must be terminals this phone enrolled with and whose signature verifies: 1 of 2.", adoptable = false,
    )

    fun wrapKeyState(stage: ReshareStage) = when (stage) {
        ReshareStage.Empty -> WrapKeyState(3, 7, 3, 9, 9, stage, null)
        ReshareStage.Start -> WrapKeyState(3, 7, 3, 0, 9, stage, null)
        ReshareStage.Rotated -> WrapKeyState(4, 7, 3, 0, 9, stage, null)
        ReshareStage.Adopted -> WrapKeyState(4, 7, 3, 9, 9, stage, candidate)
        ReshareStage.Published -> WrapKeyState(4, 8, 3, 9, 9, stage, candidate)
    }

    fun reshareState(stage: ReshareStage): ReshareState {
        val w = wrapKeyState(stage)
        return ReshareState(
            loaded = true, stage = stage, generation = w.generation, epoch = w.epoch, holderNumber = w.holderNumber,
            ready = w.credentialsReady, total = w.credentialsTotal, adopted = w.reshare,
        )
    }

    val summary = RecoveryNoticeSummary(TOKEN, "workstation [9C61-E2A8]", startedAt, endsAt)
    val recoveryEmpty = RecoveryState(emptyList(), now)
    val recoveryWithNotice = RecoveryState(listOf(summary), now)

    val noticeView = RecoveryNoticeView(
        startedBy = "workstation [9C61-E2A8]", phoneName = "pixel-8",
        operation = "phone recovery: a new phone replaces pixel-8",
        startedAt = startedAt, startedUtc = "2026-10-25 12:41Z", endsAt = endsAt, endsUtc = "2026-11-01 12:41Z",
        hash = "a3f9007c4e1b9c2d" + "5e8f0a1b3c4d5e6f" + "708192a3b4c5d6e7" + "f8091a2b3c4d5e6f",
        signatureNote = "The signature verifies: this phone knows workstation. The ring accepts the new phone after the timelock unless it is cancelled.",
        cancellable = true,
    )
    val noticeReview = NoticeState(loaded = true, view = noticeView, now = now)
    val noticeConfirm = noticeReview.copy(phase = NoticePhase.Confirm)
    val noticeSigned = noticeReview.copy(phase = NoticePhase.Signed, signed = outgoing(4))
    val noticeClosed = noticeReview.copy(
        view = noticeView.copy(cancellable = false, closedReason = "This recovery is already cancelled."),
    )

    val exportRows = listOf(
        ExportOpRow(41, "CertExtend", "10-26 03:03", "c71d4e90"), ExportOpRow(40, "RedPhoneSign", "10-25 18:20", "9f3c1ab2"),
        ExportOpRow(39, "CertIssue", "10-20 11:05", "52e0b7d4"), ExportOpRow(38, "WrapKeyRotate", "10-19 09:30", "a04f6c13"),
        ExportOpRow(37, "RedPhoneSign", "10-17 14:44", "e8d219a6"), ExportOpRow(36, "CertRevoke", "10-15 16:02", "1b73f0c8"),
        ExportOpRow(35, "CertIssue", "10-12 10:11", "7d95a2e1"), ExportOpRow(34, "RedPhoneSign", "10-08 19:58", "3c6e84f7"),
    )
    val exportState = ExportState(loaded = true, rows = exportRows, total = 41, outgoing = outgoing(3))
    val exportEmpty = ExportState(loaded = true)

    /** A signed frame of 150 deterministic bytes, its base64 text and `frames` QR frames. */
    fun outgoing(seed: Int, frames: Int = 1): Outgoing {
        val bytes = ByteArray(150) { ((it * 31 + seed * 17) and 0xFF).toByte() }
        return Outgoing(bytes, Base64.getEncoder().encodeToString(bytes), List(frames) { fakeQr(seed * 10 + it) })
    }

    private fun fakeQr(seed: Int, size: Int = 33): QrMatrix {
        val bits = BooleanArray(size * size)
        var x = seed * 7919 + 13
        for (i in bits.indices) { x = x * 1_103_515_245 + 12_345; bits[i] = ((x ushr 16) and 1) == 1 }
        fun finder(ox: Int, oy: Int) {
            for (dy in -1..7) for (dx in -1..7) {
                val px = ox + dx; val py = oy + dy
                if (px !in 0 until size || py !in 0 until size) continue
                val ring = dx in 0..6 && dy in 0..6 && (dx == 0 || dx == 6 || dy == 0 || dy == 6 || (dx in 2..4 && dy in 2..4))
                bits[py * size + px] = ring
            }
        }
        finder(0, 0); finder(size - 7, 0); finder(0, size - 7)
        return QrMatrix(size, bits)
    }
}

@Preview(name = "reshare rotated, dark", widthDp = 412, heightDp = 900)
@Composable
private fun ReshareDarkPreview() = SshubTheme(dark = true) { ReshareScreen(RecoverySamples.reshareState(ReshareStage.Rotated), ReshareActions()) }

@Preview(name = "reshare published, light", widthDp = 412, heightDp = 900)
@Composable
private fun ResharePublishedLightPreview() = SshubTheme(dark = false) { ReshareScreen(RecoverySamples.reshareState(ReshareStage.Published), ReshareActions()) }

@Preview(name = "recovery, dark", widthDp = 412, heightDp = 900)
@Composable
private fun RecoveryPreview() = SshubTheme(dark = true) { RecoveryScreen(RecoverySamples.recoveryWithNotice, RecoveryActions()) }

@Preview(name = "recovery notice, dark", widthDp = 412, heightDp = 900)
@Composable
private fun NoticePreview() = SshubTheme(dark = true) { RecoveryNoticeScreen(RecoverySamples.noticeReview, NoticeActions()) }

@Preview(name = "recovery notice signed, light", widthDp = 412, heightDp = 900)
@Composable
private fun NoticeSignedPreview() = SshubTheme(dark = false) { RecoveryNoticeScreen(RecoverySamples.noticeSigned, NoticeActions()) }

@Preview(name = "export, dark", widthDp = 412, heightDp = 900)
@Composable
private fun ExportPreview() = SshubTheme(dark = true) { ExportOpsScreen(RecoverySamples.exportState, ExportActions()) }
