package uk.petruha.sshub.phone.core

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
import uk.petruha.sshub.phone.core.uniffi.UniffiPhoneCore
import uk.petruha.sshub.phone.platform.SealedStateStore
import java.io.File
import java.util.UUID

/** The real native library on the device: the adapter against the public vector of assets/companion/test/fixtures/challenges.json. */
class UniffiCoreTest {
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private val now = 1_893_452_400L
    private lateinit var dir: File
    private lateinit var core: UniffiPhoneCore

    @Before fun setUp() {
        dir = File(ctx.cacheDir, "core-" + UUID.randomUUID()).also { it.mkdirs() }
        core = UniffiPhoneCore(SealedStateStore(dir))
    }

    @After fun tearDown() { dir.deleteRecursively() }

    private fun fixedSecret() = ByteArray(32) { 0x42 }

    @Test fun createSecret_is_32_random_bytes() {
        val a = core.createSecret()
        assertEquals(32, a.size)
        assertFalse(a.contentEquals(core.createSecret()))
    }

    @Test fun unlock_gives_the_fixture_device_id_and_a_never_enrolled_status() {
        val s = runBlocking { core.unlock(fixedSecret(), now) }
        assertEquals("ee8a1d5cf97016a7", s.deviceId)
        assertFalse(s.enrolled)
        assertEquals(s, runBlocking { core.status(now) })
        assertNull(runBlocking { core.ring(now) })
        assertTrue(File(dir, "state.bin").isFile)
    }

    @Test fun the_state_survives_a_new_core_over_the_same_store() {
        runBlocking { core.unlock(fixedSecret(), now) }
        val again = UniffiPhoneCore(SealedStateStore(dir))
        assertEquals("ee8a1d5cf97016a7", runBlocking { again.unlock(fixedSecret(), now) }.deviceId)
        assertFalse(runBlocking { again.status(now) }.headPending)
    }

    @Test fun backup_words_are_24_and_restore_gives_the_same_secret() {
        runBlocking { core.unlock(fixedSecret(), now) }
        val chars = runBlocking { core.backupWords() }
        val words = chars.map { String(it) }
        chars.forEach { it.fill('\u0000') }
        assertEquals(24, words.size)
        assertArrayEquals(fixedSecret(), runBlocking { core.restoreFromWords(words) })
    }

    @Test fun a_restored_phone_has_its_head_pending() {
        runBlocking { core.unlock(fixedSecret(), now) }
        val secret = runBlocking { core.restoreFromWords(core.backupWords().map { String(it) }) }
        val s = runBlocking { core.unlock(secret, now) }
        assertTrue(s.headPending)
        assertEquals("ee8a1d5cf97016a7", s.deviceId)
    }

    @Test fun a_wrong_word_is_a_malformed_error() {
        runBlocking { core.unlock(fixedSecret(), now) }
        val e = runCatching { runBlocking { core.restoreFromWords(List(24) { "zzzz" }) } }.exceptionOrNull() as CoreException
        assertEquals(CoreException.Code.Malformed, e.code)
    }

    @Test fun validateWord_uses_the_core_word_list() {
        val words = runBlocking { runBlocking { core.unlock(fixedSecret(), now) }; runBlocking { core.backupWords() }.map { String(it) } }
        assertTrue(words.all { core.validateWord(it) == WordState.Ok })
        assertEquals(WordState.Bad, core.validateWord("zzzz"))
        assertEquals(WordState.Empty, core.validateWord("  "))
    }

    @Test fun a_fresh_phone_has_no_name_and_ring_epoch_zero() {
        val s = runBlocking { core.unlock(fixedSecret(), now) }
        assertEquals("", s.deviceName)
        assertEquals(0L, s.epoch)
    }

    @Test fun garbage_is_a_value_not_an_exception() = runBlocking {
        runBlocking { core.unlock(fixedSecret(), now) }
        val o = core.openFrame(byteArrayOf(1, 2, 3), now)
        assertTrue(o is OpenedRefused || o is Opened.Ignored)
        assertTrue(core.pending(now).isEmpty())
    }

    @Test fun a_locked_core_is_an_error_not_a_crash() = runBlocking {
        val e = runCatching { core.openFrame(byteArrayOf(1, 2, 3), now) }.exceptionOrNull()
        assertTrue(e is CoreException)
    }

    @Test fun frameFromText_of_text_that_is_not_base64_is_null() {
        assertNull(runBlocking { core.frameFromText("!!") })
        assertArrayEquals("hi".toByteArray(), runBlocking { core.frameFromText(" aGk= \n") })
    }

    @Test fun lock_wipes_the_unlocked_state() {
        runBlocking { core.unlock(fixedSecret(), now) }
        core.lock()
        val e = runCatching { runBlocking { core.status(now) } }.exceptionOrNull() as CoreException
        assertEquals(CoreException.Code.Refused, e.code)
    }
}
