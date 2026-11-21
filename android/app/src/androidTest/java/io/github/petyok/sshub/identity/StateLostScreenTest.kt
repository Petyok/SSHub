package io.github.petyok.sshub.identity

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import io.github.petyok.sshub.app.AppRoot
import io.github.petyok.sshub.core.PhoneCoreApi
import io.github.petyok.sshub.fake.FakePhoneCore
import io.github.petyok.sshub.fake.FakePlatform
import io.github.petyok.sshub.ui.screens.identity.StateLostScreen

private class LostCore(private val inner: FakePhoneCore = FakePhoneCore()) : PhoneCoreApi by inner {
    var lost = true
    var confirmed = 0
    override fun stateLost() = lost
    override suspend fun confirmStateLost() { confirmed++; lost = false }
}

/** Security M2: the stop screen and its place in the shell. */
@RunWith(AndroidJUnit4::class)
class StateLostScreenTest {
    @get:Rule val rule = createComposeRule()

    @Test fun the_screen_offers_both_ways_out_and_says_nothing_was_written() {
        var cont = 0; var lock = 0
        rule.setContent { io.github.petyok.sshub.ui.theme.SshubTheme(dark = true, reducedMotion = false) { StateLostScreen(onContinue = { cont++ }, onLock = { lock++ }) } }
        rule.onNodeWithText("Nothing has been overwritten yet", substring = true).assertExists()
        rule.onNodeWithText("stays on this phone (app private)", substring = true).assertExists()
        rule.onNodeWithText("does not sign until it has taken its chain from two terminals", substring = true).assertExists()
        rule.onNodeWithText("terminals met before are met again", substring = true).assertExists()
        rule.onNodeWithText("Lock and leave it as it is").performClick()
        rule.onNodeWithText("Continue as a restored phone").performClick()
        assertEquals(1, lock); assertEquals(1, cont)
    }

    @Test fun the_shell_stops_on_the_screen_after_an_unlock_with_a_lost_state_and_goes_on_after_confirming() {
        val core = LostCore()
        rule.setContent { AppRoot(core, FakePlatform()) }
        rule.onNodeWithText("Saved state not readable", substring = true).assertExists()
        rule.onNodeWithText("Continue as a restored phone").performClick()
        assertEquals(1, core.confirmed)
        rule.onNodeWithText("Saved state not readable", substring = true).assertDoesNotExist()
    }

    @Test fun a_signer_block_together_with_a_lost_state_settles_on_the_stop_screen() {
        val settings = io.github.petyok.sshub.core.InMemorySettingsStore().also { it.setSignerMismatch(true) }
        val core = LostCore()
        rule.setContent { AppRoot(core, FakePlatform(), settings) }
        rule.waitForIdle()
        rule.mainClock.advanceTimeBy(500)
        rule.waitForIdle()
        rule.onNodeWithText("Saved state not readable", substring = true).assertExists() // no loop between About and the stop screen
        rule.onNodeWithText("Continue as a restored phone").performClick()
        rule.waitForIdle()
        rule.onNodeWithText("About and security", substring = true).assertExists() // once the stop is over the block applies
    }
}
