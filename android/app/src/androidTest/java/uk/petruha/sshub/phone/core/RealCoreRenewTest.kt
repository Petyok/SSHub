package uk.petruha.sshub.phone.core

import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uk.petruha.sshub.phone.core.uniffi.UniffiPhoneCore
import uk.petruha.sshub.phone.platform.SealedStateStore
import java.io.File
import java.util.UUID

/** Kind 11 through the REAL native core: the adapter maps `Opened::CertRenew`, keeps it under its token and lists it as pending. */
class RealCoreRenewTest {
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private val now = 1_793_007_660L

    private fun core(): UniffiPhoneCore {
        val dir = File(ctx.cacheDir, "renew-" + UUID.randomUUID()).also { it.mkdirs() }
        return UniffiPhoneCore(SealedStateStore(dir)).also { it.blocked = { false }; it.vault = PassVault(); runBlocking { it.unlock(ByteArray(32) { 0x42 }, now) } }
    }

    /** SWR1 || version 1 || kind 11 || cert_id(32) || expiry i64 BE || 0 evidence entries. */
    private fun renewFrame(): ByteArray =
        "SWR1".toByteArray() + byteArrayOf(0, 1, 11) + ByteArray(32) { 7 } + java.nio.ByteBuffer.allocate(8).putLong(now + 86_400).array() + byteArrayOf(0)

    @Test fun a_renewal_for_a_certificate_this_phone_did_not_issue_is_a_red_card_without_approve() {
        val c = core()
        val opened = runBlocking { c.openFrame(renewFrame(), now) }
        val v = (opened as OpenedTrustRenew).view
        assertTrue("refused", v.refusal != null)
        assertTrue("red", v.red)
        assertEquals("no evidence came", null, v.evidenceAgeSecs)
        assertNotNull(runBlocking { c.trustRenew(v.token) })
    }
}
