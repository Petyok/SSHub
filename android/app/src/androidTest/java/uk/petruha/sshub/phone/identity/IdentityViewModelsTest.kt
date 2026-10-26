package uk.petruha.sshub.phone.identity

import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uk.petruha.sshub.phone.core.InMemorySettingsStore
import uk.petruha.sshub.phone.core.ThemeChoice
import uk.petruha.sshub.phone.core.VaultResult
import uk.petruha.sshub.phone.core.WordState
import uk.petruha.sshub.phone.fake.DemoWords
import uk.petruha.sshub.phone.fake.FakeChainScan
import uk.petruha.sshub.phone.fake.FakeIdentityApi
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.fake.IdentitySamples
import uk.petruha.sshub.phone.ui.screens.identity.AboutViewModel
import uk.petruha.sshub.phone.ui.screens.identity.BackupPhase
import uk.petruha.sshub.phone.ui.screens.identity.BackupViewModel
import uk.petruha.sshub.phone.ui.screens.identity.ChainHeadPhase
import uk.petruha.sshub.phone.ui.screens.identity.ChainHeadViewModel
import uk.petruha.sshub.phone.ui.screens.identity.FingerprintCompare
import uk.petruha.sshub.phone.ui.screens.identity.FirstRunPhase
import uk.petruha.sshub.phone.ui.screens.identity.FirstRunViewModel
import uk.petruha.sshub.phone.ui.screens.identity.LockPhase
import uk.petruha.sshub.phone.ui.screens.identity.LockReason
import uk.petruha.sshub.phone.ui.screens.identity.LockViewModel
import uk.petruha.sshub.phone.ui.screens.identity.RestorePhase
import uk.petruha.sshub.phone.ui.screens.identity.RestoreViewModel
import uk.petruha.sshub.phone.ui.screens.identity.SecurityMenuViewModel

/** The view models run their coroutines on the main looper; the fakes never suspend, so one main-thread hop completes each action. */
private fun onMain(block: () -> Unit) {
    val i = InstrumentationRegistry.getInstrumentation()
    i.runOnMainSync(block)
    i.waitForIdleSync()
}

class FirstRunViewModelTest {
    private val core = FakeIdentityApi()
    private val platform = FakePlatform()

    @Test fun create_stores_the_secret_wraps_it_unlocks_the_core_and_continues() {
        platform.keys.secret = null
        var created = 0
        onMain { FirstRunViewModel(core, platform).create { created++ } }
        assertEquals(1, created)
        assertTrue(core.unlocked)
        assertEquals(32, platform.keys.secret?.size)
    }

    @Test fun a_phone_without_a_screen_lock_does_not_continue() {
        platform.keys.nextResult = VaultResult.NoSecureLock
        var created = 0
        lateinit var vm: FirstRunViewModel
        onMain { vm = FirstRunViewModel(core, platform); vm.create { created++ } }
        assertEquals(0, created)
        assertEquals(FirstRunPhase.NoSecureLock, vm.state.value.phase)
        assertFalse(core.unlocked)
    }

    @Test fun a_failed_store_shows_the_reason() {
        platform.keys.nextResult = VaultResult.Failed("keystore busy")
        lateinit var vm: FirstRunViewModel
        onMain { vm = FirstRunViewModel(core, platform); vm.create { } }
        assertEquals(FirstRunPhase.Failed, vm.state.value.phase)
        assertEquals("keystore busy", vm.state.value.error)
    }
}

class RestoreViewModelTest {
    private val core = FakeIdentityApi()
    private val platform = FakePlatform()

    private fun fill(vm: RestoreViewModel, words: List<String> = DemoWords.backup) = onMain { words.forEachIndexed { i, w -> vm.onWord(i, w) } }

    @Test fun typed_words_are_checked_by_the_core_and_pasted_text_spreads_over_the_boxes() {
        lateinit var vm: RestoreViewModel
        onMain { vm = RestoreViewModel(core, platform, replaces = false); vm.onWord(0, "edging evolution sanitizer") ; vm.onWord(5, "twich") }
        val s = vm.state.value
        assertEquals(listOf("edging", "evolution", "sanitizer"), s.words.take(3))
        assertEquals(WordState.Ok, s.wordStates[1])
        assertEquals(WordState.Bad, s.wordStates[5])
        assertEquals(3, s.accepted); assertEquals(1, s.bad)
    }

