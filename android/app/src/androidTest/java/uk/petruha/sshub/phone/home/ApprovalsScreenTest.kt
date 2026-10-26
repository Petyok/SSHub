package uk.petruha.sshub.phone.home

import androidx.compose.ui.test.assertExists
import androidx.compose.ui.test.assertDoesNotExist
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.fake.HomeSamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.home.ApprovalsActions
import uk.petruha.sshub.phone.ui.screens.home.ApprovalsScreen
import uk.petruha.sshub.phone.ui.screens.home.ApprovalsState

class ApprovalsScreenTest {
    @get:Rule val rule = createComposeRule()
    private val log = mutableListOf<String>()
    private val on = ApprovalsActions(onSelect = { log += "select:$it" }, onOpen = { log += "open:" + it.title }, onScan = { log += "scan" })

    private fun show(s: ApprovalsState, dark: Boolean = true, fontScale: Float = 1f) = rule.setScreen(dark, fontScale) { ApprovalsScreen(s, on) }

    @Test fun red_segment_lists_red_items_and_opens_one() {
        show(HomeSamples.approvals)
        rule.onNodeWithText("4 waiting").assertExists()
        rule.onNodeWithText("Revoke device laptop-old").assertIsDisplayed()
        rule.onNodeWithText("db-staging").assertDoesNotExist()
        rule.onNodeWithText("Revoke device laptop-old").performClick()
        check(log == listOf("open:Revoke device laptop-old")) { log.toString() }
    }

    @Test fun segments_switch_through_the_callback() {
        show(HomeSamples.approvals)
        rule.onNodeWithText("Releases").performClick()
        rule.onNodeWithText("Trust").performClick()
        check(log == listOf("select:1", "select:2")) { log.toString() }
    }

    @Test fun releases_segment_shows_releases() {
        show(HomeSamples.approvalsReleases)
        rule.onNodeWithText("db-staging").assertIsDisplayed()
        rule.onNodeWithText("bastion").assertIsDisplayed()
        rule.onNodeWithText("Revoke device laptop-old").assertDoesNotExist()
    }

    @Test fun trust_segment_groups_trust_extend_recovery_reshare() {
        show(HomeSamples.approvalsTrust)
        rule.onNodeWithText("Certificate for laptop-main").assertIsDisplayed()
        rule.onNodeWithText("Extend budget of workstation").assertIsDisplayed()
        rule.onNodeWithText("Extend").assertExists()
    }

    @Test fun empty_segment_shows_the_empty_state() {
        show(HomeSamples.approvalsTrustEmpty)
        rule.onNodeWithText("Nothing waiting").assertIsDisplayed()
    }

    @Test fun nothing_waiting_at_all() {
        show(HomeSamples.approvalsNone)
        rule.onNodeWithText("none").assertExists()
        rule.onNodeWithText("Nothing waiting").assertIsDisplayed()
        rule.onNodeWithText("Requests arrive when a terminal shows a code and you scan it. Nothing is pushed to this phone, and it keeps no connection open.").assertExists()
    }

    @Test fun scan_a_request_is_the_bar_action() {
        show(HomeSamples.approvalsNone)
        rule.onNodeWithText("Scan a request").performClick()
        check(log == listOf("scan")) { log.toString() }
    }

    @Test fun light_renders() {
        show(HomeSamples.approvals, dark = false)
        rule.onNodeWithText("Scan a request").assertIsDisplayed()
    }

    @Test fun targets_are_48() {
        show(HomeSamples.approvalsTrust)
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow() {
        show(HomeSamples.approvalsTrust, fontScale = 2f)
        rule.assertNoVisualOverflow()
    }
}
