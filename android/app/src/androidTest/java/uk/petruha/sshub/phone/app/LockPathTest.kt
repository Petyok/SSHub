package uk.petruha.sshub.phone.app

import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.InMemorySettingsStore
import uk.petruha.sshub.phone.core.HomeApi
import uk.petruha.sshub.phone.core.PendingItem
import uk.petruha.sshub.phone.core.PendingKind
import uk.petruha.sshub.phone.core.PhoneCoreApi
import uk.petruha.sshub.phone.core.Routes
import uk.petruha.sshub.phone.fake.ReleaseSamples
import uk.petruha.sshub.phone.core.VaultResult
import uk.petruha.sshub.phone.core.VaultState
import uk.petruha.sshub.phone.fake.FakePhoneCore
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.ui.screens.home.HomeViewModel

/** Behaves like the Rust core after `lock`: the reads that need the unlocked state refuse. */
private class LockingCore(private val inner: FakePhoneCore = FakePhoneCore()) : PhoneCoreApi by inner {
    @Volatile var locked = false
    override fun lock() { locked = true; inner.lock() }
    private inline fun <T> unlocked(block: () -> T): T = if (locked) throw CoreException(CoreException.Code.Refused, "This phone is locked.") else block()
    override suspend fun thisPhone(now: Long) = unlocked { inner.thisPhone(now) }
    override suspend fun ring(now: Long) = unlocked { inner.ring(now) }
    override fun pending(now: Long) = unlocked { listOf(PendingItem(PendingKind.Release, "db-staging", "now", Routes.Release + "/" + ReleaseSamples.TOKEN_DBS)) }
}

/** H1: every lock source ends as a vault that is not Unlocked, the UI follows it, and no `Locked` exception reaches a screen. */
@RunWith(AndroidJUnit4::class)
class LockPathTest {
    @get:Rule val rule = createComposeRule()

    @Test fun the_vault_locking_on_a_release_screen_shows_lock_without_a_core_exception() {
        val core = LockingCore(); val platform = FakePlatform()
        rule.setContent { AppRoot(core, platform, InMemorySettingsStore(verified = true)) }
        rule.onNode(hasText("Approvals") and SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Tab)).performClick()
        rule.onNodeWithText("db-staging").performScrollTo().performClick()
        rule.onAllNodesWithText("Approve", substring = true, ignoreCase = true).onFirst().assertExists()
        platform.keys.nextResult = VaultResult.Canceled // the Lock screen asks for the system prompt at once; here the user does not answer
        rule.runOnIdle { core.lock(); platform.keys.lock() } // what the screen-off receiver does
        rule.onNodeWithText("Locked because the screen turned off.", substring = true).assertExists()
        rule.onAllNodesWithText("db-staging").assertCountEquals(0)
    }

    @Test fun a_restored_screen_with_a_locked_vault_lands_on_lock() {
        val core = LockingCore().also { it.lock() }; val platform = FakePlatform()
        platform.keys.nextResult = VaultResult.Canceled
        platform.keys.st.value = VaultState.Locked // the new process: unlocked in no way, but the nav state was restored on Home
        rule.setContent { AppRoot(core, platform) }
        rule.onAllNodesWithText("Unlock", substring = true).onFirst().assertExists()
    }

    @Test fun home_reading_a_locked_core_keeps_its_state_and_does_not_throw() {
        val core = LockingCore(); val platform = FakePlatform()
        lateinit var vm: HomeViewModel
        androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().runOnMainSync {
            vm = HomeViewModel(core as HomeApi, platform)
        }
        val end = System.currentTimeMillis() + 10_000
        while (vm.state.value == null && System.currentTimeMillis() < end) Thread.sleep(25)
        androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().runOnMainSync {
            val before = vm.state.value
            assertNotNull(before)
            core.lock()
            vm.refresh()
            assertEquals(before, vm.state.value)
        }
    }

    @Test fun home_built_over_a_locked_core_has_no_state_instead_of_throwing() {
        val core = LockingCore().also { it.lock() }
        androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().runOnMainSync {
            assertEquals(null, HomeViewModel(core as HomeApi, FakePlatform()).state.value)
        }
    }

    @Test fun a_locked_vault_resets_the_enrollment_flow_the_core_dropped() {
        val platform = FakePlatform(); val core = FakePhoneCore()
        val inst = androidx.test.platform.app.InstrumentationRegistry.getInstrumentation()
        lateinit var vm: uk.petruha.sshub.phone.ui.screens.enroll.EnrollViewModel
        inst.runOnMainSync {
            vm = uk.petruha.sshub.phone.ui.screens.enroll.EnrollViewModel(core, platform, androidx.navigation.NavHostController(inst.targetContext), "laptop")
            vm.startScan()
        }
        inst.waitForIdleSync()
        assertEquals(true, vm.ui.value.flow.scanning)
        inst.runOnMainSync { platform.keys.lock() }
        inst.waitForIdleSync()
        assertEquals("the flow state follows the core that dropped its enrollment", false, vm.ui.value.flow.scanning)
        assertEquals("laptop", vm.ui.value.flow.name)
    }
}