    @Test fun fill_the_grid_takes_up_to_24_words_and_clears_the_paste_box() {
        lateinit var vm: RestoreViewModel
        onMain { vm = RestoreViewModel(core, platform, false); vm.onPaste(DemoWords.backup.joinToString(" ") + " extra"); vm.onFill() }
        assertEquals(DemoWords.backup, vm.state.value.words)
        assertEquals("", vm.state.value.paste)
        assertEquals(24, vm.state.value.accepted)
    }

    @Test fun restore_needs_all_words_and_the_site_data_box() {
        lateinit var vm: RestoreViewModel
        onMain { vm = RestoreViewModel(core, platform, false) }
        fill(vm)
        var restored = 0
        onMain { vm.restore { restored++ } }
        assertEquals(0, restored)
        assertNull(core.restoredWith)
        onMain { vm.onSiteDataCleared(true); vm.restore { restored++ } }
        assertEquals(1, restored)
        assertEquals(DemoWords.backup, core.restoredWith)
        assertTrue(core.headPending)
        assertTrue(core.unlocked)
        assertTrue("words are wiped from the state", vm.state.value.words.all { it.isEmpty() })
    }

    @Test fun a_wrong_checksum_is_shown_and_nothing_continues() {
        core.rejectRestore = true
        lateinit var vm: RestoreViewModel
        onMain { vm = RestoreViewModel(core, platform, true) }
        fill(vm)
        var restored = 0
        onMain { vm.onSiteDataCleared(true); vm.restore { restored++ } }
        assertEquals(0, restored)
        assertEquals(RestorePhase.Failed, vm.state.value.phase)
        assertTrue(vm.state.value.replaces)
    }

    @Test fun no_secure_lock_after_the_words_are_good() {
        platform.keys.nextResult = VaultResult.NoSecureLock
        lateinit var vm: RestoreViewModel
        onMain { vm = RestoreViewModel(core, platform, false) }
        fill(vm)
        onMain { vm.onSiteDataCleared(true); vm.restore { } }
        assertEquals(RestorePhase.NoSecureLock, vm.state.value.phase)
    }
}

class BackupViewModelTest {
    private val core = FakeIdentityApi()
    private val platform = FakePlatform()
    private val settings = InMemorySettingsStore()

    @Test fun the_words_arrive_only_after_the_system_prompt() {
        lateinit var vm: BackupViewModel
        onMain { vm = BackupViewModel(core, platform, settings, firstRun = true) }
        assertEquals(BackupPhase.Ready, vm.state.value.phase)
        assertEquals(DemoWords.backup, vm.state.value.words)
        assertTrue(vm.state.value.firstRun)
    }

    @Test fun a_canceled_prompt_keeps_the_words_hidden_until_the_user_asks_again() {
        platform.keys.nextResult = VaultResult.Canceled
        lateinit var vm: BackupViewModel
        onMain { vm = BackupViewModel(core, platform, settings, false) }
        assertEquals(BackupPhase.Canceled, vm.state.value.phase)
        assertTrue(vm.state.value.words.isEmpty())
        platform.keys.nextResult = null
        onMain { vm.authenticate() }
        assertEquals(BackupPhase.Ready, vm.state.value.phase)
        assertEquals(24, vm.state.value.words.size)
    }

    @Test fun a_locked_out_prompt_is_a_failure_with_words_hidden() {
        platform.keys.nextResult = VaultResult.LockedOut
        lateinit var vm: BackupViewModel
        onMain { vm = BackupViewModel(core, platform, settings, false) }
        assertEquals(BackupPhase.Failed, vm.state.value.phase)
        assertTrue(vm.state.value.words.isEmpty())
    }

