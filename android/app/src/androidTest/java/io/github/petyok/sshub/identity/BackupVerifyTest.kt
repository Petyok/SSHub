package io.github.petyok.sshub.identity

import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import io.github.petyok.sshub.app.AppRoot
import io.github.petyok.sshub.core.InMemorySettingsStore
import io.github.petyok.sshub.core.PendingItem
import io.github.petyok.sshub.core.PendingKind
import io.github.petyok.sshub.core.PhoneCoreApi
import io.github.petyok.sshub.core.Routes
import io.github.petyok.sshub.fake.DemoWords
import io.github.petyok.sshub.fake.FakeIdentityApi
import io.github.petyok.sshub.fake.FakePhoneCore
import io.github.petyok.sshub.fake.FakePlatform
import io.github.petyok.sshub.fake.ReleaseSamples
import io.github.petyok.sshub.testing.setScreen
import io.github.petyok.sshub.ui.screens.identity.BackupVerifyActions
import io.github.petyok.sshub.ui.screens.identity.BackupVerifyScreen
import io.github.petyok.sshub.ui.screens.identity.BackupVerifyState
import io.github.petyok.sshub.ui.screens.identity.BackupVerifyViewModel
import io.github.petyok.sshub.ui.screens.identity.FirstRunActions
import io.github.petyok.sshub.ui.screens.identity.FirstRunScreen
import io.github.petyok.sshub.ui.screens.identity.FirstRunState
import io.github.petyok.sshub.ui.screens.identity.NewPhoneScreen
import io.github.petyok.sshub.ui.screens.identity.RestoreViewModel
import io.github.petyok.sshub.ui.screens.identity.VerifyPhase
import io.github.petyok.sshub.ui.screens.identity.pickCheckWords

private fun onMain(block: () -> Unit) {
    val i = InstrumentationRegistry.getInstrumentation()
    i.runOnMainSync(block)
    i.waitForIdleSync()
}

class PickCheckWordsTest {
    @Test fun every_draw_gives_two_distinct_numbers_in_1_to_24_ascending() {
        for (a in 0 until 24) for (b in 0 until 23) {
            var call = 0
            val pair = pickCheckWords { if (call++ == 0) a else b }
            assertEquals(2, pair.size)
            assertNotEquals("draw $a/$b", pair[0], pair[1])
            assertTrue("draw $a/$b: $pair", pair.all { it in 1..24 } && pair[0] < pair[1])
        }
    }

    @Test fun every_number_can_be_asked() {
        val seen = mutableSetOf<Int>()
        for (a in 0 until 24) for (b in 0 until 23) { var call = 0; seen += pickCheckWords { if (call++ == 0) a else b } }
        assertEquals((1..24).toSet(), seen)
    }
}

class BackupVerifyViewModelTest {
    private val core = FakeIdentityApi()
    private val platform = FakePlatform()
    private val settings = InMemorySettingsStore()

    /** Fixed draw: words no. 1 and 2, "edging" and "evolution". */
    private fun start(): BackupVerifyViewModel {
        lateinit var vm: BackupVerifyViewModel
        onMain { vm = BackupVerifyViewModel(core, platform, settings) { 0 } }
        return vm
    }

    @Test fun a_wrong_word_keeps_the_flag_false_and_marks_that_number() {
        val vm = start()
        assertEquals(VerifyPhase.Ready, vm.state.value.phase)
        assertEquals(listOf(1, 2), vm.state.value.numbers)
        var verified = 0
        onMain { vm.onTyped(0, DemoWords.backup[0]); vm.onTyped(1, "wrong"); vm.check { verified++ } }
        assertEquals(0, verified)
        assertFalse(settings.backupVerified.value)
        assertEquals(setOf(2), vm.state.value.wrong)
        onMain { vm.onTyped(1, "evolution") }
        assertTrue("editing the box clears its mark", vm.state.value.wrong.isEmpty())
    }

    @Test fun the_right_words_set_the_flag_ignoring_case_and_spaces() {
        val vm = start()
        var verified = 0
        onMain { vm.onTyped(0, "  EDGING "); vm.onTyped(1, "Evolution"); vm.check { verified++ } }
        assertEquals(1, verified)
        assertTrue(settings.backupVerified.value)
    }

    @Test fun nothing_is_checked_before_the_system_prompt_is_answered() {
        platform.keys.nextResult = io.github.petyok.sshub.core.VaultResult.Canceled
        val vm = start()
        assertEquals(VerifyPhase.Canceled, vm.state.value.phase)
        onMain { vm.onTyped(0, "edging"); vm.onTyped(1, "evolution"); vm.check { } }
        assertFalse(settings.backupVerified.value)
    }

