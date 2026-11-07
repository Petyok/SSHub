package uk.petruha.sshub.phone.release

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.core.ChannelKind
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.ReleaseApi
import uk.petruha.sshub.phone.core.ReleaseDecision
import uk.petruha.sshub.phone.core.ReleaseOutcome
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.fake.FakeReleaseApi
import uk.petruha.sshub.phone.fake.ReleaseSamples
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.release.ApprovalPhase
import uk.petruha.sshub.phone.ui.screens.release.RedActions
import uk.petruha.sshub.phone.ui.screens.release.RedScreen
import uk.petruha.sshub.phone.ui.screens.release.ReleaseActions
import uk.petruha.sshub.phone.ui.screens.release.ReleaseViewModel
import uk.petruha.sshub.phone.ui.screens.release.ReleaseState
import uk.petruha.sshub.phone.ui.screens.release.ReleasedViews
import uk.petruha.sshub.phone.ui.screens.release.ShareReleaseScreen

/** Review round 2: N1 and N-L5 (no code, words or QR on re-entry), C-M1 (error text), N6 (Red typed id reason). */
class ReleaseRound2Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val fake = FakeReleaseApi()
    private var kept: ReleaseOutcome? = null

    /** Like the adapter after Approve: the kept outcome still carries the code and the words, the screen must not show them. */
    private val api = object : ReleaseApi {
        override fun pendingView(token: Long): Opened? = fake.pendingView(token)
        override suspend fun decide(token: Long, decision: ReleaseDecision, now: Long): ReleaseOutcome =
            fake.decide(token, decision, now).also { kept = it }
        override fun lastOutcome(token: Long): ReleaseOutcome? = kept
    }

    private fun vm(a: ReleaseApi, p: FakePlatform, token: Long): ReleaseViewModel {
        lateinit var v: ReleaseViewModel
        val i = InstrumentationRegistry.getInstrumentation()
        i.runOnMainSync { v = ReleaseViewModel(a, p, token) }
        i.waitForIdleSync()
        return v
    }

    private fun settled(v: ReleaseViewModel, until: (ReleaseState?) -> Boolean) {
        val end = System.currentTimeMillis() + 5000
        while (!until(v.state.value) && System.currentTimeMillis() < end) Thread.sleep(20)
    }

    @Test fun re_entry_shows_no_code_no_words_no_qr_but_the_channels() {
        ReleasedViews.clear()
        val p = FakePlatform()
        val first = vm(api, p, ReleaseSamples.TOKEN_DBS)
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            first.shortCode(true); first.byHand(true); first.approve()
        }
        settled(first) { it?.phase == ApprovalPhase.Approved }
        assertTrue(first.state.value!!.outcome!!.shortCode != null)

        fake.views.remove(ReleaseSamples.TOKEN_DBS) // the core dropped the request on Approve
        val again = vm(api, p, ReleaseSamples.TOKEN_DBS).state.value!!
        assertEquals(ApprovalPhase.Approved, again.phase)
        assertNull(again.outcome!!.shortCode); assertNull(again.outcome!!.byHand)
        rule.setScreen { ShareReleaseScreen(again, ReleaseActions()) }
        rule.onNodeWithText("Signed.").assertExists()
        rule.onNodeWithText("Shown as: QR code. Check on the terminal that it took the share.").assertExists()
        rule.onAllNodesWithText("share index", substring = true).assertCountEquals(0)
        rule.onAllNodesWithContentDescription("Short code", substring = true).assertCountEquals(0)
        rule.onAllNodesWithText("short code", substring = true).assertCountEquals(0)
        rule.onAllNodesWithText("Copy as text").assertCountEquals(0)
        rule.onAllNodesWithContentDescription("QR code", substring = true).assertCountEquals(0)
        rule.onNodeWithText("The code was shown when you approved. It is not shown again.").assertExists()
    }

    @Test fun released_views_are_empty_after_the_phone_locks() {
        ReleasedViews.clear()
        val p = FakePlatform()
        ReleasedViews.put(5, ReleaseSamples.releaseDbs, setOf(ChannelKind.Qr))
        vm(api, p, 99)
        assertTrue(ReleasedViews.view(5) != null)
        p.keys.lock()
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        val end = System.currentTimeMillis() + 5000
        while (ReleasedViews.view(5) != null && System.currentTimeMillis() < end) Thread.sleep(20)
        assertNull(ReleasedViews.view(5)); assertNull(ReleasedViews.channels(5))
    }

    @Test fun a_failed_save_shows_the_core_sentence_without_nothing_was_signed() {
        val sentence = "The phone state could not be saved; the request was discarded."
        val failing = object : ReleaseApi {
            override fun pendingView(token: Long): Opened? = fake.pendingView(token)
            override suspend fun decide(token: Long, decision: ReleaseDecision, now: Long): ReleaseOutcome =
                throw CoreException(CoreException.Code.Internal, sentence)
        }
        val v = vm(failing, FakePlatform(), ReleaseSamples.TOKEN_DBS)
        InstrumentationRegistry.getInstrumentation().runOnMainSync { v.approve() }
        settled(v) { it?.error != null }
        assertEquals(sentence, v.state.value!!.error)
        assertEquals(ApprovalPhase.Review, v.state.value!!.phase)
    }

    @Test fun a_refusal_still_says_nothing_was_signed() {
        val refusing = object : ReleaseApi {
            override fun pendingView(token: Long): Opened? = fake.pendingView(token)
            override suspend fun decide(token: Long, decision: ReleaseDecision, now: Long): ReleaseOutcome =
                throw CoreException(CoreException.Code.Refused, "No.")
        }
        val v = vm(refusing, FakePlatform(), ReleaseSamples.TOKEN_DBS)
        InstrumentationRegistry.getInstrumentation().runOnMainSync { v.approve() }
        settled(v) { it?.error != null }
        assertEquals("No. Nothing was signed.", v.state.value!!.error)
    }

    @Test fun n6_red_with_an_unmet_proposer_names_the_typed_id_as_the_reason_and_points_to_the_field() {
        rule.setScreen { RedScreen(ReleaseSamples.redState(ReleaseSamples.redUnknownProposer), RedActions()) }
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        rule.onNodeWithText("type the id first").assertExists()
        rule.onNodeWithText("This proposer never met this phone: type its id in the field below the approvals to enable Approve.").assertExists()
    }

    @Test fun n6_a_met_proposer_has_no_such_note() {
        rule.setScreen { RedScreen(ReleaseSamples.redState(), RedActions()) }
        rule.onAllNodesWithText("type the id first").assertCountEquals(0)
        assertFalse(ReleaseSamples.redRevoke.mustType != null)
    }
}
