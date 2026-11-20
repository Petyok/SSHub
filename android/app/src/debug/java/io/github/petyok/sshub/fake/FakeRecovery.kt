package io.github.petyok.sshub.fake

import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import java.time.Instant
import java.util.Base64
import io.github.petyok.sshub.core.ExportOpRow
import io.github.petyok.sshub.core.ExportedOps
import io.github.petyok.sshub.core.Outgoing
import io.github.petyok.sshub.core.RecoveryApi
import io.github.petyok.sshub.core.RecoveryNoticeSummary
import io.github.petyok.sshub.core.RecoveryNoticeView
import io.github.petyok.sshub.core.RedField
import io.github.petyok.sshub.core.ReshareContributor
import io.github.petyok.sshub.core.ReshareHolder
import io.github.petyok.sshub.core.ReshareOutcome
import io.github.petyok.sshub.core.ReshareStage
import io.github.petyok.sshub.core.ReshareView
import io.github.petyok.sshub.core.WrapKeyState
import io.github.petyok.sshub.ui.components.QrMatrix
import io.github.petyok.sshub.ui.screens.recovery.ExportActions
import io.github.petyok.sshub.ui.screens.recovery.ExportOpsScreen
import io.github.petyok.sshub.ui.screens.recovery.ExportState
import io.github.petyok.sshub.ui.screens.recovery.NoticeActions
import io.github.petyok.sshub.ui.screens.recovery.NoticePhase
import io.github.petyok.sshub.ui.screens.recovery.NoticeState
import io.github.petyok.sshub.ui.screens.recovery.RecoveryActions
import io.github.petyok.sshub.ui.screens.recovery.RecoveryNoticeScreen
import io.github.petyok.sshub.ui.screens.recovery.RecoveryScreen
import io.github.petyok.sshub.ui.screens.recovery.RecoveryState
import io.github.petyok.sshub.ui.screens.recovery.ReshareActions
import io.github.petyok.sshub.ui.screens.recovery.ReshareScreen
import io.github.petyok.sshub.ui.screens.recovery.ReshareState
import io.github.petyok.sshub.ui.theme.SshubTheme

/** Deterministic Recovery core with the prototype's data. `FakePhoneCore` delegates to it. Counters let tests assert what was called. */
class FakeRecoveryApi(stage: ReshareStage = ReshareStage.Start) : RecoveryApi {
    private var wrap = RecoverySamples.wrapKeyState(stage)
    var rotateCalls = 0
    /** Calls of `decideReshare`: adopt and publish is one call. */
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

    override suspend fun decideReshare(token: Long, now: Long): ReshareOutcome {
        adopted += token
        publishCalls++
        if (wrap.stage < ReshareStage.Published) wrap = RecoverySamples.wrapKeyState(ReshareStage.Published)
        return ReshareOutcome(wrap, RecoverySamples.outgoing(2))
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
        ReshareContributor("not met", "E7A4-6B02", false, known = false, hasSession = false),
    )
    val holders = listOf(
        ReshareHolder("7B3E-0D15", "laptop-main", false), ReshareHolder("9C61-E2A8", "workstation", false), ReshareHolder("4F2A-91C0", "pixel-8", true),
    )
    val candidate = ReshareView(
        epoch = 8, holderNumber = 3, credentialsReady = 9, credentialsTotal = 9, contributors = contributors,
        note = "At least 2 contributors must be terminals this phone enrolled with and whose signature verifies: 2 of 2.", adoptable = true,
        holdersAfter = holders, storageKeyNew = true,
    )
    val refusedCandidate = candidate.copy(
        contributors = listOf(ReshareContributor("laptop-main", "7B3E-0D15", true), ReshareContributor("not met", "E7A4-6B02", false)),
        note = "At least 2 contributors must be terminals this phone enrolled with and whose signature verifies: 1 of 2.", adoptable = false,
    )

    fun wrapKeyState(stage: ReshareStage) = when (stage) {
        ReshareStage.Empty -> WrapKeyState(3, 7, 3, 9, 9, stage, null)
        ReshareStage.Start -> WrapKeyState(3, 7, 3, 0, 9, stage, null)
        ReshareStage.Rotated -> WrapKeyState(4, 7, 3, 0, 9, stage, null)
        ReshareStage.Candidate -> WrapKeyState(4, 7, 3, 0, 9, stage, null)
        ReshareStage.Published -> WrapKeyState(4, 8, 3, 9, 9, stage, candidate)
    }

    fun reshareState(stage: ReshareStage): ReshareState {
        val w = wrapKeyState(stage)
        return ReshareState(
            loaded = true, stage = stage, generation = w.generation, epoch = w.epoch, holderNumber = w.holderNumber,
            ready = w.credentialsReady, total = w.credentialsTotal, reshare = w.reshare,
            candidate = if (stage == ReshareStage.Candidate) candidate else null,
        )
    }

    val summary = RecoveryNoticeSummary(TOKEN, "workstation [9C61-E2A8]", startedAt, endsAt)
    val recoveryEmpty = RecoveryState(emptyList(), now)
    val recoveryWithNotice = RecoveryState(listOf(summary), now)

    val noticeView = RecoveryNoticeView(
        startedBy = "workstation [9C61-E2A8]", lamport = 1038, phoneName = "pixel-8",
        operation = "phone recovery: a new phone replaces pixel-8",
        startedAt = startedAt, startedUtc = "2026-10-25 12:41Z", endsAt = endsAt, endsUtc = "2026-11-01 12:41Z",
        hash = "a3f9007c4e1b9c2d" + "5e8f0a1b3c4d5e6f" + "708192a3b4c5d6e7" + "f8091a2b3c4d5e6f",
        signatureNote = "The signature verifies: this phone knows workstation. The ring accepts the new phone after the timelock unless it is cancelled.",
        cancellable = true,
        opRows = listOf(
            RedField("signers", null, "workstation [9C61-E2A8]"),
            RedField("via_paper", null, "false"),
            RedField("third_factor", null, "EmailCode"),
            RedField("initiated_at", null, "2026-10-25T12:41:00Z", danger = false),
        ),
    )
    val noticeReview = NoticeState(loaded = true, view = noticeView, now = now)
    val noticeConfirm = noticeReview.copy(phase = NoticePhase.Confirm)
    val noticeSigned = noticeReview.copy(phase = NoticePhase.Signed, signed = outgoing(4))
    val noticeClosed = noticeReview.copy(
        view = noticeView.copy(cancellable = false, closedReason = "This recovery is already cancelled."),
    )

    /** A 64 digit hash from an 8 digit seed: the real core gives the full BLAKE3 hex. */
    private fun hash64(seed: String) = seed.repeat(8)

    val exportRows = listOf(
        ExportOpRow(41, "CertExtend", "10-26 03:03", hash64("c71d4e90"), danger = true), ExportOpRow(40, "RedPhoneSign", "10-25 18:20", hash64("9f3c1ab2")),
        ExportOpRow(39, "CertIssue", "10-20 11:05", hash64("52e0b7d4"), danger = true), ExportOpRow(38, "WrapKeyRotate", "10-19 09:30", hash64("a04f6c13")),
        ExportOpRow(37, "RedPhoneSign", "10-17 14:44", hash64("e8d219a6")), ExportOpRow(36, "CertRevoke", "10-15 16:02", hash64("1b73f0c8"), danger = true),
        ExportOpRow(35, "CertIssue", "10-12 10:11", hash64("7d95a2e1"), danger = true), ExportOpRow(34, "RedPhoneSign", "10-08 19:58", hash64("3c6e84f7")),
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
