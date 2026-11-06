package uk.petruha.sshub.phone.identity

import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.fake.DemoWords
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.fake.IdentitySamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.identity.BackupActions
import uk.petruha.sshub.phone.ui.screens.identity.BackupWordsScreen
import uk.petruha.sshub.phone.ui.screens.identity.LockActions
import uk.petruha.sshub.phone.ui.screens.identity.LockPhase
import uk.petruha.sshub.phone.ui.screens.identity.LockReason
import uk.petruha.sshub.phone.ui.screens.identity.LockScreen
import uk.petruha.sshub.phone.ui.screens.identity.LockState
import uk.petruha.sshub.phone.ui.screens.identity.ReplaceKeyActions
import uk.petruha.sshub.phone.ui.screens.identity.ReplaceKeyScreen
import uk.petruha.sshub.phone.ui.screens.identity.ReplaceKeyState

private const val COVER = "Words hidden. Hold to reveal."

class BackupWordsScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun main_path_hold_reveals_words_and_release_covers_them_again() {
        rule.setScreen { BackupWordsScreen(IdentitySamples.backupReady, BackupActions()) }
        rule.onNodeWithContentDescription(COVER).assertExists()
        rule.onNodeWithContentDescription(COVER).performTouchInput { down(center) }
        rule.onNodeWithContentDescription("Word 1, edging").assertExists()
        rule.onNodeWithContentDescription("Word 24, snare").assertExists()
        rule.onRoot().performTouchInput { up() }
        rule.onNodeWithContentDescription(COVER).assertExists()
    }

    @Test fun paper_checkbox_then_done() {
        var done = 0
        rule.setScreen {
            var paper by remember { mutableStateOf(false) }
            BackupWordsScreen(IdentitySamples.backupReady.copy(paperChecked = paper), BackupActions(onPaperChecked = { paper = it }, onDone = { done++ }))
        }
        rule.onNodeWithText("Done").assertIsNotEnabled()
        rule.onNodeWithText("I wrote them on paper").performClick()
        rule.onNodeWithText("Done").assertIsEnabled().performClick()
        assertEquals(1, done)
        rule.assertTargets48()
    }

    @Test fun done_stays_disabled_while_the_words_are_not_shown() {
        rule.setScreen { BackupWordsScreen(IdentitySamples.backupAuthenticating.copy(paperChecked = true), BackupActions()) }
        rule.onNodeWithText("Done").assertIsNotEnabled()
        rule.onNodeWithText("waiting for the system prompt").assertExists()
        rule.onAllNodesWithText(COVER).assertCountEquals(0)
    }

    @Test fun canceled_offers_the_prompt_again_and_shows_no_words() {
        var again = 0
        rule.setScreen { BackupWordsScreen(IdentitySamples.backupCanceled, BackupActions(onAuthenticate = { again++ })) }
        rule.onNodeWithText("Show the system prompt").performClick()
        assertEquals(1, again)
        rule.onAllNodes(hasContentDescription("Word 1", substring = true)).assertCountEquals(0)
    }

    @Test fun first_run_has_no_back_arrow_but_the_menu_origin_has() {
        rule.setScreen { BackupWordsScreen(IdentitySamples.backupReady, BackupActions()) }
        rule.onAllNodes(hasContentDescription("Back")).assertCountEquals(0)
    }

    @Test fun menu_origin_back_arrow_calls_back() {
        var back = 0
        rule.setScreen { BackupWordsScreen(IdentitySamples.backupFromMenu, BackupActions(onBack = { back++ })) }
        rule.onNodeWithContentDescription("Back").performClick()
        assertEquals(1, back)
    }

    @Test fun d16_no_copy_affordance_and_nothing_reaches_the_clipboard() {
        val platform = FakePlatform()
        rule.setScreen(platform = platform) { BackupWordsScreen(IdentitySamples.backupReady, BackupActions()) }
        rule.onNodeWithContentDescription(COVER).performTouchInput { down(center) }
        rule.onAllNodes(hasClickAction() and hasText("copy", substring = true, ignoreCase = true)).assertCountEquals(0)
        rule.onAllNodes(hasClickAction() and hasContentDescription("copy", substring = true, ignoreCase = true)).assertCountEquals(0)
        rule.onRoot().performTouchInput { up() }
        assertNull(platform.clipboard.text)
        assertFalse(platform.clipboard.sensitive)
    }

    @Test fun d18_flag_secure_is_set_while_shown_and_cleared_on_leave() {
        var shown by mutableStateOf(true)
        rule.setScreen { if (shown) BackupWordsScreen(IdentitySamples.backupReady, BackupActions()) }
        rule.waitForIdle()
        var flags = 0
        rule.runOnUiThread { flags = rule.activity.window.attributes.flags }
        assertTrue("FLAG_SECURE set", flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
        shown = false
        rule.waitForIdle()
        rule.runOnUiThread { flags = rule.activity.window.attributes.flags }
        assertTrue("FLAG_SECURE cleared", flags and WindowManager.LayoutParams.FLAG_SECURE == 0)
    }

    @Test fun all_24_words_are_the_demo_words() {
        assertEquals(24, DemoWords.backup.size)
        assertEquals(24, IdentitySamples.backupReady.words.size)
    }

    @Test fun light_theme_renders() {
        rule.setScreen(dark = false) { BackupWordsScreen(IdentitySamples.backupReady, BackupActions()) }
        rule.onNodeWithContentDescription(COVER).assertExists()
    }

    @Test fun font_scale_2_has_no_visual_overflow_covered_and_revealed() {
        rule.setScreen(fontScale = 2f) { BackupWordsScreen(IdentitySamples.backupReady, BackupActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
        rule.onNodeWithContentDescription(COVER).performTouchInput { down(center) }
        rule.assertNoVisualOverflow()
        rule.onRoot().performTouchInput { up() }
    }
}

class LockScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun main_path_waiting_unlock_and_restore_call_back() {
        var unlock = 0; var restore = 0
        rule.setScreen { LockScreen(IdentitySamples.lockWaiting, LockActions(onUnlock = { unlock++ }, onRestore = { restore++ })) }
        rule.onNodeWithText("waiting for the system prompt").assertExists()
        rule.onNodeWithText("Unlock").performClick()
        rule.onNodeWithText("Restore from 24 words instead").performClick()
        assertEquals(1, unlock); assertEquals(1, restore)
        rule.assertTargets48()
    }

    @Test fun screen_off_reason_text() {
        rule.setScreen { LockScreen(LockState(reason = LockReason.ScreenOff), LockActions()) }
        rule.onNodeWithText("Locked because the screen turned off. Every key was wiped from memory.").assertExists()
    }

    @Test fun canceled_says_so_and_unlock_stays_available() {
        rule.setScreen { LockScreen(IdentitySamples.lockCanceled, LockActions()) }
        rule.onNodeWithText("Unlock was canceled. The app stays locked.").assertExists()
        rule.onNodeWithText("Unlock").assertIsEnabled()
    }

    @Test fun locked_out_names_the_words_and_offers_a_retry() {
        var unlock = 0
        rule.setScreen { LockScreen(IdentitySamples.lockLockedOut, LockActions(onUnlock = { unlock++ })) }
        rule.onNodeWithText("Android blocked the unlock for now").assertExists()
        rule.onNodeWithText("Try again").performClick()
        assertEquals(1, unlock)
        rule.onNodeWithText("Restore from 24 words instead").assertExists()
    }

    @Test fun invalidated_offers_only_the_restore_from_words() {
        var restore = 0
        rule.setScreen { LockScreen(IdentitySamples.lockInvalidated, LockActions(onRestore = { restore++ })) }
        rule.onNodeWithText("The key is gone").assertExists()
        rule.onAllNodesWithText("Unlock").assertCountEquals(0)
        rule.onAllNodesWithText("Try again").assertCountEquals(0)
        rule.onNodeWithText("Restore from 24 words").performClick()
        assertEquals(1, restore)
    }

    @Test fun failed_shows_the_message() {
        rule.setScreen { LockScreen(LockState(LockPhase.Failed, message = "The core refused the key: no"), LockActions()) }
        rule.onNodeWithText("The key could not be opened").assertExists()
    }

    @Test fun light_theme_and_font_scale_2() {
        rule.setScreen(dark = false, fontScale = 2f) { LockScreen(IdentitySamples.lockInvalidated, LockActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_locked_out() {
        rule.setScreen(fontScale = 2f) { LockScreen(IdentitySamples.lockLockedOut, LockActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }
}

class ReplaceKeyScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun main_path_type_erase_then_erase() {
        var erased = 0
        rule.setScreen {
            var typed by remember { mutableStateOf("") }
            ReplaceKeyScreen(ReplaceKeyState(typed), ReplaceKeyActions(onTyped = { typed = it }, onErase = { erased++ }))
        }
        rule.onNodeWithText("Erase and continue").assertIsNotEnabled()
        rule.onNodeWithContentDescription("Type ERASE to confirm").performTextInput("ERASE")
        rule.onNodeWithText("Erase and continue").assertIsEnabled().performClick()
        assertEquals(1, erased)
        rule.assertTargets48()
    }

    @Test fun d13_erase_is_disabled_until_the_word_is_typed_exactly() {
        for (typed in listOf("", "ERAS", "erase", "ERASEX", "ERASE NOW")) {
            assertFalse("'$typed'", ReplaceKeyState(typed).canErase)
        }
        assertTrue(ReplaceKeyState("ERASE").canErase)
        assertTrue(ReplaceKeyState(" ERASE ").canErase)
    }

    @Test fun a_wrong_long_entry_shows_the_words_does_not_match() {
        rule.setScreen { ReplaceKeyScreen(ReplaceKeyState("erase"), ReplaceKeyActions()) }
        rule.onNodeWithText("does not match").assertExists()
        rule.onNodeWithText("Erase and continue").assertIsNotEnabled()
    }

    @Test fun lists_what_is_erased_as_old_and_new_fields() {
        rule.setScreen { ReplaceKeyScreen(IdentitySamples.replaceKey, ReplaceKeyActions()) }
        rule.onNodeWithContentDescription("Field Enrollments changes from every terminal this phone met to erased").assertExists()
        rule.onNodeWithContentDescription("Field Short-code list changes from the list kept for the old key to erased").assertExists()
        rule.onNodeWithText("Erased from this phone").assertExists()
    }

    @Test fun cancel_calls_back() {
        var cancel = 0
        rule.setScreen { ReplaceKeyScreen(IdentitySamples.replaceKey, ReplaceKeyActions(onCancel = { cancel++ })) }
        rule.onNodeWithText("Cancel").performClick()
        assertEquals(1, cancel)
    }

    @Test fun the_ready_sample_enables_erase_and_light_theme_renders() {
        rule.setScreen(dark = false) { ReplaceKeyScreen(IdentitySamples.replaceKeyReady, ReplaceKeyActions()) }
        rule.onNodeWithText("Erase and continue").assertIsEnabled()
    }

    @Test fun font_scale_2_has_no_visual_overflow() {
        rule.setScreen(fontScale = 2f) { ReplaceKeyScreen(IdentitySamples.replaceKeyReady, ReplaceKeyActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }
}
