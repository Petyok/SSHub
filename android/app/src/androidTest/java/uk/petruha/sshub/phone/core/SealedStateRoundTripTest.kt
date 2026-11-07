package uk.petruha.sshub.phone.core

import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uk.petruha.sshub.phone.core.uniffi.UniffiPhoneCore
import uk.petruha.sshub.phone.platform.SealedStateStore
import java.io.File
import java.util.UUID

/** A lock followed by an unlock must not turn the saved state into an empty one (the round-2 blocker). Real core, real store dir. */
class SealedStateRoundTripTest {
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private val fx = JSONObject(InstrumentationRegistry.getInstrumentation().context.assets.open("notices.json").bufferedReader().readText())
    private val now = fx.getLong("now")
    private fun hex(s: String) = ByteArray(s.length / 2) { s.substring(2 * it, 2 * it + 2).toInt(16).toByte() }
    private val secret get() = hex(fx.getJSONObject("phone").getString("secret_hex"))
    private fun case(name: String): JSONObject = fx.getJSONArray("cases").let { a -> (0 until a.length()).map { a.getJSONObject(it) }.first { it.getString("name") == name } }
    private fun newDir() = File(ctx.cacheDir, "rt-" + UUID.randomUUID()).also { it.mkdirs() }

    private fun core(dir: File) = UniffiPhoneCore(SealedStateStore(dir)).also { it.blocked = { false }; it.vault = PassVault() }

    private class Snapshot(val deviceId: String, val epoch: Long, val headSeq: Long?, val certs: List<String>, val deviceIds: List<String>)

    private fun snapshot(c: UniffiPhoneCore): Snapshot = runBlocking {
        val s = c.status(now)
        Snapshot(s.deviceId, s.epoch, c.thisPhone(now).headSeq, c.issuedCerts(now).map { it.certId }, c.ring(now)?.devices?.map { it.id }.orEmpty())
    }

    private fun assertSame(what: String, a: Snapshot, b: Snapshot) {
        assertEquals("$what: device id", a.deviceId, b.deviceId)
        assertEquals("$what: ring epoch", a.epoch, b.epoch)
        assertEquals("$what: head seq", a.headSeq, b.headSeq)
        assertEquals("$what: issued certificates", a.certs, b.certs)
        assertEquals("$what: ring", a.deviceIds, b.deviceIds)
    }

    @Test fun lock_then_unlock_keeps_the_saved_state() {
        val dir = newDir()
        val c = core(dir)
        runBlocking { c.unlock(secret, now) }
        val emptyLen = File(dir, "state.bin").length()

        // A state change: a trust request approved issues a certificate, which the sealed state must carry.
        val rq = runBlocking { c.openFrame(hex(case("trust-request").getString("frame_hex")), now) } as OpenedTrustRequest
        runBlocking { c.decideTrust(rq.view.token, TrustDecision.Approve(case("trust-request").getJSONObject("approve").getString("typed_id")), now) }
        val before = snapshot(c)
        assertTrue("the approval issued a certificate", before.certs.isNotEmpty())
        val savedLen = File(dir, "state.bin").length()
        assertTrue("the saved state is bigger than an empty one", savedLen > emptyLen)

        c.lock()
        // lock() is queued on the core thread ahead of this call: it is refused because the core is locked
        val e = runCatching { runBlocking { c.status(now) } }.exceptionOrNull() as? CoreException
        assertEquals(CoreException.Code.Refused, e?.code)

        val status = runBlocking { c.unlock(secret, now) }
        assertFalse("the state was read, not lost", status.stateLost)
        assertFalse(c.stateLost())
        assertSame("after unlock", before, snapshot(c))
        assertTrue("state.bin was not replaced by an empty state", File(dir, "state.bin").length() >= savedLen)

        // The bytes on disk, opened by a NEW core with the same secret, carry the same state.
        val fresh = core(dir)
        assertFalse(runBlocking { fresh.unlock(secret, now) }.stateLost)
        assertSame("a new core over the same dir", before, snapshot(fresh))
    }
}
