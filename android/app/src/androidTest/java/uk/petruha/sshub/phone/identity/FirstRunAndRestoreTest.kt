package uk.petruha.sshub.phone.identity

import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasSetTextAction
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
import uk.petruha.sshub.phone.fake.IdentitySamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.identity.FirstRunActions
import uk.petruha.sshub.phone.ui.screens.identity.FirstRunScreen
import uk.petruha.sshub.phone.ui.screens.identity.RestoreActions
import uk.petruha.sshub.phone.ui.screens.identity.RestorePhase
import uk.petruha.sshub.phone.ui.screens.identity.RestoreScreen

class FirstRunScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun main_path_create_and_restore_call_back() {
        var created = 0; var restored = 0
        rule.setScreen { FirstRunScreen(IdentitySamples.firstRun, FirstRunActions(onCreate = { created++ }, onRestore = { restored++ })) }
        rule.onNodeWithText("Create a new key").performClick()
        rule.onNodeWithText("Restore from 24 words").performClick()
        assertEquals(1, created); assertEquals(1, restored)
        rule.assertTargets48()
    }

    @Test fun there_are_no_pin_fields() {
        rule.setScreen { FirstRunScreen(IdentitySamples.firstRun, FirstRunActions()) }
        rule.onAllNodes(hasSetTextAction()).assertCountEquals(0)
        rule.onNodeWithText("no PIN in the app").assertExists()
    }

    @Test fun no_secure_lock_explains_and_opens_system_settings() {
        var opened = 0
        rule.setScreen { FirstRunScreen(IdentitySamples.firstRunNoLock, FirstRunActions(onOpenSecuritySettings = { opened++ })) }
        rule.onNodeWithText("No screen lock on this phone").assertExists()
        rule.onNodeWithText("Open security settings").performScrollTo().performClick()
        assertEquals(1, opened)
        rule.assertTargets48()
    }

    @Test fun ready_state_has_no_security_settings_button() {
        rule.setScreen { FirstRunScreen(IdentitySamples.firstRun, FirstRunActions()) }
        rule.onAllNodesWithText("Open security settings").assertCountEquals(0)
    }

    @Test fun creating_disables_both_buttons() {
        rule.setScreen { FirstRunScreen(IdentitySamples.firstRunCreating, FirstRunActions()) }
        rule.onNodeWithText("Creating the key...").assertIsNotEnabled()
        rule.onNodeWithText("Restore from 24 words").assertIsNotEnabled()
    }

    @Test fun dark_and_light() {
        rule.setScreen(dark = true) { FirstRunScreen(IdentitySamples.firstRun, FirstRunActions()) }
        rule.onNodeWithText("Create a new key").assertExists()
    }

    @Test fun light_theme_renders() {
        rule.setScreen(dark = false) { FirstRunScreen(IdentitySamples.firstRunNoLock, FirstRunActions()) }
        rule.onNodeWithText("Create a new key").assertExists()
    }

    @Test fun font_scale_2_has_no_visual_overflow() {
        rule.setScreen(fontScale = 2f) { FirstRunScreen(IdentitySamples.firstRunNoLock, FirstRunActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }
}

class RestoreScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun main_path_full_and_cleared_restores() {
        var restored = 0
        rule.setScreen { RestoreScreen(IdentitySamples.restoreFull, RestoreActions(onRestore = { restored++ })) }
        rule.onNodeWithText("24 of 24 words accepted").assertExists()
        rule.onNodeWithText("Restore this phone").assertIsEnabled().performClick()
        assertEquals(1, restored)
        rule.assertTargets48()
    }

    @Test fun partial_shows_the_count_and_blocks() {
        rule.setScreen { RestoreScreen(IdentitySamples.restorePartial, RestoreActions()) }
        rule.onNodeWithText("9 of 24 words accepted").assertExists()
        rule.onNodeWithText("Restore this phone").assertIsNotEnabled()
    }

    @Test fun bad_words_are_named_with_words_and_block() {
        var restored = 0
        rule.setScreen { RestoreScreen(IdentitySamples.restoreBad, RestoreActions(onRestore = { restored++ })) }
        rule.onNodeWithText("2 not in the word list: fix the marked ones.").assertExists()
        rule.onNodeWithContentDescription("Word 8").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "not in the word list"))
        rule.onNodeWithContentDescription("Word 1").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "accepted"))
        rule.onNodeWithText("Restore this phone").assertIsNotEnabled()
        assertEquals(0, restored)
    }

    @Test fun gate_all_words_but_site_data_not_cleared_blocks_and_the_box_calls_back() {
        var cleared: Boolean? = null
        rule.setScreen { RestoreScreen(IdentitySamples.restoreFullNotCleared, RestoreActions(onSiteDataCleared = { cleared = it })) }
        rule.onNodeWithText("Restore this phone").assertIsNotEnabled()
        rule.onNodeWithText("I cleared the site data of the companion page").performScrollTo().performClick()
        assertEquals(true, cleared)
    }

    @Test fun m4_the_question_is_asked_and_unanswered_blocks_with_its_reason() {
        rule.setScreen { RestoreScreen(IdentitySamples.restoreFullUnanswered, RestoreActions()) }
        rule.onNodeWithText("Did you use the companion page in a browser on this phone before?").performScrollTo().assertExists()
        rule.onAllNodesWithText("I cleared the site data of the companion page").assertCountEquals(0)
        rule.onNodeWithText("Restore this phone").assertIsNotEnabled()
    }

    @Test fun m4_a_user_who_never_used_the_page_sees_no_box_and_can_restore() {
        var restored = 0
        rule.setScreen { RestoreScreen(IdentitySamples.restoreFullNoPage, RestoreActions(onRestore = { restored++ })) }
        rule.onAllNodesWithText("I cleared the site data of the companion page").assertCountEquals(0)
        rule.onNodeWithText("Restore this phone").assertIsEnabled().performClick()
        assertEquals(1, restored)
    }

    @Test fun m4_yes_shows_the_steps_and_the_required_box() {
        var answer: Boolean? = null
        rule.setScreen { RestoreScreen(IdentitySamples.restoreFullNotCleared, RestoreActions(onUsedPage = { answer = it })) }
        rule.onNodeWithText("Open the companion page menu", substring = true).performScrollTo().assertExists()
        rule.onNodeWithText("I cleared the site data of the companion page").performScrollTo().assertExists()
        rule.onNodeWithText("No, never").performScrollTo().performClick()
        assertEquals(false, answer)
    }

    @Test fun the_warning_that_a_restored_phone_signs_nothing_is_shown() {
        rule.setScreen { RestoreScreen(IdentitySamples.restoreEmpty, RestoreActions()) }
        rule.onNodeWithText("A restored phone signs nothing yet.").assertExists()
    }

    @Test fun over_variant_warns_and_relabels() {
        rule.setScreen { RestoreScreen(IdentitySamples.restoreOver, RestoreActions()) }
        rule.onNodeWithText("Replaces the key stored on this phone").assertExists()
        rule.onNodeWithText("Replace and restore").assertExists()
    }

    @Test fun typing_a_word_and_pasting_call_back() {
        var typed: Pair<Int, String>? = null; var pasted = ""; var filled = 0
        rule.setScreen {
            RestoreScreen(
                IdentitySamples.restoreEmpty.copy(paste = ""),
                RestoreActions(onWord = { i, t -> typed = i to t }, onPaste = { pasted = it }, onFill = { filled++ }),
            )
        }
        rule.onNodeWithContentDescription("Word 3").performTextInput("sanitizer")
        assertEquals(2 to "sanitizer", typed)
        rule.onNodeWithText("Fill the grid").assertIsNotEnabled()
        rule.onNodeWithContentDescription("Pasted words").performTextInput("edging evolution")
        assertEquals("edging evolution", pasted)
    }

    @Test fun fill_the_grid_is_enabled_with_pasted_text() {
        var filled = 0
        rule.setScreen { RestoreScreen(IdentitySamples.restoreEmpty.copy(paste = "edging evolution"), RestoreActions(onFill = { filled++ })) }
        rule.onNodeWithText("Fill the grid").assertIsEnabled().performClick()
        assertEquals(1, filled)
    }

    @Test fun no_secure_lock_phase_offers_settings() {
        var opened = 0
        rule.setScreen { RestoreScreen(IdentitySamples.restoreFull.copy(phase = RestorePhase.NoSecureLock), RestoreActions(onOpenSecuritySettings = { opened++ })) }
        rule.onNodeWithText("Open security settings").performScrollTo().performClick()
        assertEquals(1, opened)
    }

    @Test fun failed_phase_shows_the_reason() {
        rule.setScreen { RestoreScreen(IdentitySamples.restoreFull.copy(phase = RestorePhase.Failed, error = "These 24 words are not a valid recovery phrase."), RestoreActions()) }
        rule.onNodeWithText("The phone was not restored").assertExists()
    }

    @Test fun d18_the_window_is_secure_while_words_can_be_typed() {
        rule.setScreen { RestoreScreen(IdentitySamples.restoreEmpty, RestoreActions()) }
        rule.waitForIdle()
        var flags = 0
        rule.runOnUiThread { flags = rule.activity.window.attributes.flags }
        assertTrue("FLAG_SECURE", flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
    }

    @Test fun light_theme_renders() {
        rule.setScreen(dark = false) { RestoreScreen(IdentitySamples.restoreBad, RestoreActions()) }
        rule.onNodeWithText("Restore this phone").assertExists()
    }

    @Test fun font_scale_2_has_no_visual_overflow() {
        rule.setScreen(fontScale = 2f) { RestoreScreen(IdentitySamples.restoreBad, RestoreActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_1_3_targets() {
        rule.setScreen(fontScale = 1.3f) { RestoreScreen(IdentitySamples.restoreOver, RestoreActions()) }
        rule.assertTargets48()
    }
}
