package uk.petruha.sshub.phone.release

import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.SemanticsMatcher
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.core.ReleaseOutcome
import uk.petruha.sshub.phone.fake.ReleaseSamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.release.ApprovalPhase
import uk.petruha.sshub.phone.ui.screens.release.RedActions
import uk.petruha.sshub.phone.ui.screens.release.RedScreen
import uk.petruha.sshub.phone.ui.screens.release.RedState

class RedScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val prefix = "Hash of the operation 1 B F 0, D 5 7 E, F 9 D D"
    private fun fieldNodes() = descStartsWith("Field ")

    private fun ComposeContentTestRule.show(
        initial: RedState, dark: Boolean = true, fontScale: Float = 1f, log: MutableList<String> = mutableListOf(),
    ): MutableState<RedState> {
        val st = mutableStateOf(initial)
        setScreen(dark, fontScale) {
            RedScreen(
                st.value,
                RedActions(
                    onBack = { log += "back" }, onApprove = { log += "approve" }, onDecline = { log += "decline" },
                    onToggleHash = { st.value = st.value.copy(showHash = !st.value.showHash) },
                    onCopy = { log += "copy"; st.value = st.value.copy(copied = true, clearsIn = 60) },
                ),
            )
        }
        return st
    }

    private val signed get() = ReleaseSamples.redState().copy(phase = ApprovalPhase.Approved, outcome = ReleaseOutcome(ReleaseSamples.outgoing()))
    private val declined get() = ReleaseSamples.redState().copy(phase = ApprovalPhase.Declined)

    @Test fun main_path_render_and_callbacks() {
        val log = mutableListOf<String>()
        rule.show(ReleaseSamples.redState(), log = log)
        rule.onNodeWithText("Revoke device laptop-old").assertExists()
        rule.onNodeWithText("proposed by laptop-main 7B3E-0D15, lamport 1042").assertExists()
        rule.onNodeWithText("6 min ago").assertExists()
        rule.onNodeWithText("lapses in 2 d 23 h").assertExists()
        rule.onNodeWithText("Approve").assertIsEnabled().performClick()
        rule.onNodeWithText("Decline").performClick()
        assertEquals(listOf("approve", "decline"), log)
    }

    @Test fun d1_red_banner_is_the_first_element_under_the_header() {
        rule.show(ReleaseSamples.redState())
        val header = rule.onNodeWithText("Red operation").fetchSemanticsNode().boundsInRoot
        val banner = rule.onNodeWithText("RED OPERATION").fetchSemanticsNode().boundsInRoot
        val title = rule.onNodeWithText("Revoke device laptop-old").fetchSemanticsNode().boundsInRoot
        assertTrue("banner below header", banner.top >= header.bottom - 1f)
        assertTrue("banner above all content", banner.bottom <= title.top + 1f)
    }

    @Test fun d2_d5_every_field_is_one_node_and_the_heading_count_equals_the_rows() {
        rule.show(ReleaseSamples.redState())
        rule.onNodeWithText("6 fields").assertExists()
        rule.onAllNodes(fieldNodes()).assertCountEquals(ReleaseSamples.redRevoke.fields.size)
        rule.onNodeWithContentDescription("Field Holders changes from 4 to 3").assertExists()
        rule.onNodeWithContentDescription("Field Its shares changes from valid (epoch 7) to void").assertExists()
        rule.onNodeWithContentDescription("Field Operation, new value Revoke holder").assertExists()
        rule.onNodeWithContentDescription("Field Credentials affected, new value 9 Quorum credentials are reshared").assertExists()
    }

    @Test fun d5_the_mode_variant_shows_all_its_fields_too() {
        rule.show(ReleaseSamples.redState(ReleaseSamples.redMode))
        rule.onNodeWithText("Switch credential mode to Quorum").assertExists()
        rule.onNodeWithText("6 fields").assertExists()
        rule.onAllNodes(fieldNodes()).assertCountEquals(6)
        rule.onNodeWithContentDescription("Field Mode changes from Single to Quorum").assertExists()
        rule.onNodeWithContentDescription("Proof 2 of 3, second, signed").assertExists()
    }

    @Test fun d6_three_proofs_exist_in_every_state() {
        val s = rule.show(ReleaseSamples.redState())
        listOf(ReleaseSamples.redState(), signed, declined, ReleaseSamples.redState(ReleaseSamples.redFailing), ReleaseSamples.redState(ReleaseSamples.redLapsed)).forEach {
            rule.runOnIdle { s.value = it }
            rule.onAllNodes(descStartsWith("Proof ")).assertCountEquals(3)
        }
        rule.runOnIdle { s.value = ReleaseSamples.redState() }
        rule.onNodeWithContentDescription("Proof 1 of 3, phone, your turn").assertExists()
        rule.onNodeWithContentDescription("Proof 3 of 3, factor, waiting").assertExists()
        rule.runOnIdle { s.value = signed }
        rule.onNodeWithContentDescription("Proof 1 of 3, phone, signed").assertExists()
        rule.runOnIdle { s.value = declined }
        rule.onNodeWithContentDescription("Proof 1 of 3, phone, refused").assertExists()
    }

    @Test fun d7_hash_prefix_is_always_there_and_a_tap_shows_all_16_groups() {
        val s = rule.show(ReleaseSamples.redState())
        rule.onNodeWithContentDescription(prefix).assertExists()
        rule.onNodeWithText("tap for all 64").assertExists()
        rule.onNodeWithContentDescription(prefix).performScrollTo().performClick()
        val full = rule.onNode(descStartsWith(prefix)).fetchSemanticsNode().config[SemanticsProperties.ContentDescription].first()
        assertEquals(15, full.count { it == ',' })
        assertTrue(full.endsWith("B 9 3 0"))
        rule.onNodeWithText("tap to fold").assertExists()
        for (state in listOf(signed, declined, ReleaseSamples.redState(ReleaseSamples.redFailing))) {
            rule.runOnIdle { s.value = state.copy(showHash = false) }
            rule.onNodeWithContentDescription(prefix).assertExists()
        }
    }

    @Test fun d8_approve_is_disabled_while_a_check_fails_and_names_the_reason() {
        rule.show(ReleaseSamples.redState(ReleaseSamples.redFailing))
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        rule.onNodeWithText("Approve").assert(
            SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Disabled, Hash does not match what the terminal shows"),
        )
        rule.onNodeWithContentDescription("Failed: Hash does not match what the terminal shows").assertExists()
        rule.onNodeWithText("Checks").assertExists()
        rule.onNodeWithText("3 of 4 pass").assertExists()
    }

    @Test fun d8_approve_is_enabled_when_every_check_passes() {
        rule.show(ReleaseSamples.redState())
        rule.onNodeWithText("Approve").assertIsEnabled()
        rule.onNodeWithText("4 of 4 pass").assertExists()
    }

    @Test fun d9_literal_labels() {
        rule.show(ReleaseSamples.redState())
        rule.onNodeWithText("Approve").assertExists()
        rule.onNodeWithText("sign with the phone").assertExists()
        rule.onNodeWithText("Decline").assertExists()
        listOf("Yes", "OK", "Continue", "Refuse").forEach { rule.onAllNodesWithText(it).assertCountEquals(0) }
    }

    @Test fun d10_window_closed_after_12_minutes_shows_the_message_and_no_approve() {
        val t0 = ReleaseSamples.T0
        val log = mutableListOf<String>()
        rule.show(ReleaseSamples.redState(now = t0, openedAt = t0 - 720), log = log)
        rule.onNodeWithText("Window closed").assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
        rule.onAllNodesWithText("Decline").assertCountEquals(0)
        rule.onNodeWithText("Back to approvals").performClick()
        assertEquals(listOf("back"), log)
    }

    @Test fun d10_the_countdown_is_a_polite_live_region_in_10_second_steps() {
        val t0 = ReleaseSamples.T0
        val s = rule.show(ReleaseSamples.redState(now = t0, openedAt = t0))
        rule.onNodeWithContentDescription("12 minutes left").assertExists()
        rule.runOnIdle { s.value = s.value.copy(now = t0 + 10) }
        rule.onNodeWithContentDescription("11 minutes 50 seconds left").assertExists()
        rule.runOnIdle { s.value = s.value.copy(now = t0 + 1) }
        rule.onNodeWithContentDescription("11 minutes 50 seconds left").assertExists()
        rule.onNodeWithContentDescription("11 minutes 50 seconds left").assert(
            SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, androidx.compose.ui.semantics.LiveRegionMode.Polite),
        )
    }


    @Test fun d10_a_lapsed_notice_cannot_be_approved() {
        rule.show(ReleaseSamples.redState(ReleaseSamples.redLapsed))
        rule.onNodeWithText("Lapsed").assertExists()
        rule.onNodeWithText("lapsed").assertExists()
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        rule.onNodeWithText("Approve").assert(
            SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Disabled, the request lapsed after 3 days"),
        )
    }

    @Test fun d11_a_tick_of_the_clock_never_fires_a_callback() {
        val log = mutableListOf<String>()
        val s = rule.show(ReleaseSamples.redState(), log = log)
        rule.runOnIdle { s.value = s.value.copy(now = s.value.now + 600) }
        rule.waitForIdle()
        assertEquals(emptyList<String>(), log)
    }

    @Test fun signed_hand_off_has_qr_text_with_copy_and_three_proofs() {
        val log = mutableListOf<String>()
        rule.show(signed, log = log)
        rule.onNodeWithText("Signed with this phone").assertExists()
        rule.onNode(descStartsWith("QR code")).assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
        rule.onNodeWithText("Copy as text").performScrollTo().performClick()
        assertEquals(listOf("copy"), log)
        rule.onNodeWithText("Copied, clears in 60 s").assertExists()
        rule.onNodeWithText("Done").performClick()
        assertEquals(listOf("copy", "back"), log)
        rule.onAllNodes(descStartsWith("Proof ")).assertCountEquals(3)
    }

    @Test fun nothing_is_copied_by_default() {
        rule.show(signed)
        rule.onAllNodesWithText("Copied, clears in 60 s").assertCountEquals(0)
    }

    @Test fun declined_state() {
        rule.show(declined)
        rule.onNodeWithText("DECLINED", substring = true).assertExists()
        rule.onNodeWithText("Declined.").assertExists()
        rule.onNodeWithText("Nothing was signed. The operation lapses on its own in 2 d 23 h.").assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
        rule.onNodeWithText("Back to approvals").assertExists()
    }

    @Test fun an_error_from_the_core_is_shown_and_approve_stays_possible() {
        rule.show(ReleaseSamples.redState().copy(error = "The system lock was canceled. Nothing was signed."))
        rule.onNodeWithText("The system lock was canceled. Nothing was signed.").assertExists()
        rule.onNodeWithText("Approve").assertIsEnabled()
    }

    @Test fun dark_theme() {
        rule.show(ReleaseSamples.redState(), dark = true)
        rule.onNodeWithText("RED OPERATION", substring = true).assertExists()
        rule.assertTargets48()
    }

    @Test fun light_theme_signed() {
        rule.show(signed, dark = false)
        rule.onNodeWithText("Signed with this phone").assertExists()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_visual_overflow() {
        rule.show(ReleaseSamples.redState(), fontScale = 2f)
        rule.onNodeWithText("Revoke device laptop-old").assertExists()
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_failing_and_signed_states() {
        val s = rule.show(ReleaseSamples.redState(ReleaseSamples.redFailing), fontScale = 2f)
        rule.assertNoVisualOverflow()
        rule.runOnIdle { s.value = signed }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_1_3() {
        rule.show(ReleaseSamples.redState(ReleaseSamples.redMode), fontScale = 1.3f)
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun d18_flag_secure_is_set_while_the_screen_is_shown() {
        rule.show(ReleaseSamples.redState())
        rule.waitForIdle()
        var flags = 0
        rule.activityRule.scenario.onActivity { flags = it.window.attributes.flags }
        assertTrue("FLAG_SECURE", flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
    }

    @Test fun d12_no_node_offers_a_continue_anyway() {
        rule.show(ReleaseSamples.redState(ReleaseSamples.redFailing))
        rule.onAllNodes(hasText("continue anyway", ignoreCase = true, substring = true)).assertCountEquals(0)
        rule.onAllNodes(hasContentDescription("Approve all", substring = true, ignoreCase = true)).assertCountEquals(0)
    }
}
