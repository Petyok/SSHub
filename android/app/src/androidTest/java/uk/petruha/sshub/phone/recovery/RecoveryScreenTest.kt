package uk.petruha.sshub.phone.recovery

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertExists
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.assertDoesNotExist
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.fake.RecoverySamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.recovery.RecoveryActions
import uk.petruha.sshub.phone.ui.screens.recovery.RecoveryScreen
import uk.petruha.sshub.phone.ui.screens.recovery.RecoveryState

class RecoveryScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun show(state: RecoveryState, on: RecoveryActions = RecoveryActions(), dark: Boolean = true, fontScale: Float = 1f) =
        rule.setScreen(dark, fontScale) { RecoveryScreen(state, on) }

    @Test fun explains_recovery_and_the_three_situations() {
        show(RecoverySamples.recoveryEmpty)
        rule.onNodeWithText("If someone starts replacing your phone", substring = true).assertExists()
        rule.onNodeWithText("I still have the 24 words").assertExists()
        rule.onNodeWithText("I lost the phone and the words").assertExists()
        rule.onNodeWithText("A recovery is running and I did not start it").assertExists()
        rule.onNodeWithText("can unlock it can still approve requests", substring = true).assertExists()
    }

    @Test fun without_a_notice_it_says_so() {
        show(RecoverySamples.recoveryEmpty)
        rule.onNodeWithText("No recovery notice is known to this phone").assertExists()
        rule.onNodeWithText("A recovery notice is waiting").assertDoesNotExist()
    }

    @Test fun a_waiting_notice_is_a_row_that_opens_it() {
        var opened = -1L
        show(RecoverySamples.recoveryWithNotice, RecoveryActions(onOpenNotice = { opened = it }))
        rule.onNodeWithText("workstation [9C61-E2A8] started a recovery, 6 d 3 h left").assertExists()
        rule.onNodeWithText("A recovery notice is waiting").performScrollTo().performClick()
        assertEquals(RecoverySamples.TOKEN, opened)
    }

    @Test fun a_notice_past_its_timelock_says_so() {
        show(RecoveryState(listOf(RecoverySamples.summary), RecoverySamples.endsAt + 5))
        rule.onNodeWithText("workstation [9C61-E2A8] started a recovery, timelock ended").assertExists()
    }

    @Test fun scan_button_opens_the_scanner() {
        var scans = 0
        show(RecoverySamples.recoveryEmpty, RecoveryActions(onScan = { scans++ }))
        rule.button("Scan a recovery notice").performClick()
        assertEquals(1, scans)
    }

    @Test fun back_arrow_goes_back() {
        var back = 0
        show(RecoverySamples.recoveryEmpty, RecoveryActions(onBack = { back++ }))
        rule.onNodeWithContentDescription("Back").performClick()
        assertEquals(1, back)
    }

    @Test fun dark_theme() {
        show(RecoverySamples.recoveryWithNotice, dark = true)
        rule.onNodeWithText("Recovery and a new phone").assertExists()
        rule.assertTargets48()
    }

    @Test fun light_theme() {
        show(RecoverySamples.recoveryWithNotice, dark = false)
        rule.onNodeWithText("Recovery and a new phone").assertExists()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow() {
        show(RecoverySamples.recoveryWithNotice, fontScale = 2f)
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }
}
