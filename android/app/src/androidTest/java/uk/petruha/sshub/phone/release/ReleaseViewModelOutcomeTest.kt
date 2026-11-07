package uk.petruha.sshub.phone.release

import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertSame
import org.junit.Test
import uk.petruha.sshub.phone.core.ChannelKind
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.ReleaseApi
import uk.petruha.sshub.phone.core.ReleaseDecision
import uk.petruha.sshub.phone.core.ReleaseOutcome
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.fake.ReleaseSamples
import uk.petruha.sshub.phone.ui.screens.release.ApprovalPhase
import uk.petruha.sshub.phone.ui.screens.release.ReleaseViewModel
import uk.petruha.sshub.phone.ui.screens.release.ReleasedViews

/** M18: a release screen opened again after Approve shows the outcome the core kept, not an Approve button. */
class ReleaseViewModelOutcomeTest {
    private val kept = ReleaseOutcome(ReleaseSamples.outgoing())

    private val api = object : ReleaseApi {
        override fun pendingView(token: Long): Opened? = null
        override suspend fun decide(token: Long, decision: ReleaseDecision, now: Long): ReleaseOutcome = error("not asked")
        override fun lastOutcome(token: Long): ReleaseOutcome? = if (token == 77L) kept else null
    }

    private fun vm(token: Long): ReleaseViewModel {
        lateinit var v: ReleaseViewModel
        val i = InstrumentationRegistry.getInstrumentation()
        i.runOnMainSync { v = ReleaseViewModel(api, FakePlatform(), token) }
        i.waitForIdleSync()
        return v
    }

    @Test fun the_kept_outcome_is_shown_on_re_entry() {
        ReleasedViews.put(77, ReleaseSamples.releaseDbs, setOf(ChannelKind.Qr))
        val s = vm(77).state.value
        assertNotNull(s)
        assertEquals(ApprovalPhase.Approved, s!!.phase)
        assertSame(kept.outgoing, s.outcome!!.outgoing)
        assertEquals(setOf(ChannelKind.Qr), s.channelsOn)
    }

    @Test fun without_a_kept_outcome_a_gone_request_shows_nothing() {
        assertEquals(null, vm(78).state.value)
    }
}
