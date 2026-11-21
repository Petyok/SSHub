package io.github.petyok.sshub.recovery

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import io.github.petyok.sshub.core.ReshareStage
import io.github.petyok.sshub.release.descStartsWith
import io.github.petyok.sshub.fake.RecoverySamples
import io.github.petyok.sshub.testing.assertNoVisualOverflow
import io.github.petyok.sshub.testing.assertTargets48
import io.github.petyok.sshub.testing.setScreen
import io.github.petyok.sshub.ui.screens.recovery.ReshareActions
import io.github.petyok.sshub.ui.screens.recovery.ReshareScreen
import io.github.petyok.sshub.ui.screens.recovery.ReshareState

class ReshareScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun show(state: ReshareState, on: ReshareActions = ReshareActions(), dark: Boolean = true, fontScale: Float = 1f) =
        rule.setScreen(dark, fontScale) { ReshareScreen(state, on) }

    private fun sample(stage: ReshareStage) = RecoverySamples.reshareState(stage)

    @Test fun start_shows_plain_facts_and_rotate_is_the_current_step() {
        var rotated = 0
        show(sample(ReshareStage.Start), ReshareActions(onRotate = { rotated++ }))
        rule.onNodeWithText("Passwords ready for this phone: 0 of 9").assertExists()
        rule.onNodeWithText("0 of 2 done").assertExists()
        rule.button("Renew the phone keys").assertIsEnabled().performScrollTo().performClick()
        assertEquals(1, rotated)
    }

    @Test fun u13_23_design_numbers_sit_in_a_collapsed_technical_details() {
        show(sample(ReshareStage.Start))
        rule.onNodeWithText("sharing round").assertDoesNotExist()
        rule.onNodeWithText("phone key generation").assertDoesNotExist()
        rule.button("Technical details").performClick()
        rule.onNodeWithText("phone key generation").assertExists()
        rule.onNodeWithText("sharing round").assertExists()
        rule.onNodeWithText("#3", substring = false).assertExists()
    }

    @Test fun m1_adopt_is_enabled_before_any_rotation_in_this_process() {
        show(sample(ReshareStage.Start))
        rule.button("Adopt a reshare").assertIsEnabled()
    }

    @Test fun there_is_no_separate_publish_step() {
        var stage by mutableStateOf(ReshareStage.Start)
        rule.setScreen { ReshareScreen(sample(stage), ReshareActions()) }
        for (s in ReshareStage.entries) {
            rule.runOnIdle { stage = s }
            rule.onNodeWithText("Publish the new shares", substring = false).assertDoesNotExist()
            rule.onNodeWithText("3 Publish", substring = true).assertDoesNotExist()
        }
    }

    @Test fun rotated_shows_done_text_and_adopt_opens_the_scanner() {
        var scans = 0
        show(sample(ReshareStage.Rotated), ReshareActions(onScan = { scans++ }))
        rule.onNodeWithText("Done: phone key generation 4. Older keys stay readable: they derive from the same 24 words.").assertExists()
        rule.onNodeWithText("1 of 2 done").assertExists()
        rule.onNodeWithText("Renew the phone keys", substring = false).assertDoesNotExist()
        rule.button("Adopt a reshare").assertIsEnabled().performScrollTo().performClick()
        assertEquals(1, scans)
    }

    @Test fun candidate_lists_contributors_and_one_tap_adopts_and_publishes() {
        var adopted = 0
        show(sample(ReshareStage.Candidate), ReshareActions(onAdoptPublish = { adopted++ }))
        rule.onNodeWithText("New shares from a terminal").assertExists()
        rule.onNodeWithText("laptop-main [7B3E-0D15]: signature verified").assertExists()
        rule.onNodeWithText("not met [E7A4-6B02]: signature not checked").assertExists()
        // H3: the flags are lines of their own, a missing session key is a failing one
        rule.onNodeWithText("laptop-main [7B3E-0D15]: known to this phone").assertExists()
        rule.onNodeWithText("laptop-main [7B3E-0D15]: session key for this phone").assertExists()
        rule.onNodeWithContentDescription("Failed: not met [E7A4-6B02]: not known to this phone").assertExists()
        rule.onNodeWithContentDescription("Failed: not met [E7A4-6B02]: no session key for this phone").assertExists()
        // the full devices afterwards, this phone marked, one node each
        rule.onNodeWithContentDescription("Full device afterwards: laptop-main [7B3E-0D15]").assertExists()
        rule.onNodeWithContentDescription("Full device afterwards: pixel-8 [4F2A-91C0] (this phone)").assertExists()
        rule.onAllNodes(descStartsWith("Full device afterwards: ")).assertCountEquals(RecoverySamples.holders.size)
        rule.onNodeWithText("A storage key of this sharing round is published with the new shares.").assertExists()
        rule.button("Adopt and publish the new shares").assertIsEnabled().performScrollTo().performClick()
        assertEquals(1, adopted)
    }

    @Test fun candidate_the_core_refuses_cannot_be_adopted() {
        var adopted = 0
        show(sample(ReshareStage.Candidate).copy(candidate = RecoverySamples.refusedCandidate), ReshareActions(onAdoptPublish = { adopted++ }))
        rule.button("Adopt and publish the new shares").assertDisabled(RecoverySamples.refusedCandidate.note)
        assertEquals(0, adopted)
    }

    @Test fun m1_a_candidate_can_be_adopted_without_a_rotation_in_this_process() {
        show(sample(ReshareStage.Start).copy(candidate = RecoverySamples.candidate))
        rule.button("Adopt and publish the new shares").assertIsEnabled()
    }

    @Test fun candidate_while_busy_cannot_be_adopted_twice() {
        show(sample(ReshareStage.Candidate).copy(busy = true))
        rule.button("Adopt and publish the new shares").assertIsNotEnabled()
    }

    @Test fun published_result_shows_contributors_qr_text_and_copy() {
        var copied = 0
        var done = 0
        show(
            sample(ReshareStage.Published).copy(outgoing = RecoverySamples.outgoing(2)),
            ReshareActions(onCopy = { copied++ }, onDone = { done++ }),
        )
        rule.onNodeWithText("2 of 2 done").assertExists()
        rule.onNodeWithText("Contributors").assertExists()
        rule.onNodeWithText("3 counted").assertExists()
        rule.onNodeWithText("not met [E7A4-6B02]: signature not checked").assertExists()
        rule.onNodeWithText("Passwords ready for this phone: 9 of 9").assertExists()
        rule.onNode(hasContentDescription("QR code with the signed new shares", substring = true)).performScrollTo().assertExists()
        rule.onNodeWithContentDescription("Signed text", substring = true).assertExists()
        rule.onNodeWithText("Copying marks the text as sensitive. It leaves the clipboard after 60 s.").assertExists()
        rule.button("Copy the text").performClick()
        rule.button("Done").performClick()
        assertEquals(1, copied)
        assertEquals(1, done)
    }

    @Test fun published_after_copy_says_when_the_clipboard_clears() {
        show(sample(ReshareStage.Published).copy(outgoing = RecoverySamples.outgoing(2), copied = true, clearsIn = 60))
        rule.button("Copied").assertExists()
        rule.onNodeWithText("Copied, clears in 60 s").assertExists()
    }

    @Test fun published_without_the_signed_ops_offers_export_or_done() {
        var exported = 0
        var done = 0
        show(sample(ReshareStage.Published), ReshareActions(onExport = { exported++ }, onDone = { done++ }))
        rule.onNodeWithText("Done: the new shares are adopted", substring = true).assertExists()
        rule.onNodeWithContentDescription("Signed text", substring = true).assertDoesNotExist()
        rule.button("Export my ops").performClick()
        rule.button("Done").performClick()
        assertEquals(1, exported)
        assertEquals(1, done)
    }

    @Test fun empty_says_no_reshare_is_waiting_and_has_no_steps() {
        show(sample(ReshareStage.Empty))
        rule.onNodeWithText("No reshare is waiting.").assertExists()
        rule.onNodeWithText("Passwords ready for this phone: 9 of 9").assertExists()
        rule.onNodeWithText("Steps").assertDoesNotExist()
        rule.onNodeWithText("Renew the phone keys", substring = false).assertDoesNotExist()
    }

    @Test fun consequence_warning_is_on_every_variant() {
        var stage by mutableStateOf(ReshareStage.Start)
        rule.setScreen { ReshareScreen(sample(stage), ReshareActions()) }
        for (s in ReshareStage.entries) {
            rule.runOnIdle { stage = s }
            rule.onNodeWithText("Two terminals plus this phone cannot remove a device by updating the shares.", substring = true).assertExists()
            rule.onNodeWithText("This phone only receives new shares: it never contributes any and keeps none of the old ones.").assertExists()
        }
    }

    @Test fun a_core_error_is_shown_as_text() {
        show(sample(ReshareStage.Start).copy(error = "The reshare is for another ring"))
        rule.onNodeWithText("The reshare is for another ring").assertExists()
    }

    @Test fun while_busy_the_signing_buttons_are_off() {
        show(sample(ReshareStage.Start).copy(busy = true))
        rule.button("Renew the phone keys").assertIsNotEnabled()
    }

    @Test fun d18_window_is_secure() {
        show(sample(ReshareStage.Published))
        rule.assertSecure(rule.activity)
    }

    @Test fun dark_theme() {
        show(sample(ReshareStage.Rotated), dark = true)
        rule.onNodeWithText("Update this phone after a device was removed").assertExists()
        rule.assertTargets48()
    }

    @Test fun light_theme() {
        show(sample(ReshareStage.Rotated), dark = false)
        rule.onNodeWithText("Update this phone after a device was removed").assertExists()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow_and_keeps_targets() {
        show(sample(ReshareStage.Candidate), fontScale = 2f)
        rule.onNodeWithText("Update this phone after a device was removed").assertExists()
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_published_has_no_overflow() {
        show(sample(ReshareStage.Published).copy(outgoing = RecoverySamples.outgoing(2), copied = true, clearsIn = 60), fontScale = 2f)
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_1_3_stacks_the_facts() {
        show(sample(ReshareStage.Start), fontScale = 1.3f)
        rule.button("Technical details").performScrollTo().performClick()
        rule.onNodeWithText("full device no.").assertExists()
        rule.assertNoVisualOverflow()
    }
}
