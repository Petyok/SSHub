package io.github.petyok.sshub.trust

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
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
import org.junit.runners.Parameterized
import io.github.petyok.sshub.core.HandoffKind
import io.github.petyok.sshub.core.TrustExtendView
import io.github.petyok.sshub.core.TrustHandoff
import io.github.petyok.sshub.fake.TrustSamples
import io.github.petyok.sshub.testing.assertNoVisualOverflow
import io.github.petyok.sshub.testing.assertTargets48
import io.github.petyok.sshub.testing.setScreen
import io.github.petyok.sshub.ui.screens.trust.TrustExtendScreen
import io.github.petyok.sshub.ui.screens.trust.TrustExtendState
import io.github.petyok.sshub.ui.screens.trust.TrustPhase

@RunWith(AndroidJUnit4::class)
class TrustExtendTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val ok = TrustExtendState(TrustSamples.extendOk)

    @Test fun main_path_shows_the_change_and_approve_decline_call_back() {
        var approved = 0; var declined = 0
        rule.setScreen { TrustExtendScreen(ok, extendActions(onApprove = { approved++ }, onDecline = { declined++ })) }
        rule.onNodeWithText("BUDGET EXTENSION").assertExists()
        rule.onNodeWithText("Budget extension requested").assertExists()
        rule.onNode(hasContentDescription("Field New hosts allowed in the next 24 hours changes from 10 per day, 10 of 10 used to 10 more, for 24 hours only")).assertExists()
        rule.onNodeWithText("sign with the phone").assertExists()
        rule.onNodeWithText("Approve").assertIsEnabled().performClick()
        rule.onNodeWithText("Decline").performClick()
        assertEquals(1, approved); assertEquals(1, declined)
        rule.assertTargets48()
    }

    @Test fun d18_window_is_secure() {
        rule.setScreen { TrustExtendScreen(ok, extendActions()) }
        assertFlagSecure(rule)
    }

    @Test fun approve_is_disabled_while_signing() {
        assertTrue(ok.approveEnabled)
        assertFalse(ok.copy(busy = true).approveEnabled)
        assertFalse(ok.copy(phase = TrustPhase.Declined).approveEnabled)
        assertFalse(TrustExtendState(TrustSamples.extendAgain).approveEnabled)
    }

    @Test fun l5_declined_offers_back_to_approvals_and_no_review_again() {
        var back = 0
        rule.setScreen { TrustExtendScreen(ok.copy(phase = TrustPhase.Declined), extendActions(onBack = { back++ })) }
        rule.onNodeWithText("DECLINED").assertExists()
        rule.onNodeWithText("Nothing was signed.").assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
        rule.onAllNodesWithText("Review again").assertCountEquals(0)
        rule.onNodeWithText("Back to approvals").performClick()
        assertEquals(1, back)
    }

    @Test fun signed_shows_the_handoff() {
        var copied = 0; var done = 0
        val h = TrustHandoff(HandoffKind.Extension, TrustSamples.extendOk.signedSummary, TrustSamples.outgoing("ext-20"))
        rule.setScreen { TrustExtendScreen(ok.copy(phase = TrustPhase.Signed, handoff = h), extendActions(onCopy = { copied++ }, onDone = { done++ })) }
        rule.onNodeWithText("SIGNED").assertExists()
        rule.onNodeWithText("workstation [9C61-E2A8], +10 new hosts for 24 hours").assertExists()
        rule.onNode(hasContentDescription("QR code with the signed budget extension", substring = true)).assertExists()
        rule.onNodeWithText("Copy the text").performClick()
        rule.onNodeWithText("Done").performClick()
        assertEquals(1, copied); assertEquals(1, done)
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
    }

    @Test fun window_closed_has_no_approve() {
        rule.setScreen { TrustExtendScreen(ok.copy(phase = TrustPhase.Closed), extendActions()) }
        rule.onNodeWithText("Window closed.").assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
    }

    @Test fun again_shows_the_wait_on_a_tick_scale() {
        rule.setScreen { TrustExtendScreen(TrustExtendState(TrustSamples.extendAgain), extendActions()) }
        rule.onNode(hasContentDescription("Possible again in 17 hours 22 minutes")).assertExists()
        rule.onNodeWithText("17 h 22 min").assertExists()
    }

    @Test fun renders_dark() {
        rule.setScreen(dark = true) { TrustExtendScreen(ok, extendActions()) }
        rule.onNodeWithText("Approve").assertExists()
        rule.assertTargets48()
    }

    @Test fun renders_light() {
        rule.setScreen(dark = false) { TrustExtendScreen(ok, extendActions()) }
        rule.onNodeWithText("Approve").assertExists()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow() {
        rule.setScreen(fontScale = 2f) { TrustExtendScreen(ok, extendActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow_again() {
        rule.setScreen(fontScale = 2f) { TrustExtendScreen(TrustExtendState(TrustSamples.extendAgain), extendActions()) }
        rule.assertNoVisualOverflow()
    }
}

/** Refusals notmine, revoked, again: never an Approve button. */
@RunWith(Parameterized::class)
class TrustExtendRefusalTest(private val name: String, private val view: TrustExtendView) {
    companion object {
        @JvmStatic @Parameterized.Parameters(name = "{0}")
        fun cases(): List<Array<Any>> = listOf(
            arrayOf("notmine", TrustSamples.extendNotMine), arrayOf("revoked", TrustSamples.extendRevoked), arrayOf("again", TrustSamples.extendAgain),
        )
    }

    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun refusal_shows_reason_and_never_an_approve() {
        var back = 0
        rule.setScreen { TrustExtendScreen(TrustExtendState(view), extendActions(onBack = { back++ })) }
        rule.onNodeWithText(view.refusal!!.title).assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
        rule.onAllNodesWithText("Decline").assertCountEquals(0)
        rule.onNodeWithText("Back to approvals").performClick()
        assertEquals(1, back)
        rule.assertTargets48()
    }
}
