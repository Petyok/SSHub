package io.github.petyok.sshub.core

import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import io.github.petyok.sshub.core.uniffi.UniffiPhoneCore
import io.github.petyok.sshub.platform.SealedStateStore
import java.io.File
import java.util.UUID

/** Review round 2 on the real adapter: persist failure, pending queue, fail closed, lost copies, kept outcomes. */
class AdapterRound2Test {
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private val fixtures = JSONObject(InstrumentationRegistry.getInstrumentation().context.assets.open("challenges.json").bufferedReader().readText())
    private val now = fixtures.getLong("now")
    private val secret = ByteArray(32) { 0x42 }
    private fun hex(s: String) = ByteArray(s.length / 2) { s.substring(2 * it, 2 * it + 2).toInt(16).toByte() }
    private fun newDir() = File(ctx.cacheDir, "r2-" + UUID.randomUUID()).also { it.mkdirs() }

    private fun core(vault: KeyVault? = PassVault(), dir: File = newDir(), blocked: Boolean = true): UniffiPhoneCore =
        UniffiPhoneCore(SealedStateStore(dir)).also {
            if (blocked) it.blocked = { false }
            it.vault = vault
            runBlocking { it.unlock(secret, now) }
        }

    private fun okFrames(): List<ByteArray> = fixtures.getJSONArray("challenges").let { a ->
        (0 until a.length()).map { a.getJSONObject(it) }.filter { it.getString("expect") == "ok" }.map { hex(it.getString("frame_hex")) }
    }

    private fun open(c: UniffiPhoneCore, frame: ByteArray = okFrames()[0]): ReleaseView = (runBlocking { c.openFrame(frame, now) } as OpenedRelease).view
    private fun approve(v: ReleaseView, shortCode: Boolean = false, byHand: Boolean = false) =
        ReleaseDecision.Approve(channels = ReleaseChannels(qr = true), shortCode = shortCode, byHand = byHand, typedId = v.mustType)

    private fun refusal(call: suspend () -> Any?): CoreException = try {
        runBlocking { call() }
        throw AssertionError("expected a CoreException")
    } catch (e: CoreException) { e }

    // ------------------------------------------------------------------------------ C-M1: a failed write locks everything

    @Test fun a_failed_state_write_locks_the_phone_and_discards_the_request() {
        val dir = newDir()
        val vault = PassVault()
        val c = core(vault, dir)
        val v = open(c)
        // The temp file the store writes first is a directory: the write throws IOException.
        File(dir, "state.bin.tmp").mkdirs()
        val e = refusal { c.decide(v.token, approve(v), now) }
        assertEquals(CoreException.Code.Internal, e.code)
        assertEquals("The phone state could not be saved; the request was discarded.", e.message)
        assertFalse(e.message!!.contains("Nothing was signed"))
        assertTrue("the system lock was locked", vault.locks >= 1)
        assertNull("the request is gone", c.pendingView(v.token))
        assertEquals("the core is locked", CoreException.Code.Refused, refusal { c.status(now) }.code)
    }

    @Test fun a_failed_write_drops_the_in_memory_state_so_the_discarded_op_never_comes_back() {
        val dir = newDir()
        val c = core(PassVault(), dir)
        val before = runBlocking { c.exportOps(now) }.total
        val v = open(c)
        File(dir, "state.bin.tmp").mkdirs()
        refusal { c.decide(v.token, approve(v), now) }
        File(dir, "state.bin.tmp").deleteRecursively()
        runBlocking { c.unlock(secret, now) }
        assertEquals("the op the user was told was discarded is not in the chain after lock and unlock", before, runBlocking { c.exportOps(now) }.total)
    }

    // ------------------------------------------------------------------------------ R3-M1: lock and unlock keep the stop

    @Test fun lock_then_unlock_keeps_the_state_lost_stop_and_never_overwrites_the_unreadable_file() {
        val dir = newDir()
        val junk = ByteArray(200) { 7 }
        File(dir, "state.bin").writeBytes(junk)
        val c = UniffiPhoneCore(SealedStateStore(dir)).also { it.blocked = { false }; it.vault = PassVault() }
        assertTrue(runBlocking { c.unlock(secret, now) }.stateLost)
        assertTrue(c.stateLost())
        c.lock()
        refusal { c.status(now) } // queued behind the lock: refused once it ran
        runBlocking { c.unlock(secret, now) } // the core now opens its own sealed (empty) state and says stateLost = false
        assertTrue("the stop stays until confirmStateLost", c.stateLost())
        assertArrayEquals("the unreadable bytes were not overwritten", junk, File(dir, "state.bin").readBytes())
        runBlocking { c.confirmStateLost() }
        assertFalse(c.stateLost())
        assertFalse(junk.contentEquals(File(dir, "state.bin").readBytes()))
    }

    // ------------------------------------------------------------------------------ L3, N-L6: the cache follows the core's queue

