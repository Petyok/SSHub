package io.github.petyok.sshub.trust

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import io.github.petyok.sshub.fake.TrustSamples
import io.github.petyok.sshub.testing.assertNoVisualOverflow
import io.github.petyok.sshub.testing.assertTargets48
import io.github.petyok.sshub.testing.setScreen
import io.github.petyok.sshub.ui.screens.trust.CodeIssuedScreen
import io.github.petyok.sshub.ui.screens.trust.CodeIssuedState

@RunWith(AndroidJUnit4::class)
class CodeIssuedTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val ok = CodeIssuedState(TrustSamples.codeIssued(), TrustSamples.NOW)
    private val live = CodeIssuedState(TrustSamples.codeLive(), TrustSamples.NOW)

    @Test fun main_path_shows_the_code_once_in_four_groups_and_done_calls_back() {
        var done = 0
        rule.setScreen { CodeIssuedScreen(ok, issuedActions(onDone = { done++ })) }
        rule.onAllNodesWithText("Short code issued").onFirst().assertExists()
        rule.onNodeWithText("Argon2id sealed, valid for 7 days").assertExists()
        rule.onNode(hasContentDescription("Short code, K 7 Q 2, 9 F M X, C 4 1 T, 8 P W D")).assertExists()
        rule.onNodeWithText("The code exists only on this screen.").assertExists()
        rule.onNodeWithText("deploy@web-01").assertExists()
        rule.onNode(hasContentDescription("Expires 2026-11-02, in 7 days")).assertExists()
        rule.onNodeWithText("I wrote it down").performClick()
        assertEquals(1, done)
        rule.assertTargets48()
    }

    @Test fun no_copy_button_by_default() {
        rule.setScreen { CodeIssuedScreen(ok, issuedActions()) }
        rule.onAllNodesWithText("Copy", substring = true).assertCountEquals(0)
        rule.onAllNodesWithText("Copied", substring = true).assertCountEquals(0)
    }

    @Test fun d18_window_is_secure() {
        rule.setScreen { CodeIssuedScreen(ok, issuedActions()) }
        assertFlagSecure(rule)
    }

    @Test fun live_variant_refuses_and_has_no_code() {
        var done = 0
        rule.setScreen { CodeIssuedScreen(live, issuedActions(onDone = { done++ })) }
        rule.onNodeWithText("Refused. A short code is already live.").assertExists()
        rule.onAllNodesWithText("I wrote it down").assertCountEquals(0)
        rule.onNodeWithText("2026-10-31").assertExists()
        rule.onNodeWithText("Back to short codes").performClick()
        assertEquals(1, done)
        rule.assertTargets48()
    }

    @Test fun nothing_pending_says_so() {
        rule.setScreen { CodeIssuedScreen(CodeIssuedState(null, TrustSamples.NOW), issuedActions()) }
        rule.onNodeWithText("There is no new short code to show. A code is shown once, right after it is issued.").assertExists()
        rule.onAllNodesWithText("I wrote it down").assertCountEquals(0)
    }

    @Test fun renders_dark() {
        rule.setScreen(dark = true) { CodeIssuedScreen(ok, issuedActions()) }
        rule.onNodeWithText("I wrote it down").assertExists()
    }

    @Test fun renders_light() {
        rule.setScreen(dark = false) { CodeIssuedScreen(ok, issuedActions()) }
        rule.onNodeWithText("I wrote it down").assertExists()
    }

    @Test fun font_scale_2_has_no_overflow() {
        rule.setScreen(fontScale = 2f) { CodeIssuedScreen(ok, issuedActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_live_has_no_overflow() {
        rule.setScreen(fontScale = 2f) { CodeIssuedScreen(live, issuedActions()) }
        rule.assertNoVisualOverflow()
    }
}
