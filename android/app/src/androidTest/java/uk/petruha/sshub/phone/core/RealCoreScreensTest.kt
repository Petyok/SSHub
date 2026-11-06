package uk.petruha.sshub.phone.core

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.core.uniffi.UniffiPhoneCore
import uk.petruha.sshub.phone.core.uniffi.enrollRefusalKind
import uniffi.sshub_phone_core.RefusalKind as RsRefusalKind
import uk.petruha.sshub.phone.fake.EnrollSamples
import uk.petruha.sshub.phone.platform.SealedStateStore
import uk.petruha.sshub.phone.release.descStartsWith
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.components.groupedUid
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollKeysActions
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollKeysScreen
import uk.petruha.sshub.phone.ui.screens.enroll.KeysStatus
import uk.petruha.sshub.phone.ui.screens.recovery.ExportActions
import uk.petruha.sshub.phone.ui.screens.recovery.ExportOpsScreen
import uk.petruha.sshub.phone.ui.screens.recovery.ExportState
import uk.petruha.sshub.phone.ui.screens.recovery.NoticeActions
import uk.petruha.sshub.phone.ui.screens.recovery.NoticeState
import uk.petruha.sshub.phone.ui.screens.recovery.RecoveryNoticeScreen
import uk.petruha.sshub.phone.ui.screens.release.ReleaseActions
import uk.petruha.sshub.phone.ui.screens.release.ReleaseState
import uk.petruha.sshub.phone.ui.screens.release.ShareReleaseScreen
import java.io.File
import java.util.UUID

/**
 * M11: the screens as the shipped app shows them, fed by the REAL native core over the public fixtures
 * (assets/companion/test/fixtures), not by FakePhoneCore. Each test leaves a screenshot (assertTargets48 writes it) at font scale 1.0 and
 * is checked at 2.0 for overflow and broken words. Red, trust request and extension need Rust-built fixtures and are not here.
 */
class RealCoreScreensTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private val assets = InstrumentationRegistry.getInstrumentation().context.assets
    private fun json(name: String) = JSONObject(assets.open(name).bufferedReader().readText())
    private val challenges = json("challenges.json")
    private val now = challenges.getLong("now")

    private fun hex(s: String) = ByteArray(s.length / 2) { s.substring(2 * it, 2 * it + 2).toInt(16).toByte() }

    private fun core(): UniffiPhoneCore {
        val dir = File(ctx.cacheDir, "real-" + UUID.randomUUID()).also { it.mkdirs() }
        return UniffiPhoneCore(SealedStateStore(dir)).also { it.unlock(ByteArray(32) { 0x42 }, now) }
    }

    @Test fun release_shows_the_host_and_every_op_row_of_the_real_challenge() {
        val c = challenges.getJSONArray("challenges").getJSONObject(0)
        val v = (runBlocking { core().openFrame(hex(c.getString("frame_hex")), now) } as OpenedRelease).view
        assertTrue("the core gives the host", v.hostUid.isNotEmpty())
        assertTrue("the core gives op rows", v.opRows.isNotEmpty())
        rule.setScreen { ShareReleaseScreen(ReleaseState(v, now), ReleaseActions()) }
        // M8: the host in groups of 4, the same text the lists use
        val first4 = groupedUid(v.hostUid).substringBefore(' ')
        rule.onNode(descStartsWith("Host id " + first4.uppercase().toList().joinToString(" ")), useUnmergedTree = true).assertExists()
        rule.onNodeWithText("${v.opRows.size} fields").assertExists()
        v.opRows.forEach { r ->
            rule.onNodeWithContentDescription("Field ${r.key}, new value ${r.to}").assertExists()
        }
        rule.assertTargets48()
    }

    @Test fun release_at_font_scale_2_has_no_overflow_and_no_broken_word() {
        val c = challenges.getJSONArray("challenges").getJSONObject(0)
        val v = (runBlocking { core().openFrame(hex(c.getString("frame_hex")), now) } as OpenedRelease).view
        rule.setScreen(fontScale = 2f) { ShareReleaseScreen(ReleaseState(v, now), ReleaseActions()) }
        rule.assertNoVisualOverflow()
    }

    @Test fun recovery_notice_shows_every_leaf_of_the_real_op() {
        val frame = hex(challenges.getJSONObject("recovery_notice").getString("frame_hex"))
        val opened = runBlocking { core().openFrame(frame, now) } as RecoveryNoticeOpened
        val v = opened.view
        assertTrue("the core gives op rows", v.opRows.isNotEmpty())
        rule.setScreen { RecoveryNoticeScreen(NoticeState(loaded = true, view = v, now = now), NoticeActions()) }
        rule.onNodeWithText("A phone recovery was proposed. If you did not start it, cancel it.").assertExists()
        v.opRows.forEach { r -> rule.onNodeWithContentDescription("Field ${r.key}, new value ${r.to}").assertExists() }
        rule.assertTargets48()
    }

    @Test fun recovery_notice_at_font_scale_2_has_no_overflow() {
        val frame = hex(challenges.getJSONObject("recovery_notice").getString("frame_hex"))
        val v = (runBlocking { core().openFrame(frame, now) } as RecoveryNoticeOpened).view
        rule.setScreen(fontScale = 2f) { RecoveryNoticeScreen(NoticeState(loaded = true, view = v, now = now), NoticeActions()) }
        rule.assertNoVisualOverflow()
    }

    @Test fun export_lists_the_real_ops_with_full_hashes_in_groups() {
        val core = core()
        runBlocking { core.rotateWrapKeys(now) }
        val e = runBlocking { core.exportOps(now) }
        assertTrue(e.rows.isNotEmpty())
        assertEquals(64, e.rows.first().hash.length)
        rule.setScreen { ExportOpsScreen(ExportState(loaded = true, rows = e.rows, total = e.total, outgoing = e.outgoing), ExportActions()) }
        rule.onNodeWithText(groupedUid(e.rows.first().hash), useUnmergedTree = true).assertExists()
        rule.onAllNodes(descStartsWith("Operation ")).assertCountEquals(e.rows.size)
        rule.assertTargets48()
    }

    @Test fun export_at_font_scale_2_has_no_overflow() {
        val core = core()
        runBlocking { core.rotateWrapKeys(now) }
        val e = runBlocking { core.exportOps(now) }
        rule.setScreen(fontScale = 2f) { ExportOpsScreen(ExportState(loaded = true, rows = e.rows, total = e.total, outgoing = e.outgoing), ExportActions()) }
        rule.assertNoVisualOverflow()
    }

    /** The fixtures hold the existing terminal's commit and a PHONE-role reveal: the core refuses it as the wrong frame ("Expected the terminal reveal."). */
    private fun wrongFrameRefusal(): EnrollRefusal {
        val core = core()
        val commit = hex(json("enroll.json").getJSONArray("frames_hex").getString(0))
        val opened = runBlocking { core.openFrame(commit, now) } as OpenedEnroll
        runBlocking { core.enrollDecide(opened.token, EnrollDecision.Name("pixel-8"), now) }
        val reveal = hex(json("parity-page.json").getJSONObject("enroll").getString("reveal_frame"))
        return runBlocking { core.openFrame(reveal, now) } as EnrollRefusal
    }

    @Test fun a_wrong_frame_from_the_real_core_is_a_stray_refusal_with_its_own_sentence_and_no_compared_values() {
        val r = wrongFrameRefusal()
        assertEquals(r.reason, RefusalKind.Stray, r.kind)
        assertEquals("Expected the terminal reveal.", r.reason)
        assertEquals(null, r.committed)
        assertEquals(null, r.shown)
    }

    /** No public fixture makes the core stop a session, so the stop screens use the core's own sentences (enroll.rs) in a state the view model builds for them. */
    @Test fun the_mismatch_screen_without_compared_values_draws_no_empty_boxes() {
        rule.setScreen {
            EnrollKeysScreen(
                EnrollSamples.keys(KeysStatus.Mismatch).copy(committed = "", shown = "", reason = "The terminal reveal does not open the commitment it showed first. Enrollment stopped: someone is interfering."),
                EnrollKeysActions(),
            )
        }
        rule.onNodeWithText("The keys do not match the first code.").assertExists()
        rule.onAllNodesWithText("first code committed to").assertCountEquals(0)
        rule.assertTargets48()
    }

    @Test fun enrollment_refusal_kinds_follow_the_cores_own_kind() {
        assertEquals(RefusalKind.Mismatch, enrollRefusalKind(RsRefusalKind.MISMATCH))
        for (k in listOf(RsRefusalKind.DECLINED, RsRefusalKind.BAD_SIGNATURE, RsRefusalKind.UNUSABLE_KEYS)) assertEquals(k.name, RefusalKind.Stopped, enrollRefusalKind(k))
        for (k in listOf(RsRefusalKind.WRONG_ORDER, RsRefusalKind.STRAY_FRAME, RsRefusalKind.OTHER)) assertEquals(k.name, RefusalKind.Stray, enrollRefusalKind(k))
    }
}
