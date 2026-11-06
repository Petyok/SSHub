package uk.petruha.sshub.phone.platform

import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uk.petruha.sshub.phone.app.AppRoot
import uk.petruha.sshub.phone.app.AppRouter
import uk.petruha.sshub.phone.core.AutoLockChoices
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.ReleaseDecision
import uk.petruha.sshub.phone.core.ThemeChoice
import uk.petruha.sshub.phone.core.uniffi.UniffiPhoneCore
import uk.petruha.sshub.phone.fake.FakePhoneCore
import uk.petruha.sshub.phone.fake.FakePlatform
import java.io.File
import java.util.UUID

/** H2 and D17 (security M1): the settings and the signer-mismatch block survive a restart, and the block is enforced. */
@RunWith(AndroidJUnit4::class)
class SignerMismatchTest {
    @get:Rule val rule = createComposeRule()
    private val ctx = InstrumentationRegistry.getInstrumentation().targetContext
    private fun file() = File(ctx.cacheDir, "settings-" + UUID.randomUUID() + "/settings.txt")

    @Test fun settings_and_the_block_survive_a_process_restart() {
        val f = file()
        val first = FileSettingsStore(f)
        first.setTheme(ThemeChoice.Light); first.setAutoLock(300); first.setBackupConfirmed(true); first.setSignerMismatch(true)
        val again = FileSettingsStore(f) // a new process reads the file
        assertEquals(ThemeChoice.Light, again.theme.value)
        assertEquals(300, again.autoLockSeconds.value)
        assertTrue(again.backupConfirmed.value)
        assertTrue("the D17 block persists", again.signerMismatch.value)
    }

    @Test fun the_block_is_sticky() {
        val f = file()
        val s = FileSettingsStore(f)
        s.setSignerMismatch(true)
        s.setSignerMismatch(false) // the user types a matching value later: the block stays
        assertTrue(s.signerMismatch.value)
        assertTrue(FileSettingsStore(f).signerMismatch.value)
    }

    @Test fun defaults_without_a_file_and_a_corrupt_file() {
        val f = file()
        val d = FileSettingsStore(f)
        assertEquals(AutoLockChoices.DEFAULT, d.autoLockSeconds.value); assertFalse(d.signerMismatch.value); assertFalse(d.backupConfirmed.value)
        f.parentFile!!.mkdirs(); f.writeText("autolock=7\ngarbage\ntheme=Nope\n")
        val c = FileSettingsStore(f)
        assertEquals(AutoLockChoices.DEFAULT, c.autoLockSeconds.value); assertEquals(ThemeChoice.Dark, c.theme.value)
    }

    @Test fun a_persisted_block_keeps_every_flow_closed_after_restart() {
        val f = file(); FileSettingsStore(f).setSignerMismatch(true)
        val platform = FakePlatform()
        rule.setContent { AppRoot(FakePhoneCore(), platform, FileSettingsStore(f)) }
        rule.onNodeWithText("About and security", substring = true).assertExists() // About
        rule.onAllNodes(hasText("Approvals") and SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Tab)).assertCountEquals(0)
    }

    @Test fun a_scanned_frame_opens_no_route_while_blocked() {
        val opened = Opened.Ignored("x")
        assertNull(AppRouter { true }.route(opened))
    }

    @Test fun the_core_refuses_to_open_frames_and_sign_while_blocked() {
        val dir = File(ctx.cacheDir, "blk-" + UUID.randomUUID()).also { it.mkdirs() }
        val core = UniffiPhoneCore(SealedStateStore(dir)).also { runBlocking { it.unlock(ByteArray(32) { 0x42 }, 1L) } }
        core.blocked = { true }
        for (call in listOf<suspend () -> Any?>(
            { core.openFrame(ByteArray(8), 1L) },
            { core.decide(1L, ReleaseDecision.Decline, 1L) },
            { core.rotateWrapKeys(1L) },
        )) {
            try { runBlocking { call() }; fail("must refuse while the signer mismatch is set") }
            catch (e: CoreException) { assertEquals(CoreException.Code.Refused, e.code); assertTrue(e.message!!.contains("fingerprint")) }
        }
    }
}
