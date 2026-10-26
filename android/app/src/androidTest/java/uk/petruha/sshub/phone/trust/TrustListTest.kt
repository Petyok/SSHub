package uk.petruha.sshub.phone.trust

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.hasText
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
import uk.petruha.sshub.phone.ui.screens.trust.TrustListScreen
import uk.petruha.sshub.phone.ui.screens.trust.TrustListState

@RunWith(AndroidJUnit4::class)
class TrustListTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val certs = TrustSamples.certs()
    private val list = TrustListState(certs)

    @Test fun main_path_lists_active_expired_and_revoked_with_words() {
        var scanned = 0
        rule.setScreen { TrustListScreen(list, listActions(onScan = { scanned++ })) }
        rule.onNodeWithText("4 certificates").assertExists()
        rule.onAllNodesWithText("active").assertCountEquals(2)
        rule.onAllNodesWithText("expired").assertCountEquals(1)
        rule.onAllNodesWithText("revoked").assertCountEquals(1)
        rule.onNodeWithText("5 hosts, 30 d, ends 2026-11-11").assertExists()
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
        rule.onNode(hasText("web-01, web-02, db-staging, bastion, nuc-lab", substring = true)).assertExists()
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
        rule.onNodeWithText("Revoke this trust session").performClick()
        assertEquals(listOf("3F9A-07C2"), asked)
    }

    @Test fun sheet_confirms_or_keeps_and_shows_no_dangerous_detail() {
        var revoked = 0; var kept = 0
        rule.setScreen { TrustListScreen(list.copy(confirming = "3F9A-07C2"), listActions(onRevoke = { revoked++ }, onKeep = { kept++ })) }
        rule.onNodeWithText("Revoke this trust session?").assertExists()
        rule.onNodeWithText("Revoke and sign").assertExists()
        rule.onNodeWithText("Keep it").assertExists()
        // collapsed list behind the sheet and a sheet without hosts, hashes or certificate bodies
        rule.onAllNodesWithText("web-01", substring = true).assertCountEquals(0)
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
