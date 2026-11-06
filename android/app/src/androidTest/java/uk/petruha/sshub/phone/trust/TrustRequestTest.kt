package uk.petruha.sshub.phone.trust

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.Parameterized
import uk.petruha.sshub.phone.core.TrustCheck
import uk.petruha.sshub.phone.fake.TrustSamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.trust.TrustPhase
import uk.petruha.sshub.phone.ui.screens.trust.TrustRequestScreen
import uk.petruha.sshub.phone.ui.screens.trust.TrustRequestState

@RunWith(AndroidJUnit4::class)
class TrustRequestTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val ok = TrustRequestState(TrustSamples.requestOk)
    private val unmet = TrustRequestState(TrustSamples.requestUnmet)

    @Test fun main_path_approve_and_decline_call_back() {
        var approved = 0; var declined = 0
        rule.setScreen { TrustRequestScreen(ok, requestActions(onApprove = { approved++ }, onDecline = { declined++ })) }
        rule.onNodeWithText("TRUST CERTIFICATE").assertExists()
        rule.onNodeWithText("sign with the phone").assertExists()
        rule.onNodeWithText("Approve").assertIsEnabled().performClick()
        rule.onNodeWithText("Decline").performClick()
        assertEquals(1, approved); assertEquals(1, declined)
        rule.assertTargets48()
    }

    @Test fun shows_device_hosts_term_budget_and_epoch_range_with_checks() {
        rule.setScreen { TrustRequestScreen(ok, requestActions()) }
        rule.onNode(hasContentDescription("Field Trusted device, new value laptop-main [7B3E-0D15]")).assertExists()
        rule.onNode(hasContentDescription("Field Hosts (5), new value web-01, web-02, db-staging, bastion, nuc-lab")).assertExists()
        rule.onNode(hasContentDescription("Field Term, new value 30 days, ends 2026-11-25")).assertExists()
        rule.onNode(hasContentDescription("Field New hosts per day, new value 10")).assertExists()
        rule.onNode(hasContentDescription("Field Epoch range", substring = true)).assertExists()
        rule.onNode(hasContentDescription("Passed: starts at this phone's own epoch 7")).assertExists()
        rule.onNode(hasContentDescription("Passed: at most one epoch wide")).assertExists()
        rule.onAllNodesWithText("Never met").assertCountEquals(0)
    }

    @Test fun d5_every_host_is_listed_and_the_count_equals_the_names() {
        rule.setScreen { TrustRequestScreen(TrustRequestState(TrustSamples.requestRefused(3)), requestActions()) }
        val d = rule.onNode(hasContentDescription("Field Hosts (65)", substring = true)).description()
        TrustSamples.MANY_HOSTS.forEach { assertTrue("missing $it", d.contains(it)) }
        assertFalse(d.contains(" more"))
    }

    @Test fun d13_primary_disabled_until_typed_id_matches() {
        var approved = 0
        rule.setScreen {
            var st by remember { mutableStateOf(unmet) }
            TrustRequestScreen(st, requestActions(onTyped = { st = st.copy(typed = it) }, onApprove = { approved++ }))
        }
        rule.onNodeWithText("Never met").assertExists()
        rule.onNode(hasContentDescription("Device id E 7 A 4, 6 B 0 2")).assertExists()
        rule.onAllNodesWithText("not met [E7A4-6B02]", substring = true).onFirst().assertExists()
        rule.onNodeWithText("Approve").assertIsNotEnabled().performClick()
        assertEquals(0, approved)
        rule.onNodeWithContentDescription("Type the device id of the requester").performTextInput("E7A4-6B0")
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        rule.onNodeWithContentDescription("Type the device id of the requester").performTextInput("2")
        rule.onNodeWithText("Approve").assertIsEnabled().performClick()
        assertEquals(1, approved)
    }

    @Test fun n7_n_l9_the_hint_points_at_the_field_under_the_warning_and_the_banner_counts_nothing() {
        rule.setScreen { TrustRequestScreen(unmet, requestActions()) }
        rule.onNodeWithText("Type the device id in the field under the warning to enable Approve.").assertExists()
        rule.onAllNodesWithText("Type the device id above", substring = true).assertCountEquals(0)
        rule.onNodeWithText("Red operation").assertExists()
        rule.onAllNodesWithText("Red: 3 proofs").assertCountEquals(0)
    }

    @Test fun d8_approve_enabled_iff_all_checks_pass() {
        assertTrue(ok.approveEnabled)
        val failed = ok.view.epoch!!.copy(checks = listOf(TrustCheck("starts at this phone's own epoch 7", false), TrustCheck("at most one epoch wide", true)))
        assertFalse(ok.copy(view = ok.view.copy(epoch = failed)).approveEnabled)
        assertTrue(ok.copy(view = ok.view.copy(epoch = null)).approveEnabled)
        assertFalse(TrustRequestState(TrustSamples.requestRefused(0)).approveEnabled)
        assertFalse(ok.copy(busy = true).approveEnabled)
        assertFalse(ok.copy(phase = TrustPhase.Declined).approveEnabled)
        assertFalse(unmet.approveEnabled)
        assertFalse(unmet.copy(typed = "E7A4-6B0").approveEnabled)
        assertTrue(unmet.copy(typed = " e7a4-6b02 ").approveEnabled)
    }

    @Test fun d8_a_failed_check_is_a_red_line_and_disables_approve() {
        val failed = ok.view.epoch!!.copy(checks = listOf(TrustCheck("starts at this phone's own epoch 7", false), TrustCheck("at most one epoch wide", true)))
        rule.setScreen { TrustRequestScreen(ok.copy(view = ok.view.copy(epoch = failed)), requestActions()) }
        rule.onNode(hasContentDescription("Failed: starts at this phone's own epoch 7")).assertExists()
        rule.onNodeWithText("Approve").assertIsNotEnabled()
    }

    @Test fun d9_literal_wording() {
        rule.setScreen { TrustRequestScreen(ok, requestActions()) }
        rule.onNodeWithText("Approve").assertExists()
        rule.onNodeWithText("sign with the phone").assertExists()
        rule.onNodeWithText("Decline").assertExists()
        rule.onAllNodesWithText("Yes").assertCountEquals(0)
        rule.onAllNodesWithText("Continue").assertCountEquals(0)
    }

    @Test fun d18_window_is_secure() {
        rule.setScreen { TrustRequestScreen(ok, requestActions()) }
        assertFlagSecure(rule)
    }

    @Test fun l5_declined_ends_without_signing_and_does_not_offer_to_review_again() {
        var back = 0
        rule.setScreen { TrustRequestScreen(ok.copy(phase = TrustPhase.Declined), requestActions(onBack = { back++ })) }
        rule.onNodeWithText("DECLINED").assertExists()
        rule.onNodeWithText("Nothing was signed. This phone dropped the request: to look at it again, ask the terminal for a new one.").assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
        rule.onAllNodesWithText("Review again").assertCountEquals(0)
        rule.onNodeWithText("Back to approvals").performClick()
        assertEquals(1, back)
    }

    @Test fun window_closed_after_12_minutes_has_no_approve() {
        var back = 0
        rule.setScreen { TrustRequestScreen(ok.copy(phase = TrustPhase.Closed), requestActions(onBack = { back++ })) }
        rule.onNodeWithText("Window closed.").assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
        rule.onNodeWithText("Back to approvals").performClick()
        assertEquals(1, back)
    }

    @Test fun error_from_the_core_is_shown() {
        rule.setScreen { TrustRequestScreen(ok.copy(error = "Unlock was canceled. Nothing was signed."), requestActions()) }
        rule.onNodeWithText("Unlock was canceled. Nothing was signed.").assertExists()
    }

    @Test fun renders_dark() {
        rule.setScreen(dark = true) { TrustRequestScreen(unmet, requestActions()) }
        rule.onNodeWithText("Never met").assertExists()
        rule.assertTargets48()
    }

    @Test fun renders_light() {
        rule.setScreen(dark = false) { TrustRequestScreen(ok, requestActions()) }
        rule.onNodeWithText("Approve").assertExists()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow_unmet() {
        rule.setScreen(fontScale = 2f) { TrustRequestScreen(unmet, requestActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow_with_65_hosts() {
        rule.setScreen(fontScale = 2f) { TrustRequestScreen(TrustRequestState(TrustSamples.requestRefused(3)), requestActions()) }
        rule.assertNoVisualOverflow()
    }
}

/** Refusals 0..8 of the prototype: a refusal screen never has an Approve button (spec 12, C21). */
@RunWith(Parameterized::class)
class TrustRequestRefusalTest(private val n: Int) {
    companion object {
        @JvmStatic @Parameterized.Parameters(name = "refusal {0}")
        fun cases(): List<Int> = (0..8).toList()
    }

    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun refusal_shows_reason_and_never_an_approve() {
        var back = 0
        rule.setScreen { TrustRequestScreen(TrustRequestState(TrustSamples.requestRefused(n)), requestActions(onBack = { back++ })) }
        rule.onNodeWithText("Refused. Nothing was signed.").assertExists()
        rule.onNodeWithText(TrustSamples.REFUSAL_TEXTS[n]).assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
        rule.onAllNodesWithText("sign with the phone").assertCountEquals(0)
        rule.onAllNodesWithText("Decline").assertCountEquals(0)
        rule.onNodeWithText("Back to approvals").performClick()
        assertEquals(1, back)
        rule.assertTargets48()
    }
}
