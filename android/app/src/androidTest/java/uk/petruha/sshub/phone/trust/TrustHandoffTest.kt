package uk.petruha.sshub.phone.trust

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
import uk.petruha.sshub.phone.core.HandoffKind
import uk.petruha.sshub.phone.core.TrustHandoff
import uk.petruha.sshub.phone.fake.TrustSamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.trust.TrustHandoffState
import uk.petruha.sshub.phone.ui.screens.trust.TrustRevokedScreen
import uk.petruha.sshub.phone.ui.screens.trust.TrustSignedScreen

@RunWith(AndroidJUnit4::class)
class TrustHandoffTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val signed = TrustHandoffState(TrustHandoff(HandoffKind.Certificate, TrustSamples.requestOk.signedSummary, TrustSamples.outgoing("trust-1")))
    private val revoked = TrustHandoffState(
        TrustHandoff(HandoffKind.Revocation, "laptop-old [1D88-A340], 2 hosts, was to end 2026-11-27", TrustSamples.outgoing("revoke-6B20-F1D5")),
    )

    @Test fun signed_main_path_qr_text_three_proofs_and_buttons() {
        var copied = 0; var done = 0
        rule.setScreen { TrustSignedScreen(signed, handoffActions(onCopy = { copied++ }, onDone = { done++ })) }
        rule.onNodeWithText("SIGNED").assertExists()
        rule.onNodeWithText("laptop-main [7B3E-0D15], 5 hosts, 30 days, ends 2026-11-25").assertExists()
        rule.onNode(hasContentDescription("QR code with the signed certificate", substring = true)).assertExists()
        rule.onNode(hasContentDescription("Signed text")).assertExists()
        rule.onNode(hasContentDescription("Proof 1 of 3, phone, signed")).assertExists()
        rule.onNode(hasContentDescription("Proof 2 of 3, second, waiting")).assertExists()
        rule.onNode(hasContentDescription("Proof 3 of 3, factor, waiting")).assertExists()
        rule.onNodeWithText("Copy the text").performClick()
        rule.onNodeWithText("Done").performClick()
        assertEquals(1, copied); assertEquals(1, done)
        rule.assertTargets48()
    }

    @Test fun nothing_is_copied_by_default_and_the_copy_is_announced() {
        rule.setScreen { TrustSignedScreen(signed, handoffActions()) }
        rule.onAllNodesWithText("Copied", substring = true).assertCountEquals(0)
    }

    @Test fun copied_line_shows_the_clearing_time() {
        rule.setScreen { TrustSignedScreen(signed.copy(clearsIn = 60), handoffActions()) }
        rule.onNodeWithText("Copied, clears in 60 s").assertExists()
        rule.onNodeWithText("paste it into the terminal").assertExists()
    }

    @Test fun d18_signed_window_is_secure() {
        rule.setScreen { TrustSignedScreen(signed, handoffActions()) }
        assertFlagSecure(rule)
    }

    @Test fun d18_revoked_window_is_secure() {
        rule.setScreen { TrustRevokedScreen(revoked, handoffActions()) }
        assertFlagSecure(rule)
    }

    @Test fun revoked_main_path() {
        var done = 0
        rule.setScreen { TrustRevokedScreen(revoked, handoffActions(onDone = { done++ })) }
        rule.onNodeWithText("REVOKED").assertExists()
        rule.onNodeWithText("laptop-old [1D88-A340], 2 hosts, was to end 2026-11-27").assertExists()
        rule.onNode(hasContentDescription("QR code with the signed revocation", substring = true)).assertExists()
        rule.onNodeWithText("Copy the text").assertExists()
        rule.onNodeWithText("Done").performClick()
        assertEquals(1, done)
        rule.assertTargets48()
    }

    @Test fun signed_without_a_reply_says_so_and_offers_done() {
        var done = 0
        rule.setScreen { TrustSignedScreen(TrustHandoffState(null), handoffActions(onDone = { done++ })) }
        rule.onAllNodesWithText("Copy the text").assertCountEquals(0)
        rule.onNodeWithText("Done").performClick()
        assertEquals(1, done)
    }

    @Test fun signed_dark() {
        rule.setScreen(dark = true) { TrustSignedScreen(signed, handoffActions()) }
        rule.onAllNodesWithText("Trust session signed").onFirst().assertExists()
    }

    @Test fun signed_light() {
        rule.setScreen(dark = false) { TrustSignedScreen(signed, handoffActions()) }
        rule.onAllNodesWithText("Trust session signed").onFirst().assertExists()
    }

    @Test fun revoked_light() {
        rule.setScreen(dark = false) { TrustRevokedScreen(revoked, handoffActions()) }
        rule.onAllNodesWithText("Trust session revoked").onFirst().assertExists()
    }

    @Test fun signed_font_scale_2_has_no_overflow() {
        rule.setScreen(fontScale = 2f) { TrustSignedScreen(signed.copy(clearsIn = 42), handoffActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun revoked_font_scale_2_has_no_overflow() {
        rule.setScreen(fontScale = 2f) { TrustRevokedScreen(revoked, handoffActions()) }
        rule.assertNoVisualOverflow()
    }
}
