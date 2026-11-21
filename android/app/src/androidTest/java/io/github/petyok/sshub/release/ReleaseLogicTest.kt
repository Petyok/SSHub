package io.github.petyok.sshub.release

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import io.github.petyok.sshub.core.ChannelKind
import io.github.petyok.sshub.core.ReleaseCheck
import io.github.petyok.sshub.core.VaultResult
import io.github.petyok.sshub.fake.ReleaseSamples
import io.github.petyok.sshub.ui.screens.release.ApprovalPhase
import io.github.petyok.sshub.ui.screens.release.NO_CHANNEL
import io.github.petyok.sshub.ui.screens.release.TYPE_ID_FIRST
import io.github.petyok.sshub.ui.screens.release.announceStep
import io.github.petyok.sshub.ui.screens.release.announcement
import io.github.petyok.sshub.ui.screens.release.durationText
import io.github.petyok.sshub.ui.screens.release.hashPrefix
import io.github.petyok.sshub.ui.screens.release.expiresText
import io.github.petyok.sshub.ui.screens.release.redGate
import io.github.petyok.sshub.ui.screens.release.redLapsed
import io.github.petyok.sshub.ui.screens.release.releaseGate
import io.github.petyok.sshub.ui.screens.release.spokenDuration
import io.github.petyok.sshub.ui.screens.release.spokenWindow
import io.github.petyok.sshub.ui.screens.release.typedIdMatches
import io.github.petyok.sshub.ui.screens.release.vaultFailure
import io.github.petyok.sshub.ui.screens.release.windowClosed
import io.github.petyok.sshub.ui.screens.release.windowLeft

/** Pure functions only: no Compose, no clock. Time is an explicit `now`. */
class ReleaseLogicTest {
    private fun checks(vararg ok: Boolean) = ok.mapIndexed { i, b -> ReleaseCheck("check $i", b) }

    @Test fun d8_approve_enabled_iff_all_checks_pass() {
        for (mask in 0 until 8) {
            val c = checks(mask and 1 != 0, mask and 2 != 0, mask and 4 != 0)
            val gate = redGate(c, lapsed = false, closed = false, busy = false)
            assertEquals("mask $mask", mask == 7, gate.enabled)
            if (mask != 7) assertEquals(c.first { !it.ok }.text, gate.reason)
            else assertNull(gate.reason)
        }
    }

    @Test fun d8_release_gate_blocks_on_any_failing_check_without_typed_id_waiver() {
        for (mask in 0 until 8) {
            val c = checks(mask and 1 != 0, mask and 2 != 0, mask and 4 != 0)
            assertEquals(mask == 7, releaseGate(c, 60, closed = false, mustType = null, typed = "", busy = false).enabled)
        }
    }

    @Test fun d8_a_failing_check_names_itself_as_the_reason() {
        val g = redGate(ReleaseSamples.redFailing.checks, lapsed = false, closed = false, busy = false)
        assertFalse(g.enabled)
        assertEquals("Hash does not match what the terminal shows", g.reason)
    }

    @Test fun d10_approval_window_closes_at_exactly_12_minutes() {
        assertFalse(windowClosed(1000, 1000 + 719))
        assertTrue(windowClosed(1000, 1000 + 720))
        assertEquals(720L, windowLeft(1000, 1000))
        assertEquals(1L, windowLeft(1000, 1000 + 719))
        assertEquals(0L, windowLeft(1000, 1000 + 5000))
        val closed = redGate(checks(true), lapsed = false, closed = true, busy = false)
        assertFalse(closed.enabled)
        assertEquals("window closed", closed.reason)
        assertTrue(ReleaseSamples.redState(now = ReleaseSamples.T0 + 720, openedAt = ReleaseSamples.T0).closed)
        assertFalse(ReleaseSamples.redState(now = ReleaseSamples.T0 + 719, openedAt = ReleaseSamples.T0).closed)
    }

    @Test fun d10_red_notice_lapses_after_3_days() {
        val v = ReleaseSamples.redRevoke
        assertFalse(redLapsed(v, v.lapsesAt - 1))
        assertTrue(redLapsed(v, v.lapsesAt))
        assertEquals(3 * 86_400L, v.lapsesAt - v.proposedAt)
        val g = redGate(v.checks, lapsed = true, closed = false, busy = false)
        assertFalse(g.enabled)
        assertEquals("the request lapsed after 3 days", g.reason)
        assertTrue(ReleaseSamples.redState(now = v.lapsesAt).lapsed)
        assertTrue(ReleaseSamples.redState(now = v.lapsesAt - 1).gate.enabled)
    }

