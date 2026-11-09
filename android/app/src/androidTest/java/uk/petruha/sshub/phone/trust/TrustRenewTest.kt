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
import uk.petruha.sshub.phone.core.RENEW_EVIDENCE_MAX_AGE_SECS
import uk.petruha.sshub.phone.core.TrustHandoff
import uk.petruha.sshub.phone.fake.TrustSamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.trust.TrustPhase
import uk.petruha.sshub.phone.ui.screens.trust.TrustRenewScreen
import uk.petruha.sshub.phone.ui.screens.trust.TrustRenewState

@RunWith(AndroidJUnit4::class)
class TrustRenewTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val fresh = TrustRenewState(TrustSamples.renewFresh)

    @Test fun fresh_evidence_shows_every_field_and_approve_calls_back() {
        var approved = 0; var declined = 0
        rule.setScreen { TrustRenewScreen(fresh, extendActions(onApprove = { approved++ }, onDecline = { declined++ })) }
        rule.onNodeWithText("Renewal requested").assertExists()
        rule.onNodeWithText("workstation [9C61-E2A8]").assertExists()
        rule.onNodeWithText("3 listed hosts").assertExists()
        rule.onNodeWithText("2026-11-17 (adds 14 days 0 hours)").assertExists()
        rule.onNodeWithText("Renewals left").assertExists()
        rule.onNodeWithText("1 minute old").assertExists()
        rule.onNodeWithText(TrustSamples.renewFresh.note).assertExists()
        rule.onNodeWithText("Approve").assertIsEnabled().performClick()
        rule.onNodeWithText("Decline").performClick()
        assertEquals(1, approved); assertEquals(1, declined)
        rule.assertTargets48()
    }

    @Test fun the_opened_hosts_are_listed_with_the_rest_counted() {
        rule.setScreen { TrustRenewScreen(fresh, extendActions()) }
        rule.onNodeWithText("web-01\ndb-staging\nand 9 more (11 in all)").assertExists()
    }

    @Test fun no_list_sent_is_not_shown_as_nothing_opened() {
        rule.setScreen { TrustRenewScreen(TrustRenewState(TrustSamples.renewNoList), extendActions()) }
        rule.onNodeWithText("no list sent", substring = true).assertExists()
        rule.onAllNodesWithText("and 0 more", substring = true).assertCountEquals(0)
    }

    @Test fun d18_window_is_secure() {
        rule.setScreen { TrustRenewScreen(fresh, extendActions()) }
        assertFlagSecure(rule)
    }

    @Test fun stale_evidence_is_red_and_cannot_be_approved() {
        rule.setScreen { TrustRenewScreen(TrustRenewState(TrustSamples.renewStale), extendActions()) }
        rule.onNode(hasContentDescription("Danger: Freshest news from another terminal: 1 day 0 hours old")).assertExists()
        rule.onNodeWithText("Approve").assertIsNotEnabled()
    }

    @Test fun evidence_only_through_the_delegate_is_red_and_cannot_be_approved() {
        rule.setScreen { TrustRenewScreen(TrustRenewState(TrustSamples.renewDelegateOnly), extendActions()) }
        rule.onNode(hasContentDescription("Danger: Freshest news from another terminal: none: only the trusted device itself vouches for the list")).assertExists()
        rule.onNodeWithText("Approve").assertIsNotEnabled()
    }

    @Test fun a_refusal_shows_the_reason_and_never_an_approve() {
        var back = 0
        rule.setScreen { TrustRenewScreen(TrustRenewState(TrustSamples.renewOverCap), extendActions(onBack = { back++ })) }
        rule.onNodeWithText("Request refused").assertExists()
        rule.onNodeWithText(TrustSamples.renewOverCap.refusal!!.text).assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
        rule.onAllNodesWithText("Decline").assertCountEquals(0)
        rule.onNodeWithText("Back to approvals").performClick()
        assertEquals(1, back)
    }

    @Test fun signed_shows_the_handoff() {
        var done = 0
        val h = TrustHandoff(HandoffKind.Renewal, TrustSamples.renewFresh.signedSummary, TrustSamples.outgoing("renew-30"))
        rule.setScreen { TrustRenewScreen(fresh.copy(phase = TrustPhase.Signed, handoff = h), extendActions(onDone = { done++ })) }
        rule.onNodeWithText("Renewal signed").assertExists()
        rule.onNode(hasContentDescription("QR code with the signed renewal", substring = true)).assertExists()
        rule.onNodeWithText("Done").performClick()
        assertEquals(1, done)
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
    }

    @Test fun font_scale_2_has_no_overflow() {
        rule.setScreen(fontScale = 2f) { TrustRenewScreen(TrustRenewState(TrustSamples.renewStale), extendActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun the_evidence_limit_is_six_hours_inclusive() {
        val v = TrustSamples.renewFresh
        assertFalse(v.copy(evidenceAgeSecs = RENEW_EVIDENCE_MAX_AGE_SECS).red)
        assertTrue(v.copy(evidenceAgeSecs = RENEW_EVIDENCE_MAX_AGE_SECS + 1).red)
        assertTrue(v.copy(evidenceAgeSecs = null).red)
        assertTrue(TrustRenewState(v.copy(evidenceAgeSecs = RENEW_EVIDENCE_MAX_AGE_SECS + 1)).approveEnabled.not())
        assertTrue(fresh.approveEnabled)
        assertFalse(fresh.copy(busy = true).approveEnabled)
    }
}
