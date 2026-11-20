package io.github.petyok.sshub.platform

import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Assert.assertEquals
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

/** H1: screen off locks at once; leaving the app starts a timer on the monotonic clock; coming back in time cancels it. */
class AutoLockTest {
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private val clock = java.util.concurrent.atomic.AtomicLong(1_000)
    private val locks = AtomicInteger()
    private fun platform(seconds: Int) = AndroidPlatform(ctx, null, { clock.get() }, CoroutineScope(SupervisorJob() + Dispatchers.Default))
        .also { it.lockPolicy = LockPolicy({ seconds }, { locks.incrementAndGet() }) }

    @Test fun screen_off_locks_at_once() {
        platform(60).onScreenOff()
        assertEquals(1, locks.get())
    }

    @Test fun the_timer_locks_after_the_configured_time_in_the_background() {
        val p = platform(1)
        p.onAppStopped()
        assertEquals(0, locks.get())
        Thread.sleep(1_600)
        assertEquals(1, locks.get())
    }

    @Test fun coming_back_before_the_time_cancels_the_timer() {
        val p = platform(1)
        p.onAppStopped()
        clock.addAndGet(200)
        p.onAppStarted()
        Thread.sleep(1_600)
        assertEquals(0, locks.get())
    }

    @Test fun coming_back_after_the_time_locks_even_if_the_timer_never_ran() {
        val p = platform(30)
        p.onAppStopped()
        clock.addAndGet(31_000) // the process was frozen: only the monotonic clock knows
        p.onAppStarted()
        assertEquals(1, locks.get())
    }
}
