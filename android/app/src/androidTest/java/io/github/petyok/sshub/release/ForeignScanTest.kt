package io.github.petyok.sshub.release

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import io.github.petyok.sshub.ui.screens.home.parseDeviceIdPayload

/** The device id code read by the camera arrives as the bytes of a small foreign document. */
class ForeignScanTest {
    @Test fun the_payload_parses_only_in_the_agreed_form() {
        assertEquals("723ed85f0566df0c", parseDeviceIdPayload("sshub:dev:1:723ED85F0566DF0C".toByteArray()))
        assertNull(parseDeviceIdPayload("sshub:dev:2:723ed85f0566df0c".toByteArray()))
        assertNull(parseDeviceIdPayload("hello".toByteArray()))
    }
}
