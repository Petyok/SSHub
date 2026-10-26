package uk.petruha.sshub.phone.home

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.fake.HomeSamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.home.HomeActions
import uk.petruha.sshub.phone.ui.screens.home.HomeState
import uk.petruha.sshub.phone.ui.screens.home.RingHomeScreen

class HomeScreenTest {
    @get:Rule val rule = createComposeRule()
    private val log = mutableListOf<String>()
    private val on = HomeActions(
        onOpenPending = { log += "open:" + it.title }, onOpenApprovals = { log += "approvals" }, onEnroll = { log += "enroll" },
        onRestore = { log += "restore" }, onTakeChain = { log += "chain" },
    )

    private fun show(s: HomeState, dark: Boolean = true, fontScale: Float = 1f) = rule.setScreen(dark, fontScale) { RingHomeScreen(s, on) }

    @Test fun main_path_shows_ring_counts_sync_and_chain() {
        show(HomeSamples.home)
        rule.onNodeWithText("home ring").assertIsDisplayed()
        rule.onNodeWithText("epoch 7").assertExists()
        rule.onNodeWithContentDescription("38 hosts").assertExists()
        rule.onNodeWithContentDescription("12 credentials").assertExists()
        rule.onNodeWithContentDescription("2 of 3 to unlock").assertExists()
        rule.onNodeWithText("9 Quorum, 3 Single credentials").assertExists()
        rule.onNodeWithText("3 min ago").assertExists()
        rule.onNodeWithContentDescription("Last sync 3 min ago, via laptop-main, ssh, scale of 60 minutes").assertExists()
        rule.onNodeWithText("4 pending").assertExists()
        rule.onNodeWithText("5 in ring").assertExists()
    }

    @Test fun chain_rows_carry_the_spec_descriptions() {
        show(HomeSamples.home)
        rule.onNodeWithContentDescription("pixel-8, this phone, seen now").assertExists()
        rule.onNodeWithContentDescription("laptop-main, seen 3 min ago").assertExists()
        rule.onNodeWithContentDescription("workstation, seen 14 min ago").assertExists()
        rule.onNodeWithContentDescription("laptop-old, stale, last seen 41 days ago").assertExists()
    }

    @Test fun grouped_pending_rows_open_the_list_when_more_than_one() {
        show(HomeSamples.home)
        rule.onNodeWithText("Red operations").performScrollTo().performClick()
        rule.onNodeWithText("Share releases").performScrollTo().performClick()
        check(log == listOf("approvals", "approvals")) { log.toString() }
    }

    @Test fun a_single_pending_item_opens_directly() {
        show(HomeSamples.home.copy(pending = HomeSamples.pending.take(1)))
        rule.onNodeWithText("Red operations").performScrollTo().performClick()
        check(log == listOf("open:Revoke device laptop-old")) { log.toString() }
    }

    @Test fun nothing_pending_says_so() {
        show(HomeSamples.homeNoPending)
        rule.onNodeWithText("0 pending").assertExists()
        rule.onNodeWithText("Nothing waiting.").assertExists()
        rule.onNodeWithText("Red operations").assertDoesNotExist()
    }

    @Test fun red_card_only_when_the_chain_is_not_taken() {
        show(HomeSamples.home)
        rule.onNodeWithText("This phone does not sign yet.").assertDoesNotExist()
    }

    @Test fun red_card_leads_to_taking_the_chain() {
        show(HomeSamples.homeHeadPending)
        rule.onNodeWithText("This phone does not sign yet.").assertIsDisplayed()
        rule.onNodeWithText("Take my chain from a terminal").performClick()
        check(log == listOf("chain")) { log.toString() }
    }

    @Test fun sparse_core_leaves_unknown_lines_out() {
        show(HomeSamples.homeSparse)
        rule.onNodeWithText("never").assertExists()
        rule.onNodeWithContentDescription("38 hosts").assertDoesNotExist()
        rule.onNodeWithText("9 Quorum, 3 Single credentials").assertDoesNotExist()
        rule.onNodeWithContentDescription("laptop-main, not seen").assertExists()
    }

    @Test fun empty_home_shows_the_phone_and_two_actions() {
        show(HomeSamples.homeEmpty)
        rule.onNodeWithText("no ring yet").assertIsDisplayed()
        rule.onNodeWithText("not enrolled").assertExists()
        rule.onNodeWithText("4F2A-91C7").assertExists()
        rule.onNodeWithText("no ops yet").assertExists()
        rule.onNodeWithText("1 here, 0 terminals").assertExists()
        rule.onNodeWithText("Restore").performClick()
        rule.onNodeWithText("Enroll with a terminal").performClick()
        check(log == listOf("restore", "enroll")) { log.toString() }
    }

    @Test fun dark_and_light_render() {
        show(HomeSamples.home, dark = true)
        rule.onNodeWithText("home ring").assertIsDisplayed()
    }

    @Test fun light_renders() {
        show(HomeSamples.homeHeadPending, dark = false)
        rule.onNodeWithText("home ring").assertIsDisplayed()
    }

    @Test fun targets_are_48() {
        show(HomeSamples.homeHeadPending)
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow() {
        show(HomeSamples.homeHeadPending, fontScale = 2f)
        rule.assertNoVisualOverflow()
    }

    @Test fun empty_home_font_scale_2_has_no_overflow() {
        show(HomeSamples.homeEmpty, dark = false, fontScale = 2f)
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }
}
