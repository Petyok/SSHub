package io.github.petyok.sshub.platform

import android.content.Intent
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import io.github.petyok.sshub.MainActivity
import io.github.petyok.sshub.core.OpenedRelease
import io.github.petyok.sshub.fake.FakeReleaseApi
import io.github.petyok.sshub.fake.ReleaseSamples
import io.github.petyok.sshub.ui.screens.release.ReleaseViewModel

/**
 * B1: the platform is one per process, so a ViewModel retained across a configuration change still reaches the live Activity's
 * prompt, launchers and observers. Before the fix every Activity built its own platform and the retained one lost its gate on destroy.
 */
class PlatformScopeTest {
    private val inst = InstrumentationRegistry.getInstrumentation()
    private val ctx = inst.targetContext
    private fun launch() = ActivityScenario.launch<MainActivity>(Intent(ctx, MainActivity::class.java))

    @After fun cleanUp() {
        AndroidPlatform.shared(ctx).promptOverride = null
        runBlocking { AndroidPlatform.shared(ctx).keys.erase() } // the shared vault object, so its state resets too
    }

    @Test fun the_platform_survives_recreate_and_is_bound_to_the_new_activity() {
        launch().use { sc ->
            lateinit var p1: AndroidPlatform
            lateinit var a1: MainActivity
            sc.onActivity { a1 = it; p1 = it.platform }
            assertSame(a1, p1.attachedActivity)
            sc.recreate()
            sc.onActivity { a2 ->
                assertNotSame(a1, a2)
                assertSame("the platform is application scoped", p1, a2.platform)
                assertSame("the prompt, launchers and observer follow the new Activity", a2, a2.platform.attachedActivity)
            }
        }
    }

    @Test fun an_approval_after_recreate_reaches_core_decide_through_the_stubbed_prompt() {
        // A key that needs no authentication: the stubbed prompt hands the cipher back and the vault opens.
        runBlocking { AndroidKeyVault(ctx, requireAuth = false).store(ByteArray(32) { 5 }) }
        launch().use { sc ->
            lateinit var p: AndroidPlatform
            sc.onActivity { p = it.platform }
            val core = FakeReleaseApi()
            val now = p.clock.nowSeconds()
            core.views[ReleaseSamples.TOKEN_DBS] = OpenedRelease(ReleaseSamples.releaseDbs.copy(signedAt = now, expiresAt = now + 120))
            lateinit var vm: ReleaseViewModel
            inst.runOnMainSync { vm = ReleaseViewModel(core, p, ReleaseSamples.TOKEN_DBS) } // built before the change, like a retained one
            sc.recreate()
            p.promptOverride = { _, _, cipher -> cipher }
            inst.runOnMainSync { assertNotNull(vm.state.value); vm.approve() }
            val end = System.currentTimeMillis() + 10_000
            while (core.decisions.isEmpty() && System.currentTimeMillis() < end) Thread.sleep(50)
            assertEquals("the decision must reach core.decide", 1, core.decisions.size)
            assertEquals(ReleaseSamples.TOKEN_DBS, core.decisions[0].first)
            assertTrue(vm.state.value?.error == null)
        }
    }
}
