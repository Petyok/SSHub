package uk.petruha.sshub.phone.platform

import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import uk.petruha.sshub.phone.core.VaultResult
import uk.petruha.sshub.phone.core.VaultState
import java.io.File
import java.util.UUID

class VaultAndAtomicTest {
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private lateinit var dir: File
    private val alias = "sshub.phone.test." + UUID.randomUUID()

    @Before fun setUp() { dir = File(ctx.cacheDir, "vault-" + UUID.randomUUID()).also { it.mkdirs() } }
    @After fun tearDown() { runBlocking { vault().erase() }; dir.deleteRecursively() }

    private fun vault() = AndroidKeyVault(ctx, dir = dir, alias = alias, requireAuth = false)

    @Test fun vault_roundtrip_and_reopen() = runBlocking {
        val secret = ByteArray(32) { it.toByte() }
        val v = vault()
        assertEquals(VaultState.NoKey, v.state.value)
        assertEquals(VaultResult.Stored, v.store(secret))
        assertEquals(VaultState.Unlocked, v.state.value)
        v.lock()
        assertEquals(VaultState.Locked, v.state.value)
        val r = v.unlock("Approve", "ab12cd34") as VaultResult.Secret
        assertArrayEquals(secret, r.bytes)
        // A new process sees the key and the file: Locked, same secret.
        val again = vault()
        assertEquals(VaultState.Locked, again.state.value)
        assertArrayEquals(secret, (again.unlock("Approve", "") as VaultResult.Secret).bytes)
    }

    @Test fun vault_file_layout_and_tamper() = runBlocking {
        val v = vault()
        v.store(ByteArray(32) { 7 })
        val f = File(dir, AndroidKeyVault.FILE)
        val blob = f.readBytes()
        assertEquals(1 + 12 + 32 + 16, blob.size)
        assertEquals(1, blob[0].toInt())
        blob[blob.size - 1] = (blob[blob.size - 1].toInt() xor 1).toByte()
        f.writeBytes(blob)
        assertTrue(v.unlock("Approve", "") is VaultResult.Failed)
    }

    @Test fun vault_erase_removes_key_and_file() = runBlocking {
        val v = vault()
        v.store(ByteArray(32))
        v.erase()
        assertEquals(VaultState.NoKey, v.state.value)
        assertFalse(File(dir, AndroidKeyVault.FILE).exists())
        assertTrue(v.unlock("Approve", "") is VaultResult.Failed)
    }

    @Test fun cancelled_prompt_leaves_no_key() = runBlocking {
        val v = AndroidKeyVault(ctx, dir = dir, alias = alias, requireAuth = false, prompt = { _, _, _ -> null })
        assertEquals(VaultResult.Canceled, v.store(ByteArray(32)))
        assertEquals(VaultState.NoKey, v.state.value)
    }

    @Test fun lockout_is_reported() = runBlocking {
        val ok = vault()
        ok.store(ByteArray(32))
        val locked = AndroidKeyVault(ctx, dir = dir, alias = alias, requireAuth = false, prompt = { _, _, _ -> throw PromptLockedOut() })
        assertEquals(VaultResult.LockedOut, locked.unlock("Approve", ""))
    }

    @Test fun atomic_write_survives_torn_temp() {
        val f = File(dir, "state.bin")
        atomicWrite(f, byteArrayOf(1, 2, 3))
        File(dir, "state.bin.tmp").writeBytes(byteArrayOf(9)) // a crash left half a write
        assertArrayEquals(byteArrayOf(1, 2, 3), atomicRead(f))
        atomicWrite(f, byteArrayOf(4, 5))
        assertArrayEquals(byteArrayOf(4, 5), atomicRead(f))
        assertFalse(File(dir, "state.bin.tmp").exists())
    }

    @Test fun sealed_state_store() {
        val s = SealedStateStore(dir)
        assertNull(s.read())
        s.write(byteArrayOf(1))
        s.write(byteArrayOf(2, 2))
        assertArrayEquals(byteArrayOf(2, 2), s.read())
        s.delete()
        assertNull(s.read())
    }
}
