package io.github.petyok.sshub.release

import androidx.activity.ComponentActivity
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import io.github.petyok.sshub.core.OpenedRefused
import io.github.petyok.sshub.core.ReleaseCheck
import io.github.petyok.sshub.fake.ReleaseSamples
import io.github.petyok.sshub.testing.assertNoVisualOverflow
import io.github.petyok.sshub.testing.assertTargets48
import io.github.petyok.sshub.testing.setScreen
import io.github.petyok.sshub.ui.screens.release.RefusalActions
import io.github.petyok.sshub.ui.screens.release.RefusalScreen

class RefusalScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val mismatchKeys = setOf("signer", "badsig", "wrongcred", "certother")

    private fun ComposeContentTestRule.show(
        initial: OpenedRefused, dark: Boolean = true, fontScale: Float = 1f, log: MutableList<String> = mutableListOf(),
    ): MutableState<OpenedRefused> {
        val st = mutableStateOf(initial)
        setScreen(dark, fontScale) {
            RefusalScreen(st.value, RefusalActions(onBack = { log += "back" }, onScanAgain = { log += "scan" }))
        }
        return st
    }

    private fun noApproveButton() {
        rule.onAllNodes(hasClickAction() and hasText("Approve", substring = true)).assertCountEquals(0)
        rule.onAllNodes(hasClickAction() and hasText("Continue", substring = true, ignoreCase = true)).assertCountEquals(0)
    }

    @Test fun there_are_exactly_18_refusals_with_distinct_keys() {
        assertEquals(18, ReleaseSamples.refusals.size)
        assertEquals(18, ReleaseSamples.refusals.map { it.reason.key }.toSet().size)
    }

    @Test fun every_variant_shows_banner_title_and_meaning_and_never_an_approve_button() {
        val s = rule.show(ReleaseSamples.refusals.first())
        for (r in ReleaseSamples.refusals) {
            rule.runOnIdle { s.value = r }
            rule.onNodeWithText("REFUSED", substring = true).assertExists()
            rule.onNodeWithText(r.reason.title).assertExists()
            rule.onNodeWithText(r.reason.text).assertExists()
            r.reason.compared.forEach { rule.onNodeWithText(it.value).assertExists() }
            rule.onNodeWithText("Nothing was signed.").assertExists()
            noApproveButton()
        }
    }

    @Test fun main_path_back_and_scan_again() {
        val log = mutableListOf<String>()
        rule.show(ReleaseSamples.refusal("expired"), log = log)
        rule.onNodeWithText("The challenge expired 47 s ago.").assertExists()
        rule.onNodeWithText("Expired").assertExists()
        rule.onNodeWithText("47 s ago").assertExists()
        rule.onNodeWithText("Back").performClick()
        rule.onNodeWithText("Scan again").performClick()
        assertEquals(listOf("back", "scan"), log)
    }

    @Test fun d14_a_mismatch_is_a_stop_box_with_both_values_and_back_as_the_only_action() {
        val s = rule.show(ReleaseSamples.refusals.first())
        for (key in mismatchKeys) {
            val r = ReleaseSamples.refusal(key)
            rule.runOnIdle { s.value = r }
            assertTrue(r.reason.mismatch)
            rule.onNodeWithText(r.reason.title).assertExists()
            assertEquals(2, r.reason.compared.size)
            r.reason.compared.forEach { rule.onNodeWithText(it.value).assertExists(); rule.onNodeWithText(it.key).assertExists() }
            rule.onAllNodesWithText("Scan again").assertCountEquals(0)
            rule.onAllNodesWithText("Back").assertCountEquals(1)
            noApproveButton()
        }
    }

    @Test fun only_the_core_decides_what_is_a_mismatch() {
        assertEquals(mismatchKeys, ReleaseSamples.refusals.filter { it.reason.mismatch }.map { it.reason.key }.toSet())
    }

    @Test fun refusal_checks_come_from_the_core_value() {
        val base = ReleaseSamples.refusal("badsig")
        rule.show(base.copy(checks = listOf(ReleaseCheck("Signature of 9C61-E2A8 verifies", false), ReleaseCheck("Not expired", true))))
        rule.onNodeWithText("1 of 2 pass").assertExists()
        rule.onNodeWithText("Checks").assertExists()
    }

    @Test fun the_banner_is_the_first_element_and_says_refused_with_the_short_label() {
        rule.show(ReleaseSamples.refusal("old12"))
        val header = rule.onNodeWithText("Refused").fetchSemanticsNode().boundsInRoot
        val banner = rule.onNodeWithText("REFUSED").fetchSemanticsNode().boundsInRoot
        val title = rule.onNodeWithText("The request was signed 14 min ago.").fetchSemanticsNode().boundsInRoot
        assertTrue(banner.top >= header.bottom - 1f)
        assertTrue(banner.bottom <= title.top + 1f)
        rule.onNodeWithText("signed too long ago").assertExists()
    }

    @Test fun dark_theme() {
        rule.show(ReleaseSamples.refusal("clock"), dark = true)
        rule.onNodeWithText("The challenge expired 212 s ago. Check the phone's clock.").assertExists()
        rule.assertTargets48()
    }

    @Test fun light_theme_mismatch() {
        rule.show(ReleaseSamples.refusal("certother"), dark = false)
        rule.onNodeWithText("A trust certificate must be issued by the phone itself. This one was written by another device.").assertExists()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_visual_overflow_for_every_variant() {
        val s = rule.show(ReleaseSamples.refusals.first(), fontScale = 2f)
        for (r in ReleaseSamples.refusals) {
            rule.runOnIdle { s.value = r }
            rule.assertNoVisualOverflow()
            rule.assertTargets48()
        }
    }

    @Test fun font_scale_1_3() {
        rule.show(ReleaseSamples.refusal("wrongcred"), fontScale = 1.3f)
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }
}
