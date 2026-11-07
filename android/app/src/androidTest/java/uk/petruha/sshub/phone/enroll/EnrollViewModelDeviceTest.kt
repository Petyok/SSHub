package uk.petruha.sshub.phone.enroll

import androidx.navigation.NavHostController
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uk.petruha.sshub.phone.core.BleDevice
import uk.petruha.sshub.phone.fake.FakeBleChannel
import uk.petruha.sshub.phone.fake.FakePhoneCore
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.platform.BleChannelState
import uk.petruha.sshub.phone.platform.BleScanError
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollViewModel

/** Device channel wiring of the enrollment view model: scan error, why a link ended, the terminal's refusal sentence. */
class EnrollViewModelDeviceTest {
    private val inst = InstrumentationRegistry.getInstrumentation()
    private val ctx = inst.targetContext

    private fun onMain(block: () -> Unit) { inst.runOnMainSync(block); inst.waitForIdleSync() }

    private fun waitFor(what: String, cond: () -> Boolean) {
        val end = System.currentTimeMillis() + 5_000
        while (!cond() && System.currentTimeMillis() < end) Thread.sleep(25)
        assertTrue(what, cond())
    }

    private fun vm(p: FakePlatform): EnrollViewModel {
        lateinit var v: EnrollViewModel
        onMain { v = EnrollViewModel(FakePhoneCore(), p, NavHostController(ctx), "pixel-8") }
        return v
    }

    @Test fun a_scan_error_is_carried_into_the_device_list_state() {
        val p = FakePlatform()
        val v = vm(p)
        p.ble.scanErr.value = BleScanError(2, "Bluetooth failed to scan. Turn Bluetooth off and on.")
        waitFor("scan error shown") { v.ui.value.flow.btScanError == "Bluetooth failed to scan. Turn Bluetooth off and on." }
        p.ble.scanErr.value = null
        waitFor("scan error cleared") { v.ui.value.flow.btScanError == null }
    }

    @Test fun a_link_that_ends_shows_its_reason() {
        val p = FakePlatform()
        val ch = FakeBleChannel().also { it.st.value = BleChannelState.Closed("The terminal ended the session.") }
        p.ble.channel = ch
        p.ble.found.value = listOf(BleDevice("AA:BB", "SSHub-7K2Q"))
        val v = vm(p)
        onMain { v.btSetTag("7K2Q"); v.btStart() }
        waitFor("device listed") { v.ui.value.flow.btDevices.isNotEmpty() }
        onMain { v.btSelect(0); v.btPair() }
        // the fake's `incoming` completes at once: the session is over and the state says why
        waitFor("reason shown") { v.ui.value.flow.btLinkEnded == "The terminal ended the session." }
        assertEquals("The terminal ended the session.", v.ui.value.flow.note)
        assertNull(v.ui.value.flow.btScanError)
    }
}