    @Test fun done_needs_the_paper_box_then_remembers_it_and_drops_the_words() {
        lateinit var vm: BackupViewModel
        onMain { vm = BackupViewModel(core, platform, settings, false) }
        var done = 0
        onMain { vm.done { done++ } }
        assertEquals(0, done); assertFalse(settings.backupConfirmed.value)
        onMain { vm.onPaperChecked(true); vm.done { done++ } }
        assertEquals(1, done)
        assertTrue(settings.backupConfirmed.value)
        assertTrue(vm.state.value.words.isEmpty())
    }
}

class LockViewModelTest {
    private val core = FakeIdentityApi()
    private val platform = FakePlatform()

    private fun start(result: VaultResult?): LockViewModel {
        platform.keys.nextResult = result
        lateinit var vm: LockViewModel
        onMain { vm = LockViewModel(core, platform, LockReason.ScreenOff) }
        return vm
    }

    @Test fun a_good_unlock_unlocks_the_core() {
        val vm = start(null)
        assertTrue(vm.unlocked.value)
        assertTrue(core.unlocked)
    }

    @Test fun canceled_locked_out_invalidated_and_failed_map_to_their_phases() {
        assertEquals(LockPhase.Canceled, start(VaultResult.Canceled).state.value.phase)
        assertEquals(LockPhase.LockedOut, start(VaultResult.LockedOut).state.value.phase)
        assertEquals(LockPhase.Invalidated, start(VaultResult.Invalidated).state.value.phase)
        val failed = start(VaultResult.Failed("no key"))
        assertEquals(LockPhase.Failed, failed.state.value.phase)
        assertEquals("no key", failed.state.value.message)
        assertFalse(core.unlocked)
    }

    @Test fun try_again_after_a_cancel_can_succeed() {
        val vm = start(VaultResult.Canceled)
        platform.keys.nextResult = null
        onMain { vm.unlock() }
        assertTrue(vm.unlocked.value)
    }
}

class ChainHeadViewModelTest {
    private val core = FakeIdentityApi().also { it.headPending = true }
    private val scan = FakeChainScan()
    private val platform = FakePlatform()

    private fun vm(): ChainHeadViewModel {
        lateinit var v: ChainHeadViewModel
        onMain { v = ChainHeadViewModel(core, scan, platform) }
        return v
    }

    @Test fun two_lists_agree_then_the_user_adopts_with_the_core_token() {
        val vm = vm()
        assertEquals("4F2A-91C7", vm.state.value.deviceId)
        onMain { vm.onOpened(IdentitySamples.chainList(agreed = false)) }
        assertEquals(1, vm.state.value.lists)
        onMain { vm.adopt() }
        assertNull("one list is not enough", core.adoptedToken)
        onMain { vm.onOpened(IdentitySamples.chainList(agreed = true)) }
        assertEquals(2, vm.state.value.lists)
        onMain { vm.adopt() }
        assertEquals(7L, core.adoptedToken)
        assertEquals(ChainHeadPhase.Taken, vm.state.value.phase)
        assertEquals(IdentitySamples.SEQ, vm.state.value.takenSeq)
        assertFalse(core.headPending)
    }

    @Test fun a_refused_list_is_a_value_and_try_another_goes_back_to_reading() {
        val vm = vm()
        onMain { vm.onOpened(IdentitySamples.refusedList()) }
        assertEquals(ChainHeadPhase.Refused, vm.state.value.phase)
        assertEquals(4, vm.state.value.checks.size)
        assertNull(core.adoptedToken)
        onMain { vm.tryAnother() }
        assertEquals(ChainHeadPhase.Read, vm.state.value.phase)
    }

    @Test fun an_ignored_frame_changes_nothing_but_the_notice() {
        val vm = vm()
        onMain { vm.onOpened(uk.petruha.sshub.phone.core.Opened.Ignored("wrong kind")) }
        assertEquals(0, vm.state.value.lists)
        assertEquals("Ignored: wrong kind", vm.state.value.notice)
    }