    @Test fun garbage_frames_keep_the_pending_release_and_a_new_frame_replaces_it_as_the_core_does() {
        val c = core()
        val frames = okFrames()
        val a = open(c, frames[0])
        for (junk in listOf(byteArrayOf(0x53, 0x57, 0x52, 0x31, 0, 1, 1, 9), byteArrayOf(0x53, 0x57, 0x52, 0x31, 9, 9, 9, 9, 9, 9, 9), ByteArray(40) { (it * 7 + 3).toByte() })) {
            runCatching { runBlocking { c.openFrame(junk, now) } }
            val list = c.pending(now)
            assertEquals("still pending after junk", 1, list.size)
            assertTrue(list[0].route.endsWith("/" + a.token))
            assertNotNull(c.pendingView(a.token))
        }
        val b = open(c, frames[1])
        assertTrue("a different request", b.token != a.token)
        // The core holds one pending request: the second replaced the first, and the cache did the same.
        assertNull(c.pendingView(a.token))
        assertEquals(listOf(b.token), c.pending(now).map { it.route.substringAfterLast('/').toLong() })
        assertEquals(CoreException.Code.Malformed, refusal { c.decide(a.token, approve(a), now) }.code)
        assertNotNull(runBlocking { c.decide(b.token, approve(b), now) }.outgoing)
    }

    @Test fun a_wrong_typed_id_and_a_junk_frame_keep_the_request_and_the_right_id_still_signs() {
        val c = core()
        val a = open(c)
        if (a.mustType == null) return // a met requester has no typed id to get wrong
        refusal { c.decide(a.token, ReleaseDecision.Approve(channels = ReleaseChannels(qr = true), typedId = "0000000000000000"), now) }
        runCatching { runBlocking { c.openFrame(byteArrayOf(0x53, 0x57, 0x52, 0x31, 0, 1, 1, 9), now) } }
        assertNotNull(c.pendingView(a.token))
        assertNotNull(runBlocking { c.decide(a.token, approve(a), now) }.outgoing)
    }

    // ------------------------------------------------------------------------------ N-L1: fail closed

    @Test fun a_bare_core_refuses_to_open_frames_and_to_approve() {
        val noBlock = core(PassVault(), blocked = false)
        assertEquals(CoreException.Code.Refused, refusal { noBlock.openFrame(okFrames()[0], now) }.code)

        val noVault = core(vault = null)
        val v = open(noVault)
        val e = refusal { noVault.decide(v.token, approve(v), now) }
        assertEquals(CoreException.Code.Refused, e.code)
        assertEquals("The system lock is not attached. Nothing was signed.", e.message)
        assertNotNull("nothing was consumed", noVault.pendingView(v.token))
    }

    // ------------------------------------------------------------------------------ N-L4: a lost state

    @Test fun declare_short_code_dead_is_refused_while_the_state_is_lost_and_changes_nothing() {
        val dir = newDir()
        val junk = ByteArray(200) { 7 }
        File(dir, "state.bin").writeBytes(junk)
        val c = UniffiPhoneCore(SealedStateStore(dir)).also { it.blocked = { false }; it.vault = PassVault() }
        assertTrue(runBlocking { c.unlock(secret, now) }.stateLost)
        assertEquals(CoreException.Code.Refused, refusal { c.declareShortCodeDead("aabbccdd") }.code)
        assertArrayEquals(junk, File(dir, "state.bin").readBytes())
    }

    @Test fun two_lost_events_keep_both_copies() {
        val dir = newDir()
        val store = SealedStateStore(dir)
        val one = ByteArray(50) { 1 }
        val two = ByteArray(60) { 2 }
        File(dir, "state.bin").writeBytes(one); store.keepAsLost()
        File(dir, "state.bin").writeBytes(two); store.keepAsLost()
        assertArrayEquals(one, File(dir, "state.bin.lost").readBytes())
        assertArrayEquals(two, File(dir, "state.bin.lost.1").readBytes())
    }

    // ------------------------------------------------------------------------------ N1, N-L5, N12: the kept outcome

    @Test fun the_kept_outcome_has_no_short_code_and_no_by_hand_words_and_lock_empties_it() {
        val c = core()
        val v = open(c)
        val out = runBlocking { c.decide(v.token, approve(v, shortCode = true, byHand = true), now) }
        assertNotNull("decide itself returns the by-hand words", out.byHand)
        val kept = c.lastOutcome(v.token)!!
        assertNull(kept.shortCode)
        assertNull(kept.byHand)
        assertEquals(out.outgoing, kept.outgoing)
        assertEquals(1, c.keptOutcomeCount())
        c.lock()
        assertEquals(0, c.keptOutcomeCount())
    }

    @Test fun at_most_the_newest_eight_outcomes_are_kept() {
        val c = core()
        val frames = okFrames()
        var done = 0
        var last = -1L
        for (i in 0 until 20) {
            val v = open(c, frames[i % frames.size])
            if (runCatching { runBlocking { c.decide(v.token, approve(v), now) } }.isSuccess) { done++; last = v.token }
            assertTrue(c.keptOutcomeCount() <= 8)
        }
        assertTrue("more than eight approvals ran: $done", done > 8)
        assertEquals(8, c.keptOutcomeCount())
        assertNotNull("the newest is kept", c.lastOutcome(last))
    }
}
