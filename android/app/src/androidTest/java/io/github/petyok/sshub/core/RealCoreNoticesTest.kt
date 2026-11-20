package io.github.petyok.sshub.core

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import io.github.petyok.sshub.core.uniffi.UniffiPhoneCore
import io.github.petyok.sshub.fake.EnrollSamples
import io.github.petyok.sshub.platform.SealedStateStore
import io.github.petyok.sshub.testing.assertNoVisualOverflow
import io.github.petyok.sshub.testing.assertTargets48
import io.github.petyok.sshub.testing.setScreen
import io.github.petyok.sshub.ui.screens.enroll.EnrollScanActions
import io.github.petyok.sshub.ui.screens.enroll.EnrollScanScreen
import io.github.petyok.sshub.ui.screens.release.RedActions
import io.github.petyok.sshub.ui.screens.release.RedScreen
import io.github.petyok.sshub.ui.screens.release.RedState
import io.github.petyok.sshub.ui.screens.trust.TrustExtendActions
import io.github.petyok.sshub.ui.screens.trust.TrustExtendScreen
import io.github.petyok.sshub.ui.screens.trust.TrustExtendState
import io.github.petyok.sshub.ui.screens.trust.TrustRequestActions
import io.github.petyok.sshub.ui.screens.trust.TrustRequestScreen
import io.github.petyok.sshub.ui.screens.trust.TrustRequestState
import java.io.File
import java.util.UUID

/**
 * M11 and L10: Red notice, trust request and trust extension as the real core opens the public frames of notices.json (built by the
 * terminal's encoders), and the enrollment name rules from the core's `check_enroll_name`.
 */
class RealCoreNoticesTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private val fx = JSONObject(InstrumentationRegistry.getInstrumentation().context.assets.open("notices.json").bufferedReader().readText())
    private val now = fx.getLong("now")

    private fun hex(s: String) = ByteArray(s.length / 2) { s.substring(2 * it, 2 * it + 2).toInt(16).toByte() }
    private fun case(name: String): JSONObject = fx.getJSONArray("cases").let { a -> (0 until a.length()).map { a.getJSONObject(it) }.first { it.getString("name") == name } }

    private fun core(): UniffiPhoneCore {
        val dir = File(ctx.cacheDir, "notices-" + UUID.randomUUID()).also { it.mkdirs() }
        return UniffiPhoneCore(SealedStateStore(dir)).also { it.blocked = { false }; it.vault = PassVault(); runBlocking { it.unlock(hex(fx.getJSONObject("phone").getString("secret_hex")), now) } }
    }

    private fun red(core: UniffiPhoneCore): Pair<RedView, Long> {
        val c = case("red-notice")
        val at = c.getLong("open_at")
        return (runBlocking { core.openFrame(hex(c.getString("frame_hex")), at) } as OpenedRed).view to at
    }

    private fun request(core: UniffiPhoneCore): TrustRequestView =
        (runBlocking { core.openFrame(hex(case("trust-request").getString("frame_hex")), now) } as OpenedTrustRequest).view

    private fun extension(core: UniffiPhoneCore): TrustExtendView {
        val rq = request(core)
        val typed = case("trust-request").getJSONObject("approve").getString("typed_id")
        runBlocking { core.decideTrust(rq.token, TrustDecision.Approve(typed), now) }
        val at = now + case("trust-extension").getLong("open_after_secs")
        return (runBlocking { core.openFrame(hex(case("trust-extension").getString("frame_hex")), at) } as OpenedTrustExtend).view
    }

    @Test fun red_notice_from_an_unmet_proposer_needs_its_id_and_shows_every_op_row() {
        val (v, at) = red(core())
        assertEquals(case("red-notice").getJSONObject("expect").getString("must_type"), v.mustType)
        assertTrue(v.fields.isNotEmpty())
        rule.setScreen { RedScreen(RedState(v, at), RedActions()) }
        rule.onNodeWithText("This proposer has never met this phone.").assertExists()
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        assertTrue("the core gives a plain sentence", v.plain.isNotBlank())
        rule.onNodeWithText(v.plain).assertExists()
        rule.onNodeWithText("Technical details: ${v.fields.size} fields", substring = true).performClick() // raw fields are behind the toggle
        v.fields.forEach { f -> rule.onNode(hasContentDescription("Field ${f.key}, new value ${f.to}")).assertExists() }
        assertTrue("the core flags danger rows", v.fields.any { it.danger })
        rule.assertTargets48()
    }

    @Test fun red_notice_at_font_scale_2_has_no_overflow_and_no_broken_word() {
        val (v, at) = red(core())
        rule.setScreen(fontScale = 2f) { RedScreen(RedState(v, at), RedActions()) }
        rule.assertNoVisualOverflow()
    }

    @Test fun trust_request_shows_the_cores_headline_and_the_typed_id_gate() {
        val v = request(core())
        assertEquals(case("trust-request").getJSONObject("expect").getString("headline"), v.headline)
        assertNull(v.refusal)
        rule.setScreen { TrustRequestScreen(TrustRequestState(v), TrustRequestActions({}, {}, {}, {})) }
        rule.onNodeWithText(v.headline).assertExists()
        rule.onNodeWithText("Approve").assertIsNotEnabled()
        rule.assertTargets48()
    }

    @Test fun trust_request_at_font_scale_2_has_no_overflow() {
        val v = request(core())
        rule.setScreen(fontScale = 2f) { TrustRequestScreen(TrustRequestState(v), TrustRequestActions({}, {}, {}, {})) }
        rule.assertNoVisualOverflow()
    }

    @Test fun trust_extension_shows_old_and_new_allowance_never_five_to_three() {
        val v = extension(core())
        assertEquals(case("trust-extension").getJSONObject("expect").getString("headline"), v.headline)
        assertNull(v.refusal)
        assertNotNull(v.fields.firstOrNull { it.danger })
        assertTrue("old and new allowance as a pair", v.diffFrom.isNotEmpty() && v.diffTo.startsWith("a total of ") && v.diffTo.contains("rolling day"))
        rule.setScreen { TrustExtendScreen(TrustExtendState(v), TrustExtendActions({}, {}, {}, {}, {})) }
        rule.onNode(hasContentDescription("Field ${v.diffKey} changes from ${v.diffFrom} to ${v.diffTo}")).assertExists()
        rule.assertTargets48()
    }

    @Test fun trust_extension_at_font_scale_2_has_no_overflow() {
        val v = extension(core())
        rule.setScreen(fontScale = 2f) { TrustExtendScreen(TrustExtendState(v), TrustExtendActions({}, {}, {}, {}, {})) }
        rule.assertNoVisualOverflow()
    }

    @Test fun enrollment_name_problems_come_from_the_core_and_keep_scan_disabled() {
        val core = core()
        assertNull(core.nameProblem("pixel-8"))
        val empty = core.nameProblem("")
        val long = core.nameProblem("x".repeat(65))
        val control = core.nameProblem("bad\u0007name")
        assertNotNull(empty); assertNotNull(long); assertNotNull(control)
        rule.setScreen { EnrollScanScreen(EnrollSamples.scan().copy(name = "x".repeat(65), nameProblem = long), EnrollScanActions()) }
        rule.onNodeWithText(long!!).assertExists()
        rule.onNodeWithText("Scan the terminal's code").assertIsNotEnabled()
    }
}
