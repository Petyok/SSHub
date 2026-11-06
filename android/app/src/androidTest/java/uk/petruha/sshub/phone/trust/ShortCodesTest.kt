package uk.petruha.sshub.phone.trust

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uk.petruha.sshub.phone.fake.TrustSamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.trust.ShortCodesScreen
import uk.petruha.sshub.phone.ui.screens.trust.ShortCodesState

@RunWith(AndroidJUnit4::class)
class ShortCodesTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val list = ShortCodesState(TrustSamples.NOW, TrustSamples.codes())

    @Test fun main_path_rows_states_and_expiry_on_a_tick_scale() {
        rule.setScreen { ShortCodesScreen(list, codesActions()) }
        rule.onNodeWithText("One live code per credential. The terminal shows the same host id. The code itself is never stored here.").assertExists()
        rule.onNodeWithText("4 codes").assertExists()
        rule.onAllNodesWithText("live").assertCountEquals(2)
        rule.onAllNodesWithText("expired").assertCountEquals(1)
        rule.onAllNodesWithText("declared dead").assertCountEquals(1)
        rule.onAllNodesWithText("expires (7 days)").assertCountEquals(4)
        rule.onNode(hasContentDescription("Issued 2026-10-24, expires 2026-10-31, 4 days 14 hours left")).assertExists()
        rule.onNode(hasContentDescription("Issued 2026-10-12, expired 2026-10-19")).assertExists()
        rule.assertTargets48()
    }

    @Test fun declare_it_dead_is_offered_only_for_a_live_code_and_asks_with_the_host() {
        val asked = mutableListOf<String>()
        rule.setScreen { ShortCodesScreen(list, codesActions(onAskDead = { asked += it })) }
        rule.onAllNodesWithText("Declare it dead").assertCountEquals(2)
        rule.onAllNodesWithText("Declare it dead").onFirst().performClick()
        assertEquals(listOf(TrustSamples.uid(1)), asked)
    }

    @Test fun sheet_has_a_warning_and_confirms_or_keeps() {
        var dead = 0; var kept = 0
        rule.setScreen { ShortCodesScreen(list.copy(confirming = TrustSamples.uid(1)), codesActions(onDeclareDead = { dead++ }, onKeep = { kept++ })) }
        rule.onNodeWithText("Declare it dead?").assertExists()
        rule.onNodeWithText("Only when you know the sealed blob never reached the terminal, or the credential was reissued.").assertExists()
        rule.onNodeWithText("If it did arrive, the old blob stays valid until it expires.").assertExists()
        rule.onNodeWithText("Keep it live").assertExists()
        rule.onNodeWithTag("sheet-confirm").performClick()
        assertEquals(1, dead)
        rule.onNodeWithTag("sheet-cancel").performClick()
        assertEquals(1, kept)
    }

    @Test fun empty_state() {
        rule.setScreen { ShortCodesScreen(ShortCodesState(TrustSamples.NOW, emptyList()), codesActions()) }
        rule.onNodeWithText("None.").assertExists()
        rule.onNodeWithText("No short codes issued. You can issue one when you approve a release.").assertExists()
        rule.onAllNodesWithText("Declare it dead").assertCountEquals(0)
    }

    @Test fun error_is_shown() {
        rule.setScreen { ShortCodesScreen(list.copy(error = "No live short code for that host."), codesActions()) }
        rule.onNodeWithText("No live short code for that host.").assertExists()
    }

    @Test fun renders_dark() {
        rule.setScreen(dark = true) { ShortCodesScreen(list, codesActions()) }
        rule.onAllNodes(hasContentDescription("Host id 0 1 0 1, 0 1 0 1", substring = true), useUnmergedTree = true).assertCountEquals(1)
        rule.assertTargets48()
    }

    @Test fun renders_light() {
        rule.setScreen(dark = false) { ShortCodesScreen(list, codesActions()) }
        rule.onAllNodes(hasContentDescription("Host id 0 1 0 1, 0 1 0 1", substring = true), useUnmergedTree = true).assertCountEquals(1)
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow() {
        rule.setScreen(fontScale = 2f) { ShortCodesScreen(list, codesActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_empty_has_no_overflow() {
        rule.setScreen(fontScale = 2f) { ShortCodesScreen(ShortCodesState(TrustSamples.NOW, emptyList()), codesActions()) }
        rule.assertNoVisualOverflow()
    }
}
