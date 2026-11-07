package uk.petruha.sshub.phone.release

import android.view.WindowManager
import androidx.compose.ui.test.assert
import androidx.activity.ComponentActivity
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.core.ChannelKind
import uk.petruha.sshub.phone.core.IssuedShortCode
import uk.petruha.sshub.phone.core.ReleaseOutcome
import uk.petruha.sshub.phone.core.ShareWords
import uk.petruha.sshub.phone.fake.ReleaseSamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.release.ApprovalPhase
import uk.petruha.sshub.phone.ui.screens.release.ReleaseActions
import uk.petruha.sshub.phone.ui.screens.release.ReleaseState
import uk.petruha.sshub.phone.ui.screens.release.ShareReleaseScreen

class ShareReleaseScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val dbs get() = ReleaseSamples.releaseState()
    private val bastion get() = ReleaseSamples.releaseState(ReleaseSamples.releaseBastion)

    private fun ComposeContentTestRule.show(
        initial: ReleaseState, dark: Boolean = true, fontScale: Float = 1f, log: MutableList<String> = mutableListOf(),
    ): MutableState<ReleaseState> {
        val st = mutableStateOf(initial)
        setScreen(dark, fontScale) {
            ShareReleaseScreen(
                st.value,
                ReleaseActions(
                    onBack = { log += "back" }, onApprove = { log += "approve" }, onDecline = { log += "decline" },
                    onTyped = { st.value = st.value.copy(typed = it) },
                    onChannel = { k, on -> log += "channel $k $on"; st.value = st.value.copy(channelsOn = if (on) st.value.channelsOn + k else st.value.channelsOn - k) },
                    onSound = { k -> log += "sound $k"; st.value = st.value.copy(channelsOn = st.value.channelsOn - ChannelKind.SoundFast - ChannelKind.SoundSlow + listOfNotNull(k)) },
                    onShortCode = { log += "short $it"; st.value = st.value.copy(shortCode = it) },
                    onByHand = { st.value = st.value.copy(byHand = it) },
                    onCopy = { log += "copy"; st.value = st.value.copy(copied = true, clearsIn = 60) },
                    onAskAgain = { log += "again" },
                    onScanId = { log += "scan-id" },
                ),
            )
        }
        return st
    }

    private val released get() = dbs.copy(
        phase = ApprovalPhase.Approved,
        outcome = ReleaseOutcome(
            ReleaseSamples.outgoing(),
            IssuedShortCode(ReleaseSamples.SHORT_CODE, ReleaseSamples.T0 + 7 * 86_400, false),
            ShareWords(2, ReleaseSamples.words24),
        ),
        shortCode = true, byHand = true,
    )

    @Test fun main_path_render_and_callbacks() {
        val log = mutableListOf<String>()
        rule.show(dbs, log = log)
        rule.onNodeWithText("workstation").assertExists()
        rule.onNodeWithText("9C61-E2A8").assertExists()
        rule.onNodeWithText("hybrid, sharing round 7, share 2").assertExists()
        rule.onNodeWithText("signed 38 s ago", substring = true, useUnmergedTree = true).assertExists()
        rule.onNodeWithText("Compare the host and the four words with the terminal.").assertExists()
        // H1: the host id in groups of 4 and every row of the signed op, danger rows with the red gutter
        rule.onNode(descStartsWith("Host id 0 A 0 B, 0 C 0 D"), useUnmergedTree = true).assertExists()
        rule.onNodeWithText("What you release").assertExists()
        rule.onNodeWithText("3 fields").assertExists()
        rule.onNodeWithContentDescription("Field host_uid, new value ${ReleaseSamples.HOST_UID}").assertExists()
        rule.onNodeWithContentDescription("Field requester, new value workstation [9C61-E2A8]").assertExists()
        rule.onAllNodesWithText("The terminal shows which host", substring = true).assertCountEquals(0)
        val key = rule.onNode(descStartsWith("Requester key"), useUnmergedTree = true).fetchSemanticsNode()
            .config[SemanticsProperties.ContentDescription].first()
        assertEquals(15, key.count { it == ',' })
        rule.onNodeWithText("Approve").assertIsEnabled().performClick()
        rule.onNodeWithText("Decline").performClick()
        assertEquals(listOf("approve", "decline"), log)
    }

    @Test fun timer_card_counts_down_in_whole_seconds_and_speaks_at_the_steps() {
        val s = rule.show(dbs)
        rule.onNodeWithContentDescription("Request open, up to 120 seconds left").assertExists()
        rule.onNodeWithContentDescription("Request valid for 120 seconds, 82 seconds left").assertExists()
        rule.onNodeWithContentDescription("Request open, up to 120 seconds left").assert(
            SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Polite),
        )
        rule.runOnIdle { s.value = s.value.copy(now = s.value.now + 22) }          // 60 s left
        rule.onNodeWithContentDescription("Request open, up to 60 seconds left").assertExists()
        rule.runOnIdle { s.value = s.value.copy(now = s.value.now + 1) }           // 59 s left, same words
        rule.onNodeWithContentDescription("Request open, up to 60 seconds left").assertExists()
        rule.runOnIdle { s.value = s.value.copy(now = s.value.view.expiresAt - 10) }
        rule.onNodeWithContentDescription("Request open, up to 10 seconds left").assertExists()
    }

    @Test fun d10_an_expired_challenge_cannot_be_approved_and_offers_asking_again() {
        val log = mutableListOf<String>()
        val v = ReleaseSamples.releaseDbs
        rule.show(ReleaseSamples.releaseState(now = v.expiresAt), log = log)
        rule.onNodeWithContentDescription("Request expired").assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
        rule.onNodeWithText("Ask the terminal again").performClick()
        assertEquals(listOf("again"), log)
    }

    @Test fun d10_window_closed_after_12_minutes() {
        val t0 = ReleaseSamples.T0
        val log = mutableListOf<String>()
        rule.show(ReleaseSamples.releaseState(now = t0, openedAt = t0 - 720), log = log)
        rule.onNodeWithText("Window closed").assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
        rule.onNodeWithText("Back to approvals").performClick()
        assertEquals(listOf("back"), log)
    }

    @Test fun d10_the_screen_window_is_the_one_timer_only_when_it_ends_before_the_request() {
        val t0 = ReleaseSamples.T0
        rule.show(ReleaseSamples.releaseState(now = t0, openedAt = t0 - 700)) // 20 s of screen window left, the request has 82
        rule.onAllNodesWithText("expires in", substring = true).assertCountEquals(1)
        rule.onNodeWithText("expires in 0:20").assertExists()
        rule.onNodeWithContentDescription("20 seconds left").assertExists()
    }

    @Test fun u13_17_one_timer_with_the_requests_own_time_and_no_approval_window() {
        val v = ReleaseSamples.releaseDbs
        rule.show(ReleaseSamples.releaseState(now = v.expiresAt - 120)) // 120 s left, a fresh 12 minute screen window
        rule.onAllNodesWithText("expires in", substring = true).assertCountEquals(1)
        rule.onNodeWithText("expires in 2:00").assertExists()
        rule.onAllNodesWithText("approval window", substring = true).assertCountEquals(0)
        rule.onAllNodesWithText("s left", substring = true).assertCountEquals(0)
        rule.onAllNodesWithText("expires after", substring = true).assertCountEquals(0)
        rule.onAllNodesWithText("12:00").assertCountEquals(0)
    }

    @Test fun u13_16_the_typed_id_field_has_a_scan_button_and_a_scanned_id_enables_approve() {
        val log = mutableListOf<String>()
        val v = ReleaseSamples.releaseBastion
        val st = rule.show(ReleaseSamples.releaseState(v), log = log)
        rule.onNodeWithText("Scan its QR").performScrollTo().performClick()
        assertEquals(listOf("scan-id"), log)
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        rule.runOnIdle { st.value = st.value.copy(typed = v.mustType!!) } // what the scan screen hands to `typed`
        rule.onNodeWithText("Approve").assertIsEnabled()
    }

    @Test fun d15_four_binding_words_whole_both_buttons_enabled_none_preselected() {
        rule.show(dbs)
        rule.onNodeWithContentDescription("compile, trash, chop, valley").assertExists()
        rule.onNodeWithText("Binding words").assertExists()
        rule.onNodeWithText("Approve").assertIsEnabled()
        rule.onNodeWithText("Decline").assertIsEnabled()
        listOf("They match", "They differ").forEach { rule.onAllNodesWithText(it).assertCountEquals(0) }
    }

    @Test fun d8_checks_are_listed_with_the_count() {
        rule.show(dbs)
        rule.onNodeWithText("4 of 4 pass").assertExists()
        rule.onNodeWithContentDescription("Passed: Not expired").assertExists()
    }

    @Test fun d8_a_failing_check_disables_approve_and_names_the_reason() {
        val bad = ReleaseSamples.releaseDbs.copy(checks = ReleaseSamples.releaseDbs.checks.dropLast(1) + uk.petruha.sshub.phone.core.ReleaseCheck("Own share cannot be unwrapped for deploy@db-staging", false))
        rule.show(ReleaseSamples.releaseState(bad))
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        rule.onNodeWithText("Approve").assert(
            SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Disabled, Own share cannot be unwrapped for deploy@db-staging"),
        )
        rule.onNodeWithContentDescription("Failed: Own share cannot be unwrapped for deploy@db-staging").assertExists()
        rule.onNodeWithText("3 of 4 pass").assertExists()
    }

    @Test fun d13_primary_disabled_until_typed_id_matches() {
        val s = rule.show(bastion)
        rule.onNodeWithText("This requester has never met this phone.").assertExists()
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        rule.onNodeWithText("Approve").assert(
            SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Disabled, type the id first"),
        )
        rule.onNodeWithText("type the id first").assertExists()
        rule.onNodeWithContentDescription("Typed requester id").performScrollTo().performTextInput("E7A4-6B03")
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        rule.onNodeWithText("does not match").assertExists()
        rule.runOnIdle { s.value = s.value.copy(typed = "") }
        rule.onNodeWithContentDescription("Typed requester id").performScrollTo().performTextInput("e7a4-6b02")
        rule.onNodeWithText("Approve").assertIsEnabled()
        rule.onAllNodesWithText("does not match").assertCountEquals(0)
        rule.onAllNodesWithText("type the id first").assertCountEquals(0)
    }

    @Test fun d13_never_met_box_shows_name_and_id_and_no_key_for_an_unverified_requester() {
        rule.show(bastion)
        rule.onNodeWithText("not met E7A4-6B02", substring = true).assertExists()
        rule.onAllNodes(descStartsWith("Requester key"), useUnmergedTree = true).assertCountEquals(0)
        rule.onNodeWithText("envelope, sharing round 7, share 1").assertExists()
        rule.onNodeWithText("Type E7A4-6B02", substring = true).assertExists()
    }

    @Test fun m4_a_request_on_a_phone_that_has_not_taken_its_chain_shows_the_stop_box_and_no_approve() {
        rule.show(ReleaseSamples.releaseState(ReleaseSamples.releaseDbs.copy(headPending = true)))
        rule.onNodeWithText("This phone does not sign yet.").assertExists()
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        rule.onNodeWithText("Approve").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Disabled, take the chain first"))
    }
    @Test fun a_met_requester_has_no_typed_id_gate() {
        rule.show(dbs)
        rule.onAllNodesWithText("This requester has never met this phone.").assertCountEquals(0)
        rule.onAllNodes(androidx.compose.ui.test.hasContentDescription("Typed requester id")).assertCountEquals(0)
    }

    @Test fun m3_defaults_are_qr_on_and_sound_off_and_sound_is_one_choice() {
        val log = mutableListOf<String>()
        val s = rule.show(dbs, log = log)
        assertEquals(setOf(ChannelKind.Qr), s.value.channelsOn)
        rule.onNodeWithText("1 on").assertExists()
        rule.onAllNodesWithText("Clipboard").assertCountEquals(0)
        rule.onNodeWithText("QR code").performScrollTo().performClick()
        rule.onNodeWithText("nothing chosen").assertExists()
        rule.onNodeWithText("Sound (fast)").performScrollTo().performClick()
        rule.onNodeWithText("Sound (slow)").performScrollTo().performClick()
        assertEquals(setOf(ChannelKind.SoundSlow), s.value.channelsOn)
        rule.onNodeWithText("Off").performScrollTo().performClick()
        assertTrue(s.value.channelsOn.isEmpty())
        rule.onNodeWithText("Also issue a short code").performScrollTo().performClick()
        rule.onNodeWithText("Opens this share later without the phone, 7 days").assertExists()
        assertEquals(listOf("channel ${ChannelKind.Qr} false", "sound ${ChannelKind.SoundFast}", "sound ${ChannelKind.SoundSlow}", "sound null", "short true"), log)
    }

    @Test fun m3_with_no_channel_and_no_hand_approve_is_disabled_and_says_why() {
        rule.show(dbs.copy(channelsOn = emptySet()))
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        rule.onNodeWithText("Approve").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Disabled, choose how to show the answer"))
    }

    @Test fun m3_by_hand_alone_is_enough_to_approve() {
        rule.show(dbs.copy(channelsOn = emptySet(), byHand = true))
        rule.onNodeWithText("Approve").assertIsEnabled()
    }

    @Test fun type_it_by_hand_explains_that_the_words_come_after_approve() {
        val s = rule.show(dbs)
        rule.onAllNodesWithText("Look around first.").assertCountEquals(0)
        rule.onNodeWithText("Type it by hand").performScrollTo().performClick()
        rule.onNodeWithText("Look around first.").assertExists()
        assertTrue(s.value.byHand)
    }

    @Test fun released_state_shows_qr_copy_short_code_and_covered_words_without_copy_buttons() {
        val log = mutableListOf<String>()
        rule.show(released, log = log)
        rule.onNodeWithText("Signed.").assertExists()
        rule.onNodeWithText("Shown as: QR code. Check on the terminal that it took the share.").assertExists()
        rule.onAllNodesWithText("Sent by", substring = true).assertCountEquals(0)
        rule.onAllNodesWithText("left this phone", substring = true).assertCountEquals(0)
        rule.onNode(descStartsWith("QR code")).assertExists()
        rule.onNodeWithContentDescription("Short code, shown once: K7Q2, 9FMX, C41T, 8PWD").assertExists()
        rule.onNodeWithText("share index 2").assertExists()
        rule.onNode(descStartsWith("Words hidden")).assertExists()
        rule.onNodeWithContentDescription("Share released, up to 120 seconds left").assertExists()
        // the only copy on the screen is the explicit one for the reply (D19); words and the short code have none
        rule.onAllNodes(hasText("Copy", substring = true)).assertCountEquals(1)
        rule.onNodeWithText("Copy as text").performScrollTo().performClick()
        rule.onNodeWithText("Copied, clears in 60 s").assertExists()
        assertEquals(listOf("copy"), log)
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
        rule.onNodeWithText("Back to approvals").assertExists()
    }

    @Test fun released_without_the_qr_channel_shows_no_qr() {
        val st = released.copy(channelsOn = emptySet())
        rule.show(st)
        rule.onAllNodes(descStartsWith("QR code")).assertCountEquals(0)
        rule.onNodeWithText("Shown as: nothing on this phone. Check on the terminal that it took the share.").assertExists()
    }

    @Test fun released_timer_ends_with_share_expired() {
        rule.show(released.copy(now = ReleaseSamples.releaseDbs.expiresAt))
        rule.onNodeWithContentDescription("Share expired").assertExists()
    }

    @Test fun declined_state() {
        rule.show(dbs.copy(phase = ApprovalPhase.Declined))
        rule.onNodeWithText("Declined.").assertExists()
        rule.onNodeWithText("Nothing was released.").assertExists()
        rule.onAllNodesWithText("Approve").assertCountEquals(0)
    }

    @Test fun an_error_is_shown() {
        rule.show(dbs.copy(error = "The typed id does not match. Nothing was signed."))
        rule.onNodeWithText("The typed id does not match. Nothing was signed.").assertExists()
    }

    @Test fun dark_theme_and_targets() {
        rule.show(dbs, dark = true)
        rule.assertTargets48()
    }

    @Test fun light_theme_never_met_and_targets() {
        rule.show(bastion, dark = false)
        rule.onNodeWithText("This requester has never met this phone.").assertExists()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_visual_overflow() {
        val s = rule.show(dbs, fontScale = 2f)
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
        rule.runOnIdle { s.value = bastion }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
        rule.runOnIdle { s.value = released }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_1_3() {
        rule.show(bastion, fontScale = 1.3f)
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun d18_flag_secure_is_set_while_the_screen_is_shown() {
        rule.show(dbs)
        rule.waitForIdle()
        var flags = 0
        rule.activityRule.scenario.onActivity { flags = it.window.attributes.flags }
        assertTrue("FLAG_SECURE", flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
    }
}
