package uk.petruha.sshub.phone.release

import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.core.DeviceIdScanned
import uk.petruha.sshub.phone.core.ScanEvent
import uk.petruha.sshub.phone.core.ScanPurpose
import uk.petruha.sshub.phone.core.ScanResults
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.fake.HomeScanFake
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.home.ScanActions
import uk.petruha.sshub.phone.ui.screens.home.ScanScreen
import uk.petruha.sshub.phone.ui.screens.home.ScanViewModel
import uk.petruha.sshub.phone.ui.screens.home.parseDeviceIdPayload
import uk.petruha.sshub.phone.ui.screens.home.scanHint
import uk.petruha.sshub.phone.ui.screens.home.scanTitle

/** U13-16: the approver scans the proposer's device id QR (`sshub:dev:1:<16 hex>`) instead of typing it. */
class DeviceIdScanTest {
    @get:Rule val rule = createComposeRule()
    private val platform = FakePlatform()
    private val core = HomeScanFake()

    private fun parse(s: String) = parseDeviceIdPayload(s.toByteArray(Charsets.UTF_8))

    @Test fun payload_table() {
        val id = "723ed85f0566df0c"
        assertEquals(id, parse("sshub:dev:1:$id"))
        assertEquals(id, parse("SSHUB:DEV:1:723ED85F0566DF0C"))
        assertEquals(id, parse("  sshub:dev:1:$id\n"))
        assertEquals(id, parse("sshub:dev:1:723e d85f 0566 df0c"))
        assertEquals(id, parse("sshub:dev:1:723E-D85F-0566-DF0C"))
        assertNull(parse("sshub:dev:1:723ed85f0566df0"))      // 15 digits
        assertNull(parse("sshub:dev:1:723ed85f0566df0cc"))    // 17 digits
        assertNull(parse("sshub:dev:1:723ed85f0566df0g"))     // not hex
        assertNull(parse("sshub:dev:2:$id"))                  // another version
        assertNull(parse("sshub:host:1:$id"))
        assertNull(parse(id))
        assertNull(parse(""))
        assertNull(parseDeviceIdPayload(byteArrayOf(0x53, 0x7f, -1)))
    }

    @Test fun the_scan_purpose_has_its_own_words() {
        assertEquals("Scan the device id", scanTitle(ScanPurpose.DeviceId))
        assertEquals("Open the device details on the terminal and point the camera at its QR code.", scanHint(ScanPurpose.DeviceId))
        assertEquals(ScanPurpose.DeviceId, ScanPurpose.fromArg(ScanPurpose.DeviceId.arg))
    }

    private fun start(): ScanViewModel {
        val vm = ScanViewModel(core, platform, ScanPurpose.DeviceId)
        rule.setScreen(platform = platform) {
            val s by vm.state.collectAsState()
            ScanScreen(
                s,
                ScanActions(
                    onBack = {}, onGrant = vm::grant, onOpenSettings = vm::openSettings, onTogglePaste = vm::togglePaste,
                    onPasteText = vm::pasteText, onPasteFromClipboard = vm::pasteFromClipboard, onUsePasted = vm::usePasted,
                    onContinue = { vm.finish() }, onLuma = vm::onLuma,
                ),
            )
        }
        return vm
    }

    private fun frame() = platform.camera.onLuma!!.invoke(1, 1, ByteArray(1))

    @Test fun a_scanned_id_is_published_as_plain_text_without_asking_the_core() {
        start()
        core.events += ScanEvent.Frame("sshub:dev:1:723ED85F0566DF0C".toByteArray())
        frame()
        rule.waitForIdle()
        assertTrue("the core knows nothing of this payload", core.opened.isEmpty())
        rule.onNodeWithText("Continue").performClick()
        val r = ScanResults.take(ScanPurpose.DeviceId)
        assertEquals("723ed85f0566df0c", (r?.opened as? DeviceIdScanned)?.id)
    }

    @Test fun a_wrong_payload_says_so_and_scanning_goes_on() {
        start()
        core.events += ScanEvent.Frame("https://example.org".toByteArray())
        frame()
        rule.waitForIdle()
        rule.onNodeWithText("That is not a device id code from SSHub.").assertExists()
        rule.onNodeWithText("Continue").assertDoesNotExist()
        core.events += ScanEvent.Frame("sshub:dev:1:723ed85f0566df0c".toByteArray())
        frame()
        rule.waitForIdle()
        rule.onNodeWithText("Continue").assertExists()
    }
}
