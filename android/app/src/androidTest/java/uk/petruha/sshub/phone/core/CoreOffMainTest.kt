package uk.petruha.sshub.phone.core

import android.os.Handler
import android.os.Looper
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import uk.petruha.sshub.phone.core.uniffi.UniffiPhoneCore
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.platform.SealedStateStore
import uk.petruha.sshub.phone.ui.screens.home.HomeViewModel
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Platform M6: the core's Mutex is held for seconds by a release; no call into it may make the UI wait. */
class CoreOffMainTest {
    private val inst = InstrumentationRegistry.getInstrumentation()
    private val ctx = inst.targetContext
    private val now = JSONObject(inst.context.assets.open("challenges.json").bufferedReader().readText()).getLong("now")

    private fun core(): UniffiPhoneCore {
        val dir = File(ctx.cacheDir, "m6-" + UUID.randomUUID()).also { it.mkdirs() }
        return UniffiPhoneCore(SealedStateStore(dir)).also { c -> runBlocking { c.unlock(ByteArray(32) { 0x42 }, now) } }
    }

    /** How long the main thread takes to run one posted task. */
    private fun mainLatencyMs(): Long {
        val t0 = System.nanoTime()
        val done = CountDownLatch(1)
        Handler(Looper.getMainLooper()).post { done.countDown() }
        assertTrue("the main thread did not answer within 5 s", done.await(5, TimeUnit.SECONDS))
        return (System.nanoTime() - t0) / 1_000_000
    }

    @Test fun the_ui_stays_responsive_while_the_core_is_busy_for_two_seconds() {
        val c = core()
        val busy = CountDownLatch(1)
        // Stands in for a short-code release holding the core: the one core thread is occupied for 2 s.
        val blocker = Thread { runBlocking { withContext(c.coreDispatcher) { busy.countDown(); Thread.sleep(2_000) } } }.also { it.start() }
        assertTrue(busy.await(5, TimeUnit.SECONDS))
        lateinit var vm: HomeViewModel
        inst.runOnMainSync { vm = HomeViewModel(c, FakePlatform()) } // reads status, ring, thisPhone: all queued behind the busy core
        val worst = (1..10).maxOf { Thread.sleep(100); mainLatencyMs() }
        assertTrue("main thread stalled for $worst ms while the core was busy", worst < 300)
        assertNull("nothing is drawn until the core answers", vm.state.value)
        blocker.join()
        val end = System.currentTimeMillis() + 10_000
        while (vm.state.value == null && System.currentTimeMillis() < end) Thread.sleep(50)
        assertNotNull("the state arrives once the core is free", vm.state.value)
    }

    @Test fun lock_and_reset_scan_return_at_once_and_run_after_the_busy_call() {
        val c = core()
        val busy = CountDownLatch(1)
        val blocker = Thread { runBlocking { withContext(c.coreDispatcher) { busy.countDown(); Thread.sleep(1_500) } } }.also { it.start() }
        assertTrue(busy.await(5, TimeUnit.SECONDS))
        val t0 = System.nanoTime()
        inst.runOnMainSync { c.resetScan(); c.lock() }
        assertTrue("lock and resetScan must not wait for the core", (System.nanoTime() - t0) / 1_000_000 < 500)
        blocker.join()
        // the queued lock ran: a status read now refuses
        val e = runCatching { runBlocking { c.status(now) } }.exceptionOrNull() as? CoreException
        assertNotNull("the queued lock took effect", e)
    }

    @Test fun a_core_call_from_the_main_thread_fails_in_debug_builds() {
        val c = core()
        var failed = false
        inst.runOnMainSync { try { c.scanGray(2, 2, ByteArray(4)) } catch (e: IllegalStateException) { failed = true } }
        assertTrue("scanGray on Main must throw in a debug build", failed)
    }
}
