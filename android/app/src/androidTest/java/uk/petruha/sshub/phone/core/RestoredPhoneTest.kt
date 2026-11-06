package uk.petruha.sshub.phone.core

import kotlinx.coroutines.runBlocking
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.core.uniffi.UniffiPhoneCore
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.platform.SealedStateStore
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.home.HomeActions
import uk.petruha.sshub.phone.ui.screens.home.HomeState
import uk.petruha.sshub.phone.ui.screens.home.HomeViewModel
import uk.petruha.sshub.phone.ui.screens.home.RingHomeScreen
import uk.petruha.sshub.phone.ui.screens.identity.SecurityMenuViewModel
import java.io.File
import java.util.UUID

/**
 * H5 with the REAL core: a restored phone (head pending, no ring) keeps the card, the button and the Security menu entry after the
 * process is killed and started again over the same sealed state.
 */
class RestoredPhoneTest {
    @get:Rule val rule = createComposeRule()
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private val now = 1_893_452_400L

    private fun dir() = File(ctx.cacheDir, "restored-" + UUID.randomUUID()).also { it.mkdirs() }

    /** A restored phone: words of one core restored into a fresh store, then unlocked. */
    private fun restoredIn(d: File): UniffiPhoneCore {
        val donor = UniffiPhoneCore(SealedStateStore(dir()))
        runBlocking { donor.unlock(ByteArray(32) { 0x42 }, now) }
        val chars = runBlocking { donor.backupWords() }
        val words = chars.map { String(it) }
        chars.forEach { it.fill('\u0000') }
        val core = UniffiPhoneCore(SealedStateStore(d))
        val secret = runBlocking { core.restoreFromWords(words) }
        runBlocking { core.unlock(secret, now) }
        return core
    }

    private fun homeState(core: UniffiPhoneCore): HomeState {
        lateinit var vm: HomeViewModel
        InstrumentationRegistry.getInstrumentation().runOnMainSync { vm = HomeViewModel(core, FakePlatform()) }
        // The core answers on its own thread: wait for the state instead of reading it at once.
        val end = System.currentTimeMillis() + 10_000
        while (vm.state.value == null && System.currentTimeMillis() < end) Thread.sleep(25)
        return vm.state.value!!
    }

    @Test fun restored_state_is_head_pending_without_a_ring_and_survives_a_restart() {
        val d = dir()
        val first = restoredIn(d)
        assertTrue(runBlocking { first.status(now) }.headPending)
        assertNull(runBlocking { first.ring(now) })
        val s1 = homeState(first)
        assertTrue(s1.headPending)
        assertNull(s1.ring)

        // kill and start again: a new core over the same store, unlocked with the same secret
        val again = UniffiPhoneCore(SealedStateStore(d))
        runBlocking { again.unlock(ByteArray(32) { 0x42 }, now) }
        val s2 = homeState(again)
        assertTrue("head pending survives the restart", s2.headPending)
        assertNull(s2.ring)
        assertEquals(null, s2.phone.headSeq)
    }

    @Test fun home_shows_the_card_and_the_button_for_that_state_and_the_menu_offers_take_chain() {
        val d = dir()
        val core = restoredIn(d)
        val again = UniffiPhoneCore(SealedStateStore(d)).also { runBlocking { it.unlock(ByteArray(32) { 0x42 }, now) } }
        val taken = mutableListOf<String>()
        val st = homeState(again)
        rule.setScreen { RingHomeScreen(st, HomeActions(onTakeChain = { taken += "chain" })) }
        rule.onNodeWithText("This phone does not sign yet.").assertExists()
        rule.onNodeWithText("unknown, not taken yet").assertExists()
        rule.onNodeWithText("Take my chain from a terminal").performClick()
        assertEquals(listOf("chain"), taken)

        lateinit var menu: SecurityMenuViewModel
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            menu = SecurityMenuViewModel(again, FakePlatform(), InMemorySettingsStore())
        }
        val end = System.currentTimeMillis() + 10_000
        while (!menu.state.value.headPending && System.currentTimeMillis() < end) Thread.sleep(25)
        assertTrue(menu.state.value.headPending)
        check(runBlocking { core.status(now) }.headPending)
    }
}
