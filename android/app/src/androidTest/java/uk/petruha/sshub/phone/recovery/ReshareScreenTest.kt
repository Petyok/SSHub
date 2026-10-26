package uk.petruha.sshub.phone.recovery

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertDoesNotExist
import androidx.compose.ui.test.assertExists
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.core.ReshareStage
import uk.petruha.sshub.phone.fake.RecoverySamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.recovery.ReshareActions
import uk.petruha.sshub.phone.ui.screens.recovery.ReshareScreen
import uk.petruha.sshub.phone.ui.screens.recovery.ReshareState

class ReshareScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun show(state: ReshareState, on: ReshareActions = ReshareActions(), dark: Boolean = true, fontScale: Float = 1f) =
        rule.setScreen(dark, fontScale) { ReshareScreen(state, on) }

    private fun sample(stage: ReshareStage) = RecoverySamples.reshareState(stage)

    @Test fun start_shows_facts_and_rotate_is_the_current_step() {
        var rotated = 0
        show(sample(ReshareStage.Start), ReshareActions(onRotate = { rotated++ }))
        rule.onNodeWithText("wrap key gen").assertExists()
        rule.onNodeWithText("#3", substring = false).assertExists()
        rule.onNodeWithText("0 of 9").assertExists()
        rule.onNodeWithText("0 of 3 done").assertExists()
        rule.button("Rotate wrap keys").assertIsEnabled().performScrollTo().performClick()
        assertEquals(1, rotated)
    }

    @Test fun start_adopt_and_publish_wait_and_say_why() {
        show(sample(ReshareStage.Start))
        rule.button("Adopt a reshare").assertDisabled("rotate wrap keys first")
        rule.button("Publish the new shares").assertDisabled("adopt a reshare first")
    }

    @Test fun rotated_shows_done_text_and_adopt_opens_the_scanner() {
        var scans = 0
        show(sample(ReshareStage.Rotated), ReshareActions(onScan = { scans++ }))
        rule.onNodeWithText("Done: generation 4. Old generations stay readable: they derive from the same 24 words.").assertExists()
        rule.onNodeWithText("1 of 3 done").assertExists()
        rule.onNodeWithText("Rotate wrap keys", substring = false).assertDoesNotExist()
        rule.button("Adopt a reshare").assertIsEnabled().performScrollTo().performClick()
        assertEquals(1, scans)
        rule.button("Publish the new shares").assertDisabled("adopt a reshare first")
    }

    @Test fun candidate_lists_contributors_and_is_adopted_by_one_tap() {
        var adopted = 0
        show(sample(ReshareStage.Rotated).copy(candidate = RecoverySamples.candidate), ReshareActions(onAdoptCandidate = { adopted++ }))
        rule.onNodeWithText("Reshare for epoch 8").assertExists()
        rule.onNodeWithText("laptop-main [7B3E-0D15]: signature verified").assertExists()
        rule.onNodeWithText("not met [E7A4-6B02]: signature not checked").assertExists()
        rule.button("Adopt this reshare").assertIsEnabled().performScrollTo().performClick()
        assertEquals(1, adopted)
    }

    @Test fun candidate_the_core_refuses_cannot_be_adopted() {
        var adopted = 0
        show(sample(ReshareStage.Rotated).copy(candidate = RecoverySamples.refusedCandidate), ReshareActions(onAdoptCandidate = { adopted++ }))
        rule.button("Adopt this reshare").assertDisabled(RecoverySamples.refusedCandidate.note)
        assertEquals(0, adopted)
    }

    @Test fun candidate_before_rotation_waits_for_the_rotation() {
        show(sample(ReshareStage.Start).copy(candidate = RecoverySamples.candidate))
        rule.button("Adopt this reshare").assertDisabled("rotate wrap keys first")
    }

    @Test fun adopted_shows_contributors_with_signature_not_checked_and_publish_is_current() {
        var published = 0
        show(sample(ReshareStage.Adopted), ReshareActions(onPublish = { published++ }))
        rule.onNodeWithText("Done: reshare for epoch 8 loaded, 9 credentials ready.").assertExists()
        rule.onNodeWithText("Contributors").assertExists()
        rule.onNodeWithText("3 counted").assertExists()
        rule.onNodeWithText("workstation [9C61-E2A8]: signature verified").assertExists()
        rule.onNodeWithText("not met [E7A4-6B02]: signature not checked").assertExists()
        rule.onNodeWithText("9 of 9").assertExists()
        rule.button("Publish the new shares").assertIsEnabled().performScrollTo().performClick()
        assertEquals(1, published)
    }

    @Test fun published_bar_exports_or_finishes() {
        var exported = 0
        var done = 0
        show(sample(ReshareStage.Published), ReshareActions(onExport = { exported++ }, onDone = { done++ }))
        rule.onNodeWithText("3 of 3 done").assertExists()
        rule.onNodeWithText("Done: the new shares, sealed, are in this phone chain for a terminal to take.").assertExists()
        rule.onNodeWithText("Publish the new shares", substring = false).assertDoesNotExist()
        rule.button("Export my ops").performClick()
        rule.button("Done").performClick()
        assertEquals(1, exported)
        assertEquals(1, done)
    }

    @Test fun empty_says_no_reshare_is_waiting_and_has_no_steps() {
        show(sample(ReshareStage.Empty))
        rule.onNodeWithText("No reshare is waiting.").assertExists()
        rule.onNodeWithText("9 of 9").assertExists()
        rule.onNodeWithText("Steps").assertDoesNotExist()
        rule.onNodeWithText("Rotate wrap keys", substring = false).assertDoesNotExist()
    }

    @Test fun consequence_warning_is_on_every_variant() {
        var stage by mutableStateOf(ReshareStage.Start)
        rule.setScreen { ReshareScreen(sample(stage), ReshareActions()) }
        for (s in ReshareStage.entries) {
            rule.runOnIdle { stage = s }
            rule.onNodeWithText("A ring of two terminals plus this phone cannot revoke a terminal by resharing.", substring = true).assertExists()
            rule.onNodeWithText("This phone only receives a new share: it never contributes a sub-share and keeps none of the old ones.").assertExists()
        }
    }

    @Test fun a_core_error_is_shown_as_text() {
        show(sample(ReshareStage.Start).copy(error = "The reshare is for another ring"))
        rule.onNodeWithText("The reshare is for another ring").assertExists()
    }

    @Test fun while_busy_the_signing_buttons_are_off() {
        show(sample(ReshareStage.Start).copy(busy = true))
        rule.button("Rotate wrap keys").assertIsNotEnabled()
    }

    @Test fun d18_window_is_secure() {
        show(sample(ReshareStage.Adopted))
        rule.assertSecure(rule.activity)
    }

    @Test fun dark_theme() {
        show(sample(ReshareStage.Rotated), dark = true)
        rule.onNodeWithText("Reshare and wrap keys").assertExists()
        rule.assertTargets48()
    }

    @Test fun light_theme() {
        show(sample(ReshareStage.Rotated), dark = false)
        rule.onNodeWithText("Reshare and wrap keys").assertExists()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow_and_keeps_targets() {
        show(sample(ReshareStage.Rotated).copy(candidate = RecoverySamples.candidate), fontScale = 2f)
        rule.onNodeWithText("Reshare and wrap keys").assertExists()
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_adopted_has_no_overflow() {
        show(sample(ReshareStage.Adopted), fontScale = 2f)
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_1_3_stacks_the_facts() {
        show(sample(ReshareStage.Start), fontScale = 1.3f)
        rule.onNodeWithText("holder no.").assertExists()
        rule.assertNoVisualOverflow()
    }
}
