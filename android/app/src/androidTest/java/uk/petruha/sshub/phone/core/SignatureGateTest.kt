package uk.petruha.sshub.phone.core

import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uk.petruha.sshub.phone.core.uniffi.UniffiPhoneCore
import uk.petruha.sshub.phone.platform.SealedStateStore
import java.io.File
import java.util.UUID

/** A vault that only counts the prompts it is asked for (platform H5, flows H6, security M5). */
private class CountingVault(var result: VaultResult? = null) : KeyVault {
    val prompts = mutableListOf<Pair<String, String>>()
    override val state: StateFlow<VaultState> = MutableStateFlow(VaultState.Unlocked)
    override val strongBox = false
    override suspend fun store(secret: ByteArray) = VaultResult.Stored
    override suspend fun unlock(title: String, subtitle: String): VaultResult { prompts += title to subtitle; return result ?: VaultResult.Secret(ByteArray(32) { 1 }) }
    override suspend fun erase() {}
    override fun lock() {}
}

class SignatureGateTest {
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private val fixtures = JSONObject(InstrumentationRegistry.getInstrumentation().context.assets.open("challenges.json").bufferedReader().readText())
    private val now = fixtures.getLong("now")
    private fun hex(s: String) = ByteArray(s.length / 2) { s.substring(2 * it, 2 * it + 2).toInt(16).toByte() }
    private fun newDir() = File(ctx.cacheDir, "gate-" + UUID.randomUUID()).also { it.mkdirs() }

    private fun core(vault: KeyVault?, dir: File = newDir()): UniffiPhoneCore =
        UniffiPhoneCore(SealedStateStore(dir)).also { it.unlock(ByteArray(32) { 0x42 }, now); it.vault = vault }

    private fun open(c: UniffiPhoneCore): ReleaseView {
        val ch = fixtures.getJSONArray("challenges").getJSONObject(0)
        return (runBlocking { c.openFrame(hex(ch.getString("frame_hex")), now) } as OpenedRelease).view
    }

    private fun approve(v: ReleaseView, byHand: Boolean = false) =
        ReleaseDecision.Approve(channels = ReleaseChannels(qr = true), typedId = v.mustType, byHand = byHand)

    @Test fun one_system_lock_prompt_before_an_approval_and_none_for_a_decline() {
        val vault = CountingVault()
        val c = core(vault)
        val v = open(c)
        runBlocking { c.decide(v.token, ReleaseDecision.Decline, now) }
        assertEquals("a decline is not gated", 0, vault.prompts.size)
        val w = open(c)
        val out = runBlocking { c.decide(w.token, approve(w), now) }
        assertNotNull(out.outgoing)
        assertEquals("exactly one prompt for the approval", 1, vault.prompts.size)
        assertEquals("Approve share release", vault.prompts[0].first)
        assertTrue("the subtitle names the requester", vault.prompts[0].second.contains(w.requesterId))
    }

    @Test fun a_refused_prompt_stops_the_call_before_the_core_is_touched() {
        val vault = CountingVault(VaultResult.Canceled)
        val c = core(vault)
        val v = open(c)
        val e = runCatching { runBlocking { c.decide(v.token, approve(v), now) } }.exceptionOrNull() as CoreException
        assertEquals(CoreException.Code.Refused, e.code)
        assertTrue(e.message!!.contains("Nothing was signed"))
        assertEquals(1, vault.prompts.size)
        assertNotNull("the request is still pending", c.pendingView(v.token))
    }

    @Test fun the_outcome_is_kept_until_the_phone_locks() {
        val c = core(CountingVault())
        val v = open(c)
        assertNull(c.lastOutcome(v.token))
        val out = runBlocking { c.decide(v.token, approve(v), now) }
        assertEquals(out, c.lastOutcome(v.token))
        c.lock()
        assertNull("gone with the lock", c.lastOutcome(v.token))
    }

    @Test fun the_by_hand_choice_reaches_the_core() {
        val c = core(CountingVault())
        val v = open(c)
        val out = runBlocking { c.decide(v.token, approve(v, byHand = true), now) }
        assertNotNull("by-hand words come back when asked for", out.byHand)
        val c2 = core(CountingVault())
        val w = open(c2)
        assertNull(runBlocking { c2.decide(w.token, approve(w), now) }.byHand)
    }

    @Test fun every_mutating_core_method_is_gated_or_listed_as_reviewed() {
        // A new PhoneCoreApi method fails this test until someone decides: gate it (call confirm() in the adapter) or list it with a reason.
        val gated = setOf("decide", "decideTrust", "decideReshare", "revokeCert", "rotateWrapKeys", "decideRecoveryCancel", "enrollDecide", "adoptChain", "neverSigned")
        val reviewedNotGated = setOf(
            // reads
            "backupWords", "enrollTextKind", "exportOps", "frameFromText", "issuedCerts", "lastOutcome", "nameProblem", "openFrame", "pending", "pendingView",
            "recoveryNotice", "recoveryNotices", "resetScan", "reshareCandidate", "ring", "scanGray", "shortCodes", "stateLost", "status", "takeIssuedShortCode",
            "thisPhone", "trustExtend", "trustRequest", "validateWord", "wrapKeyState",
            // state changes that sign nothing: the phone key itself, the lock, the stop screen, the local short-code list
            "createSecret", "restoreFromWords", "unlock", "lock", "confirmStateLost", "declareShortCodeDead",
        )
        val names = PhoneCoreApi::class.java.methods.map { it.name }.filter { !it.startsWith("get") && !it.contains("$") }.toSet()
        assertTrue("gated methods exist: ${gated - names}", names.containsAll(gated))
        val unknown = names - gated - reviewedNotGated
        assertTrue("decide for each new method whether it signs: $unknown", unknown.isEmpty())
    }

    @Test fun an_unreadable_state_is_kept_nothing_is_written_and_everything_is_closed_until_confirmed() {
        val dir = newDir()
        val junk = ByteArray(200) { 7 }
        File(dir, "state.bin").writeBytes(junk)
        val c = UniffiPhoneCore(SealedStateStore(dir))
        val status = c.unlock(ByteArray(32) { 0x42 }, now)
        assertTrue(status.stateLost)
        assertTrue(c.stateLost())
        assertArrayEquals("a copy of the unreadable bytes", junk, File(dir, "state.bin.lost").readBytes())
        assertArrayEquals("nothing was written over it", junk, File(dir, "state.bin").readBytes())
        val e = runCatching { runBlocking { c.openFrame(ByteArray(8), now) } }.exceptionOrNull() as? CoreException
        assertEquals(CoreException.Code.Refused, e?.code)
        c.confirmStateLost()
        assertFalse(c.stateLost())
        assertFalse("the replacement was written", junk.contentEquals(File(dir, "state.bin").readBytes()))
        assertArrayEquals("the old copy stays", junk, File(dir, "state.bin.lost").readBytes())
    }
}
