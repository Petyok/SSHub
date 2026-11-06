package uk.petruha.sshub.phone.identity

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uk.petruha.sshub.phone.app.AppRoot
import uk.petruha.sshub.phone.core.PhoneCoreApi
import uk.petruha.sshub.phone.fake.FakePhoneCore
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.ui.screens.identity.StateLostScreen

private class LostCore(private val inner: FakePhoneCore = FakePhoneCore()) : PhoneCoreApi by inner {
    var lost = true
    var confirmed = 0
    override fun stateLost() = lost
    override fun confirmStateLost() { confirmed++; lost = false }
}

/** Security M2: the stop screen and its place in the shell. */
@RunWith(AndroidJUnit4::class)
class StateLostScreenTest {
    @get:Rule val rule = createComposeRule()

    @Test fun the_screen_offers_both_ways_out_and_says_nothing_was_written() {
        var cont = 0; var lock = 0
        rule.setContent { uk.petruha.sshub.phone.ui.theme.SshubTheme(dark = true, reducedMotion = false) { StateLostScreen(onContinue = { cont++ }, onLock = { lock++ }) } }
        rule.onNodeWithText("Nothing has been overwritten yet", substring = true).assertExists()
        rule.onNodeWithText("Lock and keep the old file").performClick()
        rule.onNodeWithText("Continue with an empty state").performClick()
        assertEquals(1, lock); assertEquals(1, cont)
    }

    @Test fun the_shell_stops_on_the_screen_after_an_unlock_with_a_lost_state_and_goes_on_after_confirming() {
        val core = LostCore()
        rule.setContent { AppRoot(core, FakePlatform()) }
        rule.onNodeWithText("Saved state not readable", substring = true).assertExists()
        rule.onNodeWithText("Continue with an empty state").performClick()
        assertEquals(1, core.confirmed)
        rule.onNodeWithText("Saved state not readable", substring = true).assertDoesNotExist()
    }
}
