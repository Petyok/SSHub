package uk.petruha.sshub.phone.trust

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uk.petruha.sshub.phone.fake.TrustSamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.trust.TrustListScreen
import uk.petruha.sshub.phone.ui.screens.trust.TrustListState

@RunWith(AndroidJUnit4::class)
class TrustListTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val certs = TrustSamples.certs()
    private val list = TrustListState(certs)

    @Test fun main_path_lists_active_expired_and_revoked_with_words() {
        rule.setScreen { TrustListScreen(list, listActions()) }
        rule.onNodeWithText("4 certificates").assertExists()
        rule.onAllNodesWithText("active").assertCountEquals(2)
        rule.onAllNodesWithText("expired").assertCountEquals(1)
        rule.onAllNodesWithText("revoked").assertCountEquals(1)
        rule.onNodeWithText("5 hosts, 30 days, ends 2026-11-11").assertExists()
        rule.onNodeWithText("group prod-web, 60 days, ends 2026-11-27").assertExists()
        rule.onAllNodesWithText("epoch 7 to 8").assertCountEquals(2)
    }

    @Test fun a_device_never_met_shows_not_met_and_its_id_never_an_empty_name() {
        rule.setScreen { TrustListScreen(list, listActions()) }
        rule.onNodeWithText("not met [5A19-C0E4]").assertExists()
    }

    @Test fun expanded_group_cert_shows_the_group_instead_of_a_host_entry_and_the_chain_position() {
        rule.setScreen { TrustListScreen(list.copy(expanded = setOf("6B20-F1D5", "8E55-1C3B")), listActions()) }
        rule.onNodeWithText("prod-web").assertExists()
        rule.onAllNodesWithText("group").assertCountEquals(1)
        rule.onAllNodesWithText("chain position").assertCountEquals(1)
        rule.onAllNodesWithText("group prod-web", substring = true).assertCountEquals(1)
    }

    @Test fun scan_is_offered_and_targets_are_48() {
        var scanned = 0
        rule.setScreen { TrustListScreen(list, listActions(onScan = { scanned++ })) }
        rule.onNodeWithText("Scan a request").assertIsEnabled().performClick()
        assertEquals(1, scanned)
        rule.assertTargets48()
    }

    @Test fun d18_window_is_secure() {
        rule.setScreen { TrustListScreen(list, listActions()) }
        assertFlagSecure(rule)
    }

    @Test fun tapping_a_card_toggles_and_expanded_shows_every_host_and_extension() {
        val toggled = mutableListOf<String>()
        rule.setScreen { TrustListScreen(list.copy(expanded = setOf("3F9A-07C2")), listActions(onToggle = { toggled += it })) }
        rule.onAllNodes(hasContentDescription("Host id 0 5 0 5, 0 5 0 5", substring = true), useUnmergedTree = true).assertCountEquals(1)
        rule.onNode(hasText("+10 new hosts, 2026-10-26 03:03", substring = true)).assertExists()
        rule.onNodeWithText("3F9A-07C2").assertExists()
        rule.onAllNodesWithText("laptop-main").onFirst().performClick()
        assertEquals(listOf("3F9A-07C2"), toggled)
    }

    @Test fun revoke_button_only_on_an_active_certificate() {
        rule.setScreen { TrustListScreen(list.copy(expanded = certs.map { it.certId }.toSet()), listActions()) }
        rule.onAllNodesWithText("Revoke this trust session").assertCountEquals(2)
    }

    @Test fun revoke_button_asks_with_the_cert_id() {
        val asked = mutableListOf<String>()
        rule.setScreen { TrustListScreen(list.copy(expanded = setOf("3F9A-07C2")), listActions(onAskRevoke = { asked += it })) }
        rule.onNodeWithText("Revoke this trust session").performScrollTo().performClick()
        assertEquals(listOf("3F9A-07C2"), asked)
    }

    @Test fun sheet_confirms_or_keeps_and_shows_no_dangerous_detail() {
        var revoked = 0; var kept = 0
        rule.setScreen { TrustListScreen(list.copy(confirming = "3F9A-07C2"), listActions(onRevoke = { revoked++ }, onKeep = { kept++ })) }
        rule.onNodeWithText("Revoke this trust session?").assertExists()
        rule.onNodeWithText("Revoke and sign").assertExists()
        rule.onNodeWithText("Keep it").assertExists()
        // collapsed list behind the sheet and a sheet without hosts, hashes or certificate bodies
        rule.onAllNodes(hasContentDescription("Host id", substring = true), useUnmergedTree = true).assertCountEquals(0)
        rule.onAllNodesWithText("7B3E-0D15", substring = true).assertCountEquals(0)
        rule.onNodeWithTag("sheet-confirm").performClick()
        assertEquals(1, revoked)
        rule.onNodeWithTag("sheet-cancel").performClick()
        assertEquals(1, kept)
    }

    @Test fun empty_state_still_offers_scan() {
        rule.setScreen { TrustListScreen(TrustListState(emptyList()), listActions()) }
        rule.onNodeWithText("No trust sessions issued").assertExists()
        rule.onNodeWithText("A terminal asks for one by showing a request. Scan it: this phone signs the certificate.").assertExists()
        rule.onNodeWithText("Scan a request").assertExists()
        rule.assertTargets48()
    }

    @Test fun error_is_shown() {
        rule.setScreen { TrustListScreen(list.copy(error = "Only an active certificate can be revoked."), listActions()) }
        rule.onNodeWithText("Only an active certificate can be revoked.").assertExists()
    }

    @Test fun renders_dark() {
        rule.setScreen(dark = true) { TrustListScreen(list.copy(expanded = setOf("3F9A-07C2")), listActions()) }
        rule.onNodeWithText("Revoke this trust session").assertExists()
        rule.assertTargets48()
    }

    @Test fun renders_light() {
        rule.setScreen(dark = false) { TrustListScreen(list.copy(expanded = setOf("3F9A-07C2")), listActions()) }
        rule.onNodeWithText("Revoke this trust session").assertExists()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow() {
        rule.setScreen(fontScale = 2f) { TrustListScreen(list.copy(expanded = setOf("3F9A-07C2")), listActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_empty_has_no_overflow() {
        rule.setScreen(fontScale = 2f) { TrustListScreen(TrustListState(emptyList()), listActions()) }
        rule.assertNoVisualOverflow()
    }
}
