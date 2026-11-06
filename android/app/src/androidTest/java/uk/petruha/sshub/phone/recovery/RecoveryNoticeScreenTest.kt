package uk.petruha.sshub.phone.recovery

import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.fake.RecoverySamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.components.spokenHex
import uk.petruha.sshub.phone.ui.screens.recovery.NoticeActions
import uk.petruha.sshub.phone.ui.screens.recovery.NoticeState
import uk.petruha.sshub.phone.ui.screens.recovery.RecoveryNoticeScreen

class RecoveryNoticeScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val review = RecoverySamples.noticeReview
    private val cancelLabel = "Cancel this recovery: I still have this phone"

    private fun show(state: NoticeState, on: NoticeActions = NoticeActions(), dark: Boolean = true, fontScale: Float = 1f) =
        rule.setScreen(dark, fontScale) { RecoveryNoticeScreen(state, on) }

    @Test fun review_starts_with_the_red_banner_and_shows_every_field() {
        show(review)
        rule.onNodeWithText("RECOVERY YOU DID NOT START?").assertExists()
        rule.onNodeWithText("Someone is replacing pixel-8 with another phone.").assertExists()
        rule.onNodeWithContentDescription("Field Started by, new value workstation [9C61-E2A8]").assertExists()
        rule.onNodeWithContentDescription("Field Lamport, new value 1038").assertExists()
        rule.onNodeWithContentDescription("Field Operation, new value phone recovery: a new phone replaces pixel-8").assertExists()
        rule.onNodeWithContentDescription("Field Started, new value 21 h ago, 2026-10-25 12:41Z").assertExists()
        rule.onNodeWithText("The signature verifies", substring = true).assertExists()
    }

    @Test fun an_empty_phone_name_leaves_the_replacing_line_out() {
        show(review.copy(view = review.view!!.copy(phoneName = "")))
        rule.onNodeWithText("Someone is replacing", substring = true).assertDoesNotExist()
    }

    @Test fun d7_hash_prefix_is_three_groups_and_a_tap_shows_all_sixteen() {
        show(review)
        val hash = onHash()
        assertEquals(listOf("Notice hash ${spokenHex(listOf("a3f9", "007c", "4e1b"))}"), hash.fetchSemanticsNode().config[SemanticsProperties.ContentDescription])
        hash.performClick()
        val full = onHash().fetchSemanticsNode().config[SemanticsProperties.ContentDescription].single()
        assertEquals(16, full.removePrefix("Notice hash ").split(", ").size)
        rule.onNodeWithText("All 16 groups. Tap to show the prefix again.").assertExists()
    }

    private fun onHash() = rule.onNode(hasContentDescription("Notice hash", substring = true))

    @Test fun timelock_shows_time_left_end_and_a_described_tick_scale() {
        show(review)
        rule.onNodeWithText("6 d 3 h").assertExists()
        rule.onNodeWithText("timelock ends 2026-11-01 12:41Z").assertExists()
        rule.onNodeWithContentDescription("Timelock of 168 hours, 6 d 3 h left, ends 2026-11-01 12:41Z").assertExists()
        rule.onNodeWithText("7 day timelock").assertExists()
    }

    @Test fun cancel_action_only_opens_the_confirmation_and_never_signs() {
        var asked = 0
        var signed = 0
        var left = 0
        show(review, NoticeActions(onAskCancel = { asked++ }, onSign = { signed++ }, onLeave = { left++ }))
        rule.button(cancelLabel).performClick()
        assertEquals(1, asked)
        assertEquals(0, signed)
        rule.button("Leave it (I started this)").performClick()
        assertEquals(1, left)
    }

    @Test fun nothing_on_the_review_screen_approves() {
        show(review)
        rule.onNodeWithText("Approve", substring = true).assertDoesNotExist()
        rule.onNodeWithText("Continue", substring = true).assertDoesNotExist()
    }

    @Test fun confirm_signs_only_on_the_explicit_button() {
        var signed = 0
        var notYet = 0
        show(RecoverySamples.noticeConfirm, NoticeActions(onSign = { signed++ }, onNotYet = { notYet++ }))
        rule.onNodeWithText("Cancel this recovery?").assertExists()
        rule.onNodeWithText("This phone signs a cancellation. Do it only if you did not start this recovery. If you did, leave it.", substring = true).assertExists()
        rule.button(cancelLabel).assertDoesNotExist()
        rule.button("Not yet").performClick()
        assertEquals(1, notYet)
        assertEquals(0, signed)
        rule.button("Sign the cancellation").performClick()
        assertEquals(1, signed)
    }

    @Test fun d11_no_timer_signs() {
        var signed = 0
        show(RecoverySamples.noticeConfirm, NoticeActions(onSign = { signed++ }))
        rule.mainClock.advanceTimeBy(15 * 60 * 1000L)
        rule.waitForIdle()
        assertEquals(0, signed)
    }

    @Test fun confirm_while_busy_cannot_be_pressed_twice() {
        show(RecoverySamples.noticeConfirm.copy(busy = true))
        rule.button("Sign the cancellation").assertIsNotEnabled()
    }

    @Test fun signed_shows_green_banner_qr_text_and_copy() {
        var copied = 0
        var done = 0
        show(RecoverySamples.noticeSigned, NoticeActions(onCopy = { copied++ }, onDone = { done++ }))
        rule.onNodeWithText("CANCELLATION SIGNED").assertExists()
        rule.onNodeWithText("Recovery cancelled").assertExists()
        rule.onNodeWithText("workstation [9C61-E2A8] recovery, signed with this phone").assertExists()
        rule.onNode(hasContentDescription("QR code with the signed recovery cancellation", substring = true)).assertExists()
        rule.onNodeWithContentDescription("Signed text", substring = true).assertExists()
        rule.onNodeWithText("Copying marks the text as sensitive. It leaves the clipboard after 60 s.").assertExists()
        rule.button("Copy the text").performClick()
        rule.button("Done").performClick()
        assertEquals(1, copied)
        assertEquals(1, done)
        rule.onNodeWithText("Approve", substring = true).assertDoesNotExist()
    }

    @Test fun signed_after_copy_says_copied_and_when_it_clears() {
        show(RecoverySamples.noticeSigned.copy(copied = true, clearsIn = 60))
        rule.button("Copied").assertExists()
        rule.onNodeWithText("paste it into the terminal").assertExists()
        rule.onNodeWithText("Copied, clears in 60 s").assertExists()
    }

    @Test fun a_notice_that_cannot_be_cancelled_says_why_and_the_button_is_off() {
        show(RecoverySamples.noticeClosed)
        rule.button(cancelLabel).assertDisabled("This recovery is already cancelled.")
        rule.onAllNodesWithText("This recovery is already cancelled.", substring = true).assertCountEquals(1)
    }

    @Test fun a_core_error_is_shown() {
        show(review.copy(error = "The notice signature does not verify"))
        rule.onNodeWithText("The notice signature does not verify").assertExists()
    }

    @Test fun an_unknown_notice_offers_the_way_back() {
        var left = 0
        show(NoticeState(loaded = true), NoticeActions(onLeave = { left++ }))
        rule.onNodeWithText("This notice is gone").assertExists()
        rule.button("Back to recovery").performClick()
        assertEquals(1, left)
    }

    @Test fun d18_window_is_secure_in_every_phase() {
        show(review)
        rule.assertSecure(rule.activity)
    }

    @Test fun d18_window_is_secure_when_signed() {
        show(RecoverySamples.noticeSigned)
        rule.assertSecure(rule.activity)
    }

    @Test fun dark_theme_review() {
        show(review, dark = true)
        rule.onNodeWithText("RECOVERY YOU DID NOT START?").assertExists()
        rule.assertTargets48()
    }

    @Test fun light_theme_review() {
        show(review, dark = false)
        rule.assertTargets48()
    }

    @Test fun dark_and_light_signed() {
        show(RecoverySamples.noticeSigned, dark = false)
        rule.onNodeWithText("CANCELLATION SIGNED").assertExists()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_review_has_no_overflow() {
        show(review, fontScale = 2f)
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_confirm_has_no_overflow() {
        show(RecoverySamples.noticeConfirm, fontScale = 2f)
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_signed_has_no_overflow() {
        show(RecoverySamples.noticeSigned, fontScale = 2f)
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }
}