    @Test fun d10_expired_release_cannot_be_approved() {
        val v = ReleaseSamples.releaseDbs
        assertTrue(releaseGate(v.checks, 1, closed = false, mustType = null, typed = "", busy = false).enabled)
        assertFalse(releaseGate(v.checks, 0, closed = false, mustType = null, typed = "", busy = false).enabled)
        assertFalse(ReleaseSamples.releaseState(now = v.expiresAt).gate.enabled)
        assertTrue(ReleaseSamples.releaseState(now = v.expiresAt - 1).gate.enabled)
    }

    @Test fun d11_time_passing_never_changes_a_decision() {
        val s = ReleaseSamples.redState()
        val later = s.copy(now = s.now + 100_000)
        assertEquals(ApprovalPhase.Review, later.phase)
        assertEquals(null, later.outcome)
        val r = ReleaseSamples.releaseState()
        assertEquals(ApprovalPhase.Review, r.copy(now = r.now + 1_000).phase)
    }

    @Test fun d13_never_met_requester_needs_the_typed_id() {
        val v = ReleaseSamples.releaseBastion
        val id = v.mustType!!
        assertFalse(releaseGate(v.checks, 30, closed = false, mustType = id, typed = "", busy = false).enabled)
        assertEquals("type the id first", releaseGate(v.checks, 30, closed = false, mustType = id, typed = "E7A4", busy = false).reason)
        assertFalse(releaseGate(v.checks, 30, closed = false, mustType = id, typed = "E7A4-6B03", busy = false).enabled)
        assertTrue(releaseGate(v.checks, 30, closed = false, mustType = id, typed = "E7A4-6B02", busy = false).enabled)
        assertTrue(releaseGate(v.checks, 30, closed = false, mustType = id, typed = "  e7a4-6b02 ", busy = false).enabled)
    }

    @Test fun d13_the_waiver_covers_only_the_check_marked_for_it() {
        val v = ReleaseSamples.releaseBastion
        val extra = v.checks + ReleaseCheck("Own share unwrapped", false)
        assertFalse(releaseGate(extra, 30, closed = false, mustType = "E7A4-6B02", typed = "E7A4-6B02", busy = false).enabled)
        // a marked check with no id to type keeps blocking
        assertFalse(releaseGate(v.checks, 30, closed = false, mustType = null, typed = "", busy = false).enabled)
        assertTrue(typedIdMatches(null, ""))
    }

    @Test fun release_timer_speaks_only_at_120_60_30_10_and_0() {
        assertEquals(listOf(120, 60, 30, 10, 0), listOf(120L, 60L, 30L, 10L, 0L).map { announceStep(it) })
        assertEquals(120, announceStep(82)); assertEquals(60, announceStep(59)); assertEquals(30, announceStep(11)); assertEquals(10, announceStep(1))
        val spoken = (0L..120L).map { announcement(it, released = true) }.distinct()
        assertEquals(5, spoken.size)
        assertEquals("Share expired", announcement(0, released = true))
        assertEquals("Share released, up to 60 seconds left", announcement(45, released = true))
        assertEquals("Request open, up to 120 seconds left", announcement(82, released = false))
        assertEquals("Request expired", announcement(0, released = false))
        assertEquals(announcement(119, true), announcement(61, true))
    }

    @Test fun window_countdown_text_changes_every_10_seconds() {
        assertEquals("12 minutes left", spokenWindow(720))
        assertEquals("11 minutes 50 seconds left", spokenWindow(710))
        assertEquals("11 minutes 50 seconds left", spokenWindow(715))
        assertEquals("1 minute left", spokenWindow(60))
        assertEquals("50 seconds left", spokenWindow(59))
        assertEquals("0 seconds left", spokenWindow(0))
        assertEquals("expires in 1:58", expiresText(118)); assertEquals("expires in 0:00", expiresText(-5))
        assertEquals("expires in 59:59", expiresText(3599)); assertEquals("expires in 1 h 0 min", expiresText(3600))
        assertEquals("expires in 2 days 4 h", expiresText(2 * 86_400 + 4 * 3_600 + 59)); assertEquals("expires in 1 day 0 h", expiresText(86_400))
    }

