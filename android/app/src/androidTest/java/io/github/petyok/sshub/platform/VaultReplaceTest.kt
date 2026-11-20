package io.github.petyok.sshub.platform

import androidx.biometric.BiometricPrompt
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Runnable
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import io.github.petyok.sshub.core.VaultResult
import io.github.petyok.sshub.core.VaultState
import java.io.File
import java.security.KeyStore
import java.util.UUID
import java.util.concurrent.atomic.AtomicInteger
import kotlin.coroutines.CoroutineContext

class VaultReplaceTest {
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private lateinit var dir: File
    private val alias = "sshub.phone.test." + UUID.randomUUID()
    private val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    @Before fun setUp() { dir = File(ctx.cacheDir, "vault-" + UUID.randomUUID()).also { it.mkdirs() } }
    @After fun tearDown() { runBlocking { vault().erase() }; dir.deleteRecursively() }

    private fun vault(io: CoroutineDispatcher = Dispatchers.IO, prompt: VaultPrompt? = null) =
        AndroidKeyVault(ctx, dir = dir, alias = alias, requireAuth = false, prompt = prompt ?: { _, _, c -> c }, io = io)

    private fun secret(b: Int) = ByteArray(32) { b.toByte() }
    private fun unlocked(v: AndroidKeyVault) = (runBlocking { v.unlock("Approve", "") } as VaultResult.Secret).bytes

    @Test fun a_canceled_or_locked_out_replacement_keeps_the_old_secret() = runBlocking {
        vault().store(secret(1))
        val blobBefore = File(dir, AndroidKeyVault.FILE).readBytes()
        assertEquals(VaultResult.Canceled, vault { _, _, _ -> null }.store(secret(2)))
        assertEquals(VaultResult.LockedOut, vault { _, _, _ -> throw PromptLockedOut() }.store(secret(2)))
        assertEquals(VaultResult.Failed("busy"), vault { _, _, _ -> throw PromptFailed(BiometricPrompt.ERROR_HW_UNAVAILABLE, "busy") }.store(secret(2)))
        assertArrayEquals(blobBefore, File(dir, AndroidKeyVault.FILE).readBytes())
        assertArrayEquals(secret(1), unlocked(vault()))
        assertTrue(ks.containsAlias(alias))
        assertFalse(ks.containsAlias("$alias.b"))
        assertFalse(File(dir, "${AndroidKeyVault.FILE}.new").exists())
    }

    @Test fun a_successful_replacement_swaps_the_key_and_drops_the_old_one() = runBlocking {
        vault().store(secret(1))
        assertEquals(VaultResult.Stored, vault().store(secret(2)))
        assertArrayEquals(secret(2), unlocked(vault()))
        assertFalse(ks.containsAlias(alias))
        assertTrue(ks.containsAlias("$alias.b"))
        // and back again, the alternation works both ways
        assertEquals(VaultResult.Stored, vault().store(secret(3)))
        assertArrayEquals(secret(3), unlocked(vault()))
        assertTrue(ks.containsAlias(alias))
        assertFalse(ks.containsAlias("$alias.b"))
    }

    @Test fun a_missing_key_with_a_blob_is_invalidated_not_no_key() = runBlocking {
        vault().store(secret(1))
        ks.deleteEntry(alias)
        val v = vault()
        assertEquals(VaultState.Invalidated, v.state.value)
        assertEquals(VaultResult.Invalidated, v.unlock("Approve", ""))
        assertEquals(VaultState.Invalidated, v.state.value)
    }

    @Test fun no_blob_is_no_key() = runBlocking {
        val v = vault()
        assertEquals(VaultState.NoKey, v.state.value)
        assertEquals(VaultResult.Failed("no key"), v.unlock("Approve", ""))
    }

    @Test fun a_prompt_failure_is_reported_with_the_system_text() = runBlocking {
        vault().store(secret(1))
        val v = vault { _, _, _ -> throw PromptFailed(BiometricPrompt.ERROR_TIMEOUT, "Timed out") }
        assertEquals(VaultResult.Failed("Timed out"), v.unlock("Approve", ""))
        val nolock = vault { _, _, _ -> throw PromptFailed(BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL, "none") }
        assertEquals(VaultResult.NoSecureLock, nolock.unlock("Approve", ""))
    }

    @Test fun prompt_error_codes_map_to_their_results() {
        listOf(BiometricPrompt.ERROR_USER_CANCELED, BiometricPrompt.ERROR_NEGATIVE_BUTTON, BiometricPrompt.ERROR_CANCELED)
            .forEach { assertEquals(VaultResult.Canceled, vaultResultForPromptError(it, "x")) }
        listOf(BiometricPrompt.ERROR_LOCKOUT, BiometricPrompt.ERROR_LOCKOUT_PERMANENT)
            .forEach { assertEquals(VaultResult.LockedOut, vaultResultForPromptError(it, "x")) }
        assertEquals(VaultResult.NoSecureLock, vaultResultForPromptError(BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL, "x"))
        listOf(BiometricPrompt.ERROR_HW_UNAVAILABLE, BiometricPrompt.ERROR_NO_SPACE, BiometricPrompt.ERROR_TIMEOUT, BiometricPrompt.ERROR_VENDOR)
            .forEach { assertEquals(VaultResult.Failed("why"), vaultResultForPromptError(it, "why")) }
    }

    @Test fun store_unlock_and_erase_run_on_the_io_dispatcher() = runBlocking {
        val hops = AtomicInteger()
        val recording = object : CoroutineDispatcher() {
            override fun dispatch(context: CoroutineContext, block: Runnable) { hops.incrementAndGet(); Dispatchers.IO.dispatch(context, block) }
        }
        val v = vault(io = recording)
        v.store(secret(1)); assertTrue(hops.get() >= 1)
        val afterStore = hops.get()
        v.unlock("Approve", ""); assertTrue(hops.get() > afterStore)
        val afterUnlock = hops.get()
        v.erase(); assertTrue(hops.get() > afterUnlock)
    }
}
