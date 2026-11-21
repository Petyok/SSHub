package io.github.petyok.sshub.core

import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertTrue
import org.junit.Test
import io.github.petyok.sshub.core.uniffi.UniffiPhoneCore
import io.github.petyok.sshub.platform.SealedStateStore
import java.io.File
import java.nio.ByteBuffer
import java.util.UUID

/** Kind 12 through the REAL native core: an admission frame that names another phone's key is refused, never offered for signing. */
class RealCoreHandoverTest {
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private val now = 1_793_007_660L

    private fun core(): UniffiPhoneCore {
        val dir = File(ctx.cacheDir, "handover-" + UUID.randomUUID()).also { it.mkdirs() }
        return UniffiPhoneCore(SealedStateStore(dir)).also { it.blocked = { false }; it.vault = PassVault(); runBlocking { it.unlock(ByteArray(32) { 0x42 }, now) } }
    }

    /** SWR1 || v1 || kind 12 || ed25519 || x25519 || mlkem_len u16 || mlkem || name_len u8 || name || terminal(8) || 0x01 || op_hash(32) || op_lamport u64. */
    private fun admissionFrame(): ByteArray {
        val mlkem = ByteArray(1184) { 3 }
        val name = "pixel-9".toByteArray()
        return "SWR1".toByteArray() + byteArrayOf(0, 1, 12) + ByteArray(32) { 1 } + ByteArray(32) { 2 } +
            ByteBuffer.allocate(2).putShort(mlkem.size.toShort()).array() + mlkem + byteArrayOf(name.size.toByte()) + name + ByteArray(8) { 9 } +
            byteArrayOf(1) + ByteArray(32) { 5 } + ByteBuffer.allocate(8).putLong(7).array()
    }

    @Test fun an_admission_for_another_phone_is_a_refusal_not_a_signing_card() {
        val opened = runBlocking { core().openFrame(admissionFrame(), now) }
        assertTrue("got $opened", opened is OpenedRefused || opened is Opened.Ignored)
        assertTrue(opened !is OpenedPhoneHandover)
    }
}