    @Test fun pasted_text_goes_through_open_frame() {
        val vm = vm()
        onMain { vm.onOpenPaste(); vm.onPasteText("LIST aaaaaaaaaaaaaaaa"); vm.checkPaste() }
        assertEquals(1, vm.state.value.lists)
        assertEquals(ChainHeadPhase.Read, vm.state.value.phase)
        onMain { vm.onOpenPaste(); vm.onPasteText("BAD aaaaaaaaaaaaaaaa"); vm.checkPaste() }
        assertEquals(ChainHeadPhase.Refused, vm.state.value.phase)
    }

    @Test fun never_signed_needs_the_typed_device_id() {
        val vm = vm()
        onMain { vm.onTypedId("4F2A-91C0"); vm.neverSigned() }
        assertNull(core.neverSignedWith)
        assertNotNull(vm.state.value.phase)
        onMain { vm.onTypedId("4f2a-91c7"); vm.neverSigned() }
        assertEquals("4f2a-91c7", core.neverSignedWith)
        assertEquals(ChainHeadPhase.Taken, vm.state.value.phase)
        assertTrue(vm.state.value.neverSigned)
        assertFalse(core.headPending)
    }
}

class AboutAndMenuViewModelTest {
    private val core = FakeIdentityApi()
    private val platform = FakePlatform().also { it.system.apkCertSha256 = IdentitySamples.FINGERPRINT }
    private val settings = InMemorySettingsStore()

    @Test fun d17_a_mismatching_value_sets_the_app_wide_block_and_the_menu_follows() {
        lateinit var about: AboutViewModel
        lateinit var menu: SecurityMenuViewModel
        onMain { about = AboutViewModel(core, platform, settings); menu = SecurityMenuViewModel(core, platform, settings) }
        assertFalse(menu.state.value.blocked)
        onMain { about.toggleCompare(); about.onReference(IdentitySamples.OTHER_FINGERPRINT) }
        assertEquals(FingerprintCompare.Differ, about.state.value.compare)
        assertTrue(settings.signerMismatch.value)
        assertTrue(menu.state.value.blocked)
        onMain { about.onReference(IdentitySamples.FINGERPRINT) }
        assertEquals(FingerprintCompare.Equal, about.state.value.compare)
        assertFalse(settings.signerMismatch.value)
        assertFalse(menu.state.value.blocked)
    }

    @Test fun an_incomplete_value_does_not_block() {
        lateinit var about: AboutViewModel
        onMain { about = AboutViewModel(core, platform, settings); about.toggleCompare(); about.onReference(IdentitySamples.FINGERPRINT.take(30)) }
        assertFalse(settings.signerMismatch.value)
        assertEquals(FingerprintCompare.Incomplete, about.state.value.compare)
    }

    @Test fun about_reads_the_system_values_and_loads_the_licences_only_when_opened() {
        lateinit var about: AboutViewModel
        onMain { about = AboutViewModel(core, platform, settings) }
        assertEquals(IdentitySamples.FINGERPRINT, about.state.value.fingerprint)
        assertEquals(platform.system.versionName, about.state.value.versionName)
        assertTrue(about.state.value.strongBox)
        assertTrue(about.state.value.licences.isEmpty())
        onMain { about.toggleLicences() }
        assertEquals(listOf("licence text of JetBrainsMono-OFL.txt", "licence text of Inter-LICENSE.txt"), about.state.value.licences.map { it.text })
        onMain { about.toggleGroup(2); about.toggleGroup(2); about.toggleGroup(5) }
        assertEquals(setOf(5), about.state.value.checked)
    }

    @Test fun menu_settings_flow_through_the_store_and_lock_wipes_the_core() {
        lateinit var menu: SecurityMenuViewModel
        onMain { menu = SecurityMenuViewModel(core, platform, settings); core.unlock(ByteArray(32), 0) }
        assertTrue(core.unlocked)
        onMain { menu.setTheme(ThemeChoice.Light); menu.setAutoLock(300); menu.lock() }
        assertEquals(ThemeChoice.Light, menu.state.value.theme)
        assertEquals(300, menu.state.value.autoLockSeconds)
        assertFalse(core.unlocked)
    }
}
