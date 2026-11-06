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

    @Test fun main_path_shows_this_phone_ring_and_pending() {
        show(HomeSamples.home)
        rule.onNodeWithText("ring epoch").assertExists()
        rule.onNodeWithText("7").assertExists()
        rule.onNodeWithText("head seq").assertExists()
        rule.onNodeWithText("12").assertExists()
        rule.onNodeWithText("live short codes").assertExists()
        rule.onNodeWithText("4 pending").assertExists()
        rule.onNodeWithText("4 in ring").assertExists()
    }

    @Test fun chain_rows_carry_the_spec_descriptions() {
        show(HomeSamples.home)
        rule.onNodeWithContentDescription("pixel-8, this phone").assertExists()
        rule.onNodeWithContentDescription("laptop-main").assertExists()
        rule.onNodeWithContentDescription("workstation").assertExists()
    }

    @Test fun an_unmet_device_shows_not_met_and_its_id() {
        show(HomeSamples.home)
        rule.onNodeWithContentDescription("not met, C5F0-27B9").assertExists()
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

    @Test fun h5_a_restored_phone_without_a_ring_still_shows_the_card_and_the_button() {
        show(HomeSamples.homeEmpty.copy(headPending = true))
        rule.onNodeWithText("This phone does not sign yet.").assertIsDisplayed()
        rule.onNodeWithText("unknown, not taken yet").assertExists()
        rule.onNodeWithText("no ops yet").assertDoesNotExist()
        rule.onNodeWithText("Take my chain from a terminal").performClick()
        check(log == listOf("chain")) { log.toString() }
    }

    @Test fun h5_no_card_for_a_never_restored_empty_home() {
        show(HomeSamples.homeEmpty)
        rule.onNodeWithText("This phone does not sign yet.").assertDoesNotExist()
        rule.onNodeWithText("no ops yet").assertExists()
    }
    @Test fun dark_and_light_render() {
        show(HomeSamples.home, dark = true)
        rule.onNodeWithText("ring epoch").assertIsDisplayed()
    }

    @Test fun light_renders() {
        show(HomeSamples.homeHeadPending, dark = false)
        rule.onNodeWithText("ring epoch").assertIsDisplayed()
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
