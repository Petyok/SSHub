package io.github.petyok.sshub.home

import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import org.junit.Rule
import org.junit.Test
import io.github.petyok.sshub.core.CameraPermission
import io.github.petyok.sshub.core.HomeFakeOpened
import io.github.petyok.sshub.core.Opened
import io.github.petyok.sshub.core.ScanEvent
import io.github.petyok.sshub.core.ScanPurpose
import io.github.petyok.sshub.core.ScanResults
import io.github.petyok.sshub.fake.FakePlatform
import io.github.petyok.sshub.fake.HomeScanFake
import io.github.petyok.sshub.testing.setScreen
import io.github.petyok.sshub.ui.screens.home.ScanActions
import io.github.petyok.sshub.ui.screens.home.ScanCamera
import io.github.petyok.sshub.ui.screens.home.ScanScreen
import io.github.petyok.sshub.ui.screens.home.ScanViewModel
import io.github.petyok.sshub.ui.screens.home.scanCameraOf

/** The scan screen wired to the real ScanViewModel over FakePlatform (FakeCamera) and a scripted ScanApi. */
class ScanFlowTest {
    @get:Rule val rule = createComposeRule()
    private val platform = FakePlatform()
    private val core = HomeScanFake()
    private var backs = 0

    private fun start(purpose: ScanPurpose = ScanPurpose.Request): ScanViewModel {
        val vm = ScanViewModel(core, platform, purpose)
        rule.setScreen(platform = platform) {
            val s by vm.state.collectAsState()
            ScanScreen(
                s,
                ScanActions(
                    onBack = { backs++ }, onGrant = vm::grant, onOpenSettings = vm::openSettings, onTogglePaste = vm::togglePaste,
                    onPasteText = vm::pasteText, onPasteFromClipboard = vm::pasteFromClipboard, onUsePasted = vm::usePasted,
                    onContinue = { vm.finish() }, onLuma = vm::onLuma,
                ),
            )
        }
        return vm
    }

    private fun frame() = platform.camera.onLuma!!.invoke(1, 1, ByteArray(1))

    @Test fun permission_mapping() {
        check(scanCameraOf(CameraPermission.Unknown, true) == ScanCamera.Rationale)
        check(scanCameraOf(CameraPermission.Granted, true) == ScanCamera.Live)
        check(scanCameraOf(CameraPermission.Denied, true) == ScanCamera.Denied)
        check(scanCameraOf(CameraPermission.DeniedForever, true) == ScanCamera.DeniedForever)
        check(scanCameraOf(CameraPermission.Granted, false) == ScanCamera.None)
    }

    @Test fun camera_permission_flow_rationale_denied_forever_granted() {
        platform.camera.perm.value = CameraPermission.Unknown
        start()
        rule.onNodeWithText("The camera reads the QR code").assertExists()
        rule.onNodeWithText("fake camera").assertDoesNotExist()
        rule.onNodeWithText("Allow the camera").performClick()
        check(platform.camera.requested == 1)

        platform.camera.perm.value = CameraPermission.Denied
        rule.waitForIdle()
        rule.onNodeWithText("Camera permission is off").assertExists()
        rule.onNodeWithText("Try again").performClick()
        check(platform.camera.requested == 2)

        platform.camera.perm.value = CameraPermission.DeniedForever
        rule.waitForIdle()
        rule.onNodeWithText("Open settings").performClick()
        check(platform.camera.settingsOpened == 1)

        platform.camera.perm.value = CameraPermission.Granted
        rule.waitForIdle()
        rule.onNodeWithText("fake camera").assertExists()
        rule.onNodeWithText("Camera permission is off").assertDoesNotExist()
    }

    @Test fun no_camera_hardware_shows_paste_only() {
        platform.camera.hasCamera = false
        start()
        rule.onNodeWithText("This phone has no camera").assertExists()
        rule.onNodeWithContentDescription("Pasted text").assertExists()
    }

    @Test fun progress_then_frame_ends_in_done_and_continue_publishes_the_result() {
        ScanResults.take(ScanPurpose.Chain)
        val vm = start(ScanPurpose.Chain)
        core.events += ScanEvent.Progress(3, 10)
        frame()
        rule.waitForIdle()
        rule.onNodeWithText("Reading: 3 of 10 parts").assertExists()

        core.events += ScanEvent.Frame(byteArrayOf(1, 2, 3))
        frame()
        rule.waitForIdle()
        rule.onNodeWithText("Read 10 of 10 parts. Checking the document.").assertExists()
        check(core.opened.size == 1)
        check(ScanResults.take(ScanPurpose.Chain) == null) { "published before Continue" }

        rule.onNodeWithText("Continue").performClick()
        val r = ScanResults.take(ScanPurpose.Chain)
        check(r != null && r.opened == HomeFakeOpened("opened") && r.purpose == ScanPurpose.Chain)
        check(vm.finish() != null)
        ScanResults.take(ScanPurpose.Chain)
    }

    @Test fun foreign_qr_is_ignored_and_scanning_goes_on() {
        start()
        core.events += ScanEvent.Foreign
        frame()
        rule.waitForIdle()
        rule.onNodeWithText("Ignored a QR code that is not what this step needs.").assertExists()
        core.events += ScanEvent.Progress(1, 4)
        frame()
        rule.waitForIdle()
        rule.onNodeWithText("Reading: 1 of 4 parts").assertExists()
    }

    @Test fun a_frame_the_core_does_not_act_on_shows_its_reason_and_offers_no_continue() {
        core.openResult = Opened.Ignored("The code is for another ring.")
        start()
        core.events += ScanEvent.Frame(byteArrayOf(9))
        frame()
        rule.waitForIdle()
        rule.onNodeWithText("The code is for another ring.").assertExists()
        rule.onNodeWithText("Continue").assertDoesNotExist()
        check(core.resets >= 2)
    }

    @Test fun pasted_text_goes_through_frame_from_text_and_open_frame() {
        start()
        rule.onNodeWithText("Paste text instead").performClick()
        rule.onNodeWithContentDescription("Pasted text").performTextInput("QUJDREVGR0g=")
        rule.onNodeWithText("Use pasted text").performScrollTo().performClick()
        rule.waitForIdle()
        check(core.opened.size == 1 && String(core.opened[0]) == "QUJDREVGR0g=")
        rule.onNodeWithText("Continue").assertExists()
    }

    @Test fun bad_pasted_text_shows_an_error_and_opens_nothing() {
        start()
        rule.onNodeWithText("Paste text instead").performClick()
        rule.onNodeWithContentDescription("Pasted text").performTextInput("not base64!")
        rule.onNodeWithText("Use pasted text").performScrollTo().performClick()
        rule.waitForIdle()
        rule.onNodeWithText("That is not base64 text. Copy the whole text the terminal shows.").performScrollTo().assertExists()
        check(core.opened.isEmpty())
    }

    @Test fun paste_from_clipboard_fills_the_box_on_tap_only() {
        platform.clipboard.text = "QUJDREVGR0g="
        start()
        rule.onNodeWithText("Paste text instead").performClick()
        rule.onNodeWithText("Paste from clipboard").performScrollTo().performClick()
        rule.waitForIdle()
        rule.onNodeWithContentDescription("Pasted text").assertTextContains("QUJDREVGR0g=")
    }

    @Test fun back_goes_back() {
        start()
        rule.onNodeWithContentDescription("Back").performClick()
        check(backs == 1)
    }
}