    @Test fun durations_read_like_the_prototype() {
        assertEquals("6 min", durationText(360)); assertEquals("1 h 12 min", durationText(4320)); assertEquals("2 d 23 h", durationText(258_840))
        assertEquals("38 s", durationText(38)); assertEquals("1 min 48 s", durationText(108)); assertEquals("3 h", durationText(10_800))
        assertEquals("2 days 23 hours", spokenDuration(258_840)); assertEquals("1 hour 0 minutes", spokenDuration(3600))
    }

    @Test fun vault_failures_are_text_and_only_a_secret_goes_on() {
        assertNull(vaultFailure(VaultResult.Secret(ByteArray(32))))
        listOf(VaultResult.Canceled, VaultResult.LockedOut, VaultResult.Invalidated, VaultResult.NoSecureLock, VaultResult.Stored, VaultResult.Failed("x"))
            .forEach { assertTrue(vaultFailure(it)!!.contains("Nothing was signed")) }
    }

    @Test fun hash_prefix_is_three_groups() {
        assertEquals("1BF0 D57E F9DD", hashPrefix(ReleaseSamples.redRevoke.hash))
    }

    @Test fun h2_red_gate_needs_the_typed_id_only_for_a_proposer_never_met() {
        val ok = checks(true, true, true)
        assertTrue(redGate(ok, lapsed = false, closed = false, busy = false, mustType = null, typed = "").enabled)
        assertEquals(TYPE_ID_FIRST, redGate(ok, false, false, false, mustType = "7b3e0d15aabbccdd", typed = "").reason)
        assertFalse(redGate(ok, false, false, false, mustType = "7b3e0d15aabbccdd", typed = "0000000000000000").enabled)
        assertTrue(redGate(ok, false, false, false, mustType = "7b3e0d15aabbccdd", typed = " 7B3E0D15AABBCCDD ").enabled)
        // a failing check still blocks after the id is typed
        assertFalse(redGate(checks(true, false, true), false, false, false, mustType = "7b3e0d15aabbccdd", typed = "7b3e0d15aabbccdd").enabled)
    }

    @Test fun m3_release_gate_blocks_when_nothing_shows_the_answer() {
        val ok = checks(true, true, true)
        assertEquals(NO_CHANNEL, releaseGate(ok, 60, closed = false, mustType = null, typed = "", busy = false, noChannel = true).reason)
        assertTrue(releaseGate(ok, 60, closed = false, mustType = null, typed = "", busy = false, noChannel = false).enabled)
    }

    @Test fun m3_a_release_starts_with_qr_only() {
        assertEquals(setOf(ChannelKind.Qr), ReleaseSamples.releaseState().channelsOn)
    }

    @Test fun fake_data_has_matching_hash_length() {
        listOf(ReleaseSamples.redRevoke, ReleaseSamples.redMode).forEach { assertEquals(64, it.hash.length) }
        assertEquals(18, ReleaseSamples.refusals.size)
        assertEquals(18, ReleaseSamples.refusals.map { it.reason.key }.distinct().size)
    }

    @Test fun u13_17_the_one_timer_is_the_screen_window_only_when_it_ends_first() {
        val t0 = ReleaseSamples.T0
        val v = ReleaseSamples.releaseDbs
        val fresh = ReleaseSamples.releaseState(now = v.expiresAt - 120)
        assertEquals(120L, fresh.timerLeft); assertFalse(fresh.windowIsTimer)
        val late = ReleaseSamples.releaseState(now = t0, openedAt = t0 - 700)
        assertEquals(20L, late.timerLeft); assertTrue(late.windowIsTimer)
        val red = ReleaseSamples.redState(now = t0, openedAt = t0)
        assertEquals(720L, red.timerLeft); assertTrue(red.windowIsTimer)
        val soon = ReleaseSamples.redState(now = red.view.lapsesAt - 90, openedAt = red.view.lapsesAt - 90)
        assertEquals(90L, soon.timerLeft); assertFalse(soon.windowIsTimer)
    }
}