    @Test fun a_restore_from_the_24_words_counts_as_verified() {
        lateinit var vm: RestoreViewModel
        onMain { vm = RestoreViewModel(core, platform, false, settings) }
        DemoWords.backup.forEachIndexed { i, w -> onMain { vm.onWord(i, w) } }
        onMain { vm.onUsedPage(false); vm.restore { } }
        assertTrue(settings.backupVerified.value)
    }
}

class BackupVerifyScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun asks_for_the_two_numbers_and_says_why() {
        rule.setScreen { BackupVerifyScreen(BackupVerifyState(numbers = listOf(5, 17), phase = VerifyPhase.Ready), BackupVerifyActions()) }
        rule.onNodeWithText("Check your recovery words").assertExists()
        rule.onNodeWithText("Before the first approval, type two of the 24 words.", substring = true).assertExists()
        rule.onNodeWithText("Word no. 5").assertExists()
        rule.onNodeWithText("Word no. 17").assertExists()
    }

    @Test fun a_wrong_word_shows_the_message_and_offers_the_words_again() {
        var again = 0
        rule.setScreen {
            BackupVerifyScreen(
                BackupVerifyState(numbers = listOf(5, 17), typed = listOf("a", "b"), wrong = setOf(17), phase = VerifyPhase.Ready),
                BackupVerifyActions(onShowAgain = { again++ }),
            )
        }
        rule.onNodeWithText("That is not word no. 17. Look at your paper again.").assertExists()
        rule.onAllNodesWithText("That is not word no. 5. Look at your paper again.").assertCountEquals(0)
        rule.onNodeWithText("Show the words again").performScrollTo().performClick()
        assertEquals(1, again)
    }
}

class FirstRunAndNewPhoneTextTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun first_run_says_one_computer_has_no_backup_of_the_host_list() {
        rule.setScreen { FirstRunScreen(FirstRunState(), FirstRunActions()) }
        rule.onNodeWithText("With one computer there is no backup of your host list: this phone and the paper words hold pieces of passwords, not your hosts.", substring = true).assertExists()
        rule.onNodeWithText("Add a second computer, or a view-only copy on a server at a permanent address, before you rely on sync.", substring = true).assertExists()
        rule.onNodeWithText("A full device of your devices", substring = true).assertExists()
        rule.onAllNodesWithText("share holder", substring = true).assertCountEquals(0)
    }

    @Test fun the_new_phone_screen_is_honest_that_there_is_no_pairing() {
        var back = 0
        rule.setScreen { NewPhoneScreen { back++ } }
        rule.onNodeWithText("There is no direct pairing with the old phone yet", substring = true).assertExists()
        rule.onNodeWithContentDescription("Back").performClick()
        assertEquals(1, back)
    }
}

/** A core whose approvals list has one release, so the test can open an approval screen by tapping it. */
private class OneReleaseCore(inner: FakePhoneCore = FakePhoneCore()) : PhoneCoreApi by inner {
    override fun pending(now: Long) = listOf(PendingItem(PendingKind.Release, "db-staging", "now", Routes.Release + "/" + ReleaseSamples.TOKEN_DBS))
}

@RunWith(AndroidJUnit4::class)
class BackupVerifyGateTest {
    @get:Rule val rule = createComposeRule()

    private fun openRelease(settings: InMemorySettingsStore) {
        rule.setContent { AppRoot(OneReleaseCore(), FakePlatform(), settings) }
        rule.onNode(hasText("Approvals") and SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Tab)).performClick()
        rule.onNodeWithText("db-staging").performScrollTo().performClick()
        rule.waitForIdle()
    }

    @Test fun an_approval_route_lands_on_the_check_while_the_words_are_not_verified() {
        openRelease(InMemorySettingsStore(verified = false))
        rule.onNodeWithText("Check your recovery words").assertExists()
    }

    @Test fun a_verified_phone_opens_the_approval_screen() {
        openRelease(InMemorySettingsStore(verified = true))
        rule.onAllNodesWithText("Check your recovery words").assertCountEquals(0)
        rule.onAllNodesWithText("Approve", substring = true, ignoreCase = true).assertCountNotZero()
    }
}

private fun androidx.compose.ui.test.SemanticsNodeInteractionCollection.assertCountNotZero() {
    assertTrue(fetchSemanticsNodes().isNotEmpty())
}
