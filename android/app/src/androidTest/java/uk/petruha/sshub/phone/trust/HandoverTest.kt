package uk.petruha.sshub.phone.trust

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uk.petruha.sshub.phone.core.HandoffKind
import uk.petruha.sshub.phone.core.TrustHandoff
import uk.petruha.sshub.phone.fake.TrustSamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.trust.HandoverActions
import uk.petruha.sshub.phone.ui.screens.trust.HandoverScreen
import uk.petruha.sshub.phone.ui.screens.trust.HandoverState
import uk.petruha.sshub.phone.ui.screens.trust.TrustPhase

@RunWith(AndroidJUnit4::class)
class HandoverTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val old = HandoverState(TrustSamples.handoverOld)
    private val admission = HandoverState(TrustSamples.handoverNew)

    private fun actions(
        onWords: (Boolean) -> Unit = {}, onApprove: () -> Unit = {}, onDecline: () -> Unit = {}, onDone: () -> Unit = {}, onBack: () -> Unit = {},
    ) = HandoverActions(onWords, onApprove, onDecline, {}, onDone, onBack)

    @Test fun old_phone_shows_the_name_the_eight_words_and_the_certs_carried() {
        rule.setScreen { HandoverScreen(old, actions()) }
        rule.onNodeWithText("A new phone asks to replace this one").assertExists()
        rule.onNodeWithText("pixel-9").assertExists()
        rule.onNode(hasContentDescription("Words 1 to 4: amber, basket, cactus, dolphin")).assertExists()
        rule.onNode(hasContentDescription("Words 5 to 8: ember, falcon, garden, harbor")).assertExists()
        rule.onNodeWithText("workstation [9C61-E2A8] until 2026-11-03").assertExists()
        rule.assertTargets48()
    }

    @Test fun old_phone_cannot_approve_until_the_words_are_confirmed_identical() {
        var approved = 0; var ticks = 0
        rule.setScreen { HandoverScreen(old, actions(onApprove = { approved++ }, onWords = { ticks++ })) }
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        rule.onNodeWithText("The 8 words are identical to the terminal's screen").performClick()
        assertEquals(1, ticks)
        assertEquals(0, approved)
        assertFalse(old.approveEnabled)
        assertTrue(old.copy(wordsMatch = true).approveEnabled)
        assertFalse(old.copy(wordsMatch = true, busy = true).approveEnabled)
    }

    @Test fun confirmed_words_enable_approve_and_decline_always_works() {
        var approved = 0; var declined = 0
        rule.setScreen { HandoverScreen(old.copy(wordsMatch = true), actions(onApprove = { approved++ }, onDecline = { declined++ })) }
        rule.onNodeWithText("Approve").assertIsEnabled().performClick()
        rule.onNodeWithText("Decline").performClick()
        assertEquals(1, approved); assertEquals(1, declined)
    }

    @Test fun the_new_phone_has_no_words_to_compare_and_signs_its_admission() {
        var approved = 0
        rule.setScreen { HandoverScreen(admission, actions(onApprove = { approved++ })) }
        rule.onNodeWithText("This phone is the new phone").assertExists()
        rule.onNodeWithText("Handover operation").assertExists()
        rule.onNodeWithText("0108 0F16", substring = true).assertExists()
        rule.onAllNodesWithText("Compare these 8 words", substring = true).assertCountEquals(0)
        rule.onNodeWithText("Sign my admission").assertIsEnabled().performClick()
        assertEquals(1, approved)
        assertTrue(admission.approveEnabled)
    }

    @Test fun d18_window_is_secure() {
        rule.setScreen { HandoverScreen(old, actions()) }
        assertFlagSecure(rule)
    }

    @Test fun old_phone_signed_says_it_must_be_retired_by_its_owner() {
        val h = TrustHandoff(HandoffKind.Handover, "New phone: pixel-9", TrustSamples.outgoing("handover-40"))
        rule.setScreen { HandoverScreen(old.copy(phase = TrustPhase.Signed, handoff = h), actions()) }
        rule.onNodeWithText("Your part is signed").assertExists()
        rule.onNodeWithText("Retire this phone yourself").assertExists()
        rule.onNode(hasContentDescription("QR code with the signed handover", substring = true)).assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
    }

    @Test fun new_phone_signed_waits_for_the_completed_handover() {
        val h = TrustHandoff(HandoffKind.Admission, "New phone: pixel-9", TrustSamples.outgoing("handover-41"))
        rule.setScreen { HandoverScreen(admission.copy(phase = TrustPhase.Signed, handoff = h), actions()) }
        rule.onNodeWithText("Admission signed").assertExists()
        rule.onNodeWithText("the handover is complete only when the terminal says so", substring = true).assertExists()
        rule.onAllNodesWithText("Retire this phone yourself").assertCountEquals(0)
    }

    @Test fun declined_and_closed_have_no_approve() {
        rule.setScreen { HandoverScreen(old.copy(phase = TrustPhase.Declined), actions()) }
        rule.onNodeWithText("Nothing was signed.").assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
    }

    @Test fun font_scale_2_has_no_overflow() {
        rule.setScreen(fontScale = 2f) { HandoverScreen(old, actions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }
}
