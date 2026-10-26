package uk.petruha.sshub.phone.home

import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertDoesNotExist
import androidx.compose.ui.test.assertExists
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasContentDescription
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
import uk.petruha.sshub.phone.ui.screens.home.ScanActions
import uk.petruha.sshub.phone.ui.screens.home.ScanScreen
import uk.petruha.sshub.phone.ui.screens.home.ScanState

class ScanScreenTest {
    @get:Rule val rule = createComposeRule()
    private val log = mutableListOf<String>()
    private val on = ScanActions(
        onBack = { log += "back" }, onGrant = { log += "grant" }, onOpenSettings = { log += "settings" }, onTogglePaste = { log += "toggle" },
        onPasteText = { log += "text:$it" }, onPasteFromClipboard = { log += "clip" }, onUsePasted = { log += "use" }, onContinue = { log += "continue" },
    )

    private fun show(s: ScanState, dark: Boolean = true, fontScale: Float = 1f) = rule.setScreen(dark, fontScale) { ScanScreen(s, on) }

    private fun live(description: String) =
        SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Polite) and hasContentDescription(description)

    @Test fun live_shows_hint_camera_and_ready_announcement() {
        show(HomeSamples.scanLive)
        rule.onNodeWithText("Scan a share request").assertExists()
        rule.onNodeWithText("Open the unlock screen on the terminal and point the camera at its QR code.").assertExists()
        rule.onNodeWithText("fake camera").assertExists()
        rule.onNode(live("Camera ready, point it at the QR code")).assertExists()
        rule.onNodeWithText("waiting for the first frame").assertExists()
    }

    @Test fun live_offers_paste_instead() {
        show(HomeSamples.scanLive)
        rule.onNodeWithText("Paste text instead").performClick()
        check(log == listOf("toggle")) { log.toString() }
    }

    @Test fun progress_shows_have_and_need() {
        show(HomeSamples.scanProgress)
        rule.onNodeWithText("Reading: 3 of 10 parts").assertExists()
        rule.onNodeWithText("frame 3 of 10").assertExists()
        rule.onNode(live("Reading 3 of 10 parts")).assertExists()
    }

    @Test fun ignored_is_announced_as_not_recognised() {
        show(HomeSamples.scanIgnored)
        rule.onNodeWithText("Ignored a QR code that is not what this step needs.").assertExists()
        rule.onNode(live("Code not recognised. Ignored a QR code that is not what this step needs.")).assertExists()
        rule.onNodeWithText("Continue").assertDoesNotExist()
    }

    @Test fun done_announces_code_read_and_waits_for_continue() {
        show(HomeSamples.scanDone)
        rule.onNode(live("Code read")).assertExists()
        rule.onNodeWithText("Read 10 of 10 parts. Checking the document.").assertExists()
        rule.onNodeWithText("fake camera").assertDoesNotExist()
        rule.onNodeWithText("Continue").performClick()
        check(log == listOf("continue")) { log.toString() }
    }

    @Test fun d11_nothing_continues_on_its_own() {
        show(HomeSamples.scanDone)
        rule.mainClock.advanceTimeBy(15 * 60 * 1000L)
        rule.waitForIdle()
        check(log.isEmpty()) { log.toString() }
    }

    @Test fun rationale_comes_first_with_allow_button() {
        show(HomeSamples.scanRationale)
        rule.onNodeWithText("The camera reads the QR code").assertIsDisplayed()
        rule.onNodeWithText("fake camera").assertDoesNotExist()
        rule.onNodeWithText("Allow the camera").performClick()
        check(log == listOf("grant")) { log.toString() }
    }

    @Test fun denied_offers_try_again_and_paste() {
        show(HomeSamples.scanDenied)
        rule.onNodeWithText("Camera permission is off").assertIsDisplayed()
        rule.onNodeWithText("fake camera").assertDoesNotExist()
        rule.onNodeWithText("Try again").performClick()
        rule.onNodeWithText("Paste text").performClick()
        check(log == listOf("grant", "toggle")) { log.toString() }
    }

    @Test fun denied_forever_offers_open_settings() {
        show(HomeSamples.scanDeniedForever)
        rule.onNodeWithText("Open settings").performClick()
        check(log == listOf("settings")) { log.toString() }
        rule.onNodeWithText("Try again").assertDoesNotExist()
    }

    @Test fun no_camera_goes_straight_to_paste_and_back() {
        show(HomeSamples.scanNone)
        rule.onNodeWithText("This phone has no camera").assertIsDisplayed()
        rule.onNodeWithContentDescription("Pasted text").assertExists()
        rule.onNodeWithText("Allow the camera").assertDoesNotExist()
        rule.onNodeWithText("Back").performClick()
        check(log == listOf("back")) { log.toString() }
    }

    @Test fun paste_panel_enables_use_only_with_text() {
        show(HomeSamples.scanPaste.copy(pasteText = ""))
        rule.onNodeWithText("Use pasted text").performScrollTo().assertIsNotEnabled()
    }

    @Test fun paste_panel_with_text_uses_it_and_reads_the_clipboard() {
        show(HomeSamples.scanPaste)
        rule.onNodeWithText("Use pasted text").performScrollTo().assertIsEnabled().performClick()
        rule.onNodeWithText("Paste from clipboard").performScrollTo().performClick()
        check(log == listOf("use", "clip")) { log.toString() }
        rule.onNodeWithText("Use the camera").assertExists()
    }

    @Test fun paste_error_is_text_not_colour() {
        show(HomeSamples.scanPasteError)
        rule.onNodeWithText("That is not base64 text. Copy the whole text the terminal shows.").performScrollTo().assertExists()
    }

    @Test fun every_purpose_has_its_own_title() {
        val titles = uk.petruha.sshub.phone.core.ScanPurpose.entries.map { uk.petruha.sshub.phone.ui.screens.home.scanTitle(it) }
        check(titles.toSet().size == titles.size)
    }

    @Test fun light_theme_renders_progress() {
        show(HomeSamples.scanProgress, dark = false)
        rule.onNodeWithText("Reading: 3 of 10 parts").assertIsDisplayed()
    }

    @Test fun dark_theme_renders_denied() {
        show(HomeSamples.scanDeniedForever, dark = true)
        rule.assertTargets48()
    }

    @Test fun targets_are_48_live_and_paste() {
        show(HomeSamples.scanPasteError)
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow_live() {
        show(HomeSamples.scanProgress, fontScale = 2f)
        rule.assertNoVisualOverflow()
    }

    @Test fun font_scale_2_has_no_overflow_denied_forever() {
        show(HomeSamples.scanDeniedForever, dark = false, fontScale = 2f)
        rule.assertNoVisualOverflow()
    }
}
