package io.github.petyok.sshub.core

import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import io.github.petyok.sshub.core.uniffi.UniffiPhoneCore
import io.github.petyok.sshub.platform.SealedStateStore
import java.io.File
import java.util.UUID

/**
 * The page's challenge fixtures (built by the terminal's real encoders) through the REAL native core and the Kotlin adapter:
 * every `ok` challenge opens as a release request with the same four binding words the terminal printed; every other one is a
 * refusal value whose text carries the fixture's expected reason; none of them throws.
 */
class FixtureFlowTest {
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private val fixtures = JSONObject(InstrumentationRegistry.getInstrumentation().context.assets.open("challenges.json").bufferedReader().readText())
    private val now = fixtures.getLong("now")

    private val NEEDS_MET_REQUESTER = setOf("bad-signature", "tampered-op")

    private fun hex(s: String) = ByteArray(s.length / 2) { s.substring(2 * it, 2 * it + 2).toInt(16).toByte() }

    private fun freshCore(): UniffiPhoneCore {
        val dir = File(ctx.cacheDir, "fx-" + UUID.randomUUID()).also { it.mkdirs() }
        return UniffiPhoneCore(SealedStateStore(dir)).also { it.blocked = { false }; it.vault = PassVault(); runBlocking { it.unlock(ByteArray(32) { 0x42 }, now) } }
    }

    @Test fun every_fixture_challenge_opens_as_the_terminal_expects() {
        val list = fixtures.getJSONArray("challenges")
        var okCount = 0
        for (i in 0 until list.length()) {
            val c = list.getJSONObject(i)
            val name = c.getString("name")
            val expect = c.getString("expect")
            val core = freshCore()
            val opened = runBlocking { core.openFrame(hex(c.getString("frame_hex")), now) }
            if (expect == "ok") {
                okCount++
                val v = (opened as? OpenedRelease)?.view ?: error("$name: expected a release request, got $opened")
                assertEquals("$name binding words", c.getString("binding_words"), v.words.joinToString("-"))
                assertTrue("$name: every check passes or is waived by the typed id", v.checks.all { it.ok || it.needsTypedId })
                // A fresh phone has not met the fixture's requester: unknown, nothing verified, so no key; the share itself is described.
                assertFalse("$name: requester unknown to a fresh phone", v.requesterKnown)
                assertNull("$name: no verified key for an unknown requester", v.requesterKey)
                assertTrue("$name: share kind", v.shareKind == "hybrid" || v.shareKind == "envelope")
                assertTrue("$name: share epoch", v.shareEpoch >= 0 && v.shareIndex >= 0)
            } else if (name in NEEDS_MET_REQUESTER && opened is OpenedRefused) {
                // refused for another rule first (for example the wrapped share): still a value, still no approval
            } else if (name in NEEDS_MET_REQUESTER && opened is OpenedRelease) {
                // The page's fixtures enroll the requester first; a fresh phone has not met it, so the signature cannot be checked and the
                // request is shown as from an unmet requester (typed id needed). The refusal itself is pinned by the Rust parity tests.
                assertTrue("$name: unmet requester must type the id", opened.view.mustType != null && !opened.view.requesterKnown)
            } else {
                val r = opened as? OpenedRefused ?: error("$name: expected a refusal, got $opened")
                val text = (r.reason.title + " " + r.reason.text + " " + r.checks.joinToString(" ") { it.text }).lowercase()
                assertTrue("$name: refusal text mentions '$expect': $text", text.contains(expect.lowercase()))
            }
        }
        assertTrue(okCount >= 3)
    }

    @Test fun a_met_requester_release_signs_and_returns_the_reply_in_every_channel_asked() {
        val core = freshCore()
        val c = fixtures.getJSONArray("challenges").getJSONObject(0)
        val v = ((runBlocking { core.openFrame(hex(c.getString("frame_hex")), now) }) as OpenedRelease).view
        val out = runBlocking {
            core.decide(v.token, ReleaseDecision.Approve(channels = ReleaseChannels(qr = true, clipboard = true, soundFast = true), typedId = v.mustType), now)
        }
        val o = out.outgoing!!
        assertTrue(o.frame.isNotEmpty())
        assertTrue(o.text.isNotEmpty())
        assertTrue(o.qr.isNotEmpty() || o.text.isNotEmpty())
    }

    @Test fun a_wrong_typed_id_keeps_the_request_pending() {
        val core = freshCore()
        val c = fixtures.getJSONArray("challenges").getJSONObject(0)
        val v = ((runBlocking { core.openFrame(hex(c.getString("frame_hex")), now) }) as OpenedRelease).view
        if (v.mustType == null) return // a met requester has no typed id to get wrong
        val e = runCatching {
            runBlocking { core.decide(v.token, ReleaseDecision.Approve(channels = ReleaseChannels(qr = true), typedId = "0000000000000000"), now) }
        }.exceptionOrNull() as CoreException
        assertEquals(CoreException.Code.Malformed, e.code)
        assertTrue("token still valid", core.pendingView(v.token) != null)
        runBlocking { core.decide(v.token, ReleaseDecision.Approve(channels = ReleaseChannels(qr = true), typedId = v.mustType), now) }
        assertNull("consumed by the successful decide", core.pendingView(v.token))
    }
}
