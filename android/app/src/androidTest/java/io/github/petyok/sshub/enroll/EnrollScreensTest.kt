package io.github.petyok.sshub.enroll

import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import io.github.petyok.sshub.core.EnrollChannel
import io.github.petyok.sshub.fake.EnrollSamples as S
import io.github.petyok.sshub.testing.setScreen
import io.github.petyok.sshub.ui.screens.enroll.BtPhase
import io.github.petyok.sshub.ui.screens.enroll.ComparePhase
import io.github.petyok.sshub.ui.screens.enroll.ConfirmStatus
import io.github.petyok.sshub.ui.screens.enroll.EnrollBtActions
import io.github.petyok.sshub.ui.screens.enroll.EnrollBtScreen
import io.github.petyok.sshub.ui.screens.enroll.EnrollCompareActions
import io.github.petyok.sshub.ui.screens.enroll.EnrollCompareScreen
import io.github.petyok.sshub.ui.screens.enroll.EnrollConfirmActions
import io.github.petyok.sshub.ui.screens.enroll.EnrollConfirmScreen
import io.github.petyok.sshub.ui.screens.enroll.EnrollKeysActions
import io.github.petyok.sshub.ui.screens.enroll.EnrollKeysScreen
import io.github.petyok.sshub.ui.screens.enroll.EnrollPasteActions
import io.github.petyok.sshub.ui.screens.enroll.EnrollPasteScreen
import io.github.petyok.sshub.ui.screens.enroll.EnrollScanActions
import io.github.petyok.sshub.ui.screens.enroll.EnrollScanScreen
import io.github.petyok.sshub.ui.screens.enroll.EnrollSendActions
import io.github.petyok.sshub.ui.screens.enroll.EnrollShowActions
import io.github.petyok.sshub.ui.screens.enroll.EnrollShowScreen
import io.github.petyok.sshub.ui.screens.enroll.EnrollSoundActions
import io.github.petyok.sshub.ui.screens.enroll.EnrollSoundScreen
import io.github.petyok.sshub.ui.screens.enroll.EnrolledActions
import io.github.petyok.sshub.ui.screens.enroll.EnrolledScreen
import io.github.petyok.sshub.ui.screens.enroll.KeysStatus
import io.github.petyok.sshub.ui.screens.enroll.PasteVerdict
import io.github.petyok.sshub.ui.components.hexGroups
import io.github.petyok.sshub.ui.components.spokenHex
import io.github.petyok.sshub.ui.screens.enroll.QR_DESCRIPTION
import io.github.petyok.sshub.ui.screens.enroll.SoundPhase
import io.github.petyok.sshub.ui.screens.enroll.SoundProfile

class EnrollScreensTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val calls = mutableListOf<String>()
    private fun hit(s: String): () -> Unit = { calls += s }
    private fun click(text: String) = rule.onNodeWithText(text).performClick()
    private fun exists(text: String) = rule.onNodeWithText(text).assertExists()
    private fun absent(text: String) = rule.onNodeWithText(text).assertDoesNotExist()
    private val notSelectable = SemanticsMatcher.keyNotDefined(SemanticsProperties.Selected)

    // ---- 1 Scan ----

    @Test fun scan_main_path_scan_name_and_channel() {
        rule.setScreen {
            EnrollScanScreen(
                S.scan(),
                EnrollScanActions(onScan = hit("scan"), onChannel = { calls += "ch:$it" }),
            )
        }
        exists("This phone will be called pixel-8. The phone checks the name again when the terminal's code is read.")
        absent("Name accepted: pixel-8")
        rule.onNodeWithContentDescription("Name this phone will have among your devices").assertExists()
        click("Bluetooth")
        click("Paste text")
        click("Sound")
        click("Scan the terminal's code")
        assertEquals(listOf("ch:Bluetooth", "ch:Paste", "ch:Sound", "scan"), calls)
    }

    @Test fun scan_invalid_name_disables_every_forward_action_and_names_the_reason() {
        rule.setScreen { EnrollScanScreen(S.scanInvalid(), EnrollScanActions(onScan = hit("scan"))) }
        exists("Enter a name of 1 to 40 characters.")
        rule.onNodeWithText("Scan the terminal's code").assertIsNotEnabled()
    }

    @Test fun scan_scanning_shows_progress_and_cancel() {
        rule.setScreen { EnrollScanScreen(S.scanning(), EnrollScanActions(onCancelScan = hit("cancel"))) }
        exists("frame 2 of 4 read")
        exists("fake camera")
        absent("Scan the terminal's code")
        click("Cancel")
        assertEquals(listOf("cancel"), calls)
    }

    @Test fun scan_camera_denied_offers_the_permission_not_a_dead_end() {
        rule.setScreen {
            EnrollScanScreen(
                S.scanning().let { it.copy(receive = it.receive.copy(camera = io.github.petyok.sshub.core.CameraPermission.Denied)) },
                EnrollScanActions(receive = io.github.petyok.sshub.ui.screens.enroll.EnrollReceiveActions(onRequestCamera = hit("allow"))),
            )
        }
        click("Allow camera")
        assertEquals(listOf("allow"), calls)
    }

    @Test fun scan_bluetooth_channel_button() {
        rule.setScreen { EnrollScanScreen(S.scan(EnrollChannel.Bluetooth), EnrollScanActions(onFindTerminal = hit("find"))) }
        click("Find the terminal")
        assertEquals(listOf("find"), calls)
    }

    @Test fun scan_paste_channel_button() {
        rule.setScreen { EnrollScanScreen(S.scan(EnrollChannel.Paste), EnrollScanActions(onPasteText = hit("paste"))) }
        click("Paste the text")
        assertEquals(listOf("paste"), calls)
    }

    // ---- 2 Show ----

    @Test fun show_main_path_qr_text_copy_and_next() {
        rule.setScreen {
            EnrollShowScreen(S.show(), EnrollShowActions(onNext = hit("next"), send = EnrollSendActions(onCopy = hit("copy"), onToggleText = hit("toggle"))))
        }
        rule.onNodeWithContentDescription(QR_DESCRIPTION).assertExists()
        rule.onNode(hasText("of 6", substring = true)).assertExists()
        exists("Marked sensitive: the system hides it from clipboard previews and it is cleared after 60 s.")
        rule.onNodeWithText("Copy the text").performScrollTo().performClick()
        click("Show as text for a terminal without a camera")
        click("Next")
        assertEquals(listOf("copy", "toggle", "next"), calls)
    }

    @Test fun show_folded_hides_the_text_and_copy() {
        rule.setScreen { EnrollShowScreen(S.show(textOpen = false), EnrollShowActions()) }
        absent("Copy the text")
        rule.onNodeWithContentDescription(QR_DESCRIPTION).assertExists()
    }

    @Test fun d19_copy_shows_the_clear_countdown_as_text() {
        rule.setScreen { EnrollShowScreen(withClipboard(), EnrollShowActions()) }
        exists("Copied, clears in 60 s. Paste it into the terminal.")
        exists("Copied")
    }

    private fun withClipboard() = S.show().let { it.copy(send = it.send.copy(clipboardClearsIn = 60)) }

    // ---- 3 Keys ----

    @Test fun keys_wait_has_a_disabled_next_with_the_reason() {
        rule.setScreen { EnrollKeysScreen(S.keys(KeysStatus.Waiting), EnrollKeysActions(onCompare = hit("compare"))) }
        rule.onNodeWithText("Next").assertIsNotEnabled()
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Disabled, waiting for the keys"))
        exists("Waiting for the keys. Then the phone checks them against the first code.")
    }

    @Test fun keys_ok_goes_to_the_words() {
        rule.setScreen { EnrollKeysScreen(S.keys(KeysStatus.Ok), EnrollKeysActions(onCompare = hit("compare"), onBack = hit("back"))) }
        exists("The keys match the first code. Terminal calls itself laptop-main [7B3E-0D15].")
        click("Compare the words")
        rule.onNodeWithContentDescription("Back").performClick()
        assertEquals(listOf("compare", "back"), calls)
    }

    @Test fun d14_keys_mismatch_stops_and_offers_only_a_way_back() {
        rule.setScreen { EnrollKeysScreen(S.keys(KeysStatus.Mismatch), EnrollKeysActions(onCompare = hit("compare"), onStartAgain = hit("again"))) }
        exists("The keys do not match the first code.")
        absent("Compare the words")
        absent("Next")
        rule.onNodeWithContentDescription("The terminal showed first", substring = true).assertExists()
        rule.onNodeWithContentDescription("The terminal now shows", substring = true).assertExists()
        click("Start again")
        assertEquals(listOf("again"), calls)
    }

    @Test fun m7_a_mismatch_without_compared_values_draws_no_empty_boxes() {
        rule.setScreen { EnrollKeysScreen(S.keys(KeysStatus.Mismatch).copy(committed = "", shown = ""), EnrollKeysActions(onStartAgain = hit("again"))) }
        exists("The keys do not match the first code.")
        rule.onAllNodes(androidx.compose.ui.test.hasContentDescription("The terminal showed first", substring = true)).assertCountEquals(0)
        rule.onAllNodes(androidx.compose.ui.test.hasContentDescription("The terminal now shows", substring = true)).assertCountEquals(0)
        absent("The terminal showed first")
    }

    @Test fun m7_the_mismatch_box_draws_both_values_in_full_in_groups_of_4_and_says_what_it_means() {
        rule.setScreen { EnrollKeysScreen(S.keys(KeysStatus.Mismatch), EnrollKeysActions()) }
        exists("The terminal showed first")
        exists("The terminal now shows")
        rule.onNodeWithContentDescription("The terminal showed first " + spokenHex(hexGroups(S.committed))).assertExists()
        rule.onNodeWithContentDescription("The terminal now shows " + spokenHex(hexGroups(S.shown))).assertExists()
        exists("These two values must be the same. They are not: someone may be interfering. Nothing was enrolled. Do not retry on this channel; start again on the terminal.")
    }

    @Test fun m7_another_stop_is_titled_with_the_cores_sentence_not_as_a_mismatch() {
        rule.setScreen {
            EnrollKeysScreen(S.keys(KeysStatus.Stopped).copy(committed = "", shown = "", reason = "The terminal presented unusable keys."), EnrollKeysActions(onStartAgain = hit("again")))
        }
        exists("The terminal presented unusable keys.")
        absent("The keys do not match the first code.")
        click("Start again")
        assertEquals(listOf("again"), calls)
    }

    // ---- 4 Compare ----

    @Test fun compare_shows_the_four_words_whole_and_reports_the_choice() {
        rule.setScreen { EnrollCompareScreen(S.compare(), EnrollCompareActions(onMatch = hit("match"), onDiffer = hit("differ"))) }
        rule.onNodeWithContentDescription("triceps, sanding, pursuit, aliens").assertExists()
        click("The words match")
        click("They differ")
        assertEquals(listOf("match", "differ"), calls)
    }

    @Test fun d15_both_buttons_enabled_none_selected() {
        rule.setScreen { EnrollCompareScreen(S.compare(), EnrollCompareActions()) }
        for (label in listOf("They differ", "The words match")) {
            rule.onNodeWithText(label).assertIsEnabled().assert(notSelectable)
        }
        absent("Continue")
    }

    @Test fun compare_match_continues_to_the_confirmation() {
        rule.setScreen { EnrollCompareScreen(S.compare(ComparePhase.Match), EnrollCompareActions(onContinue = hit("continue"))) }
        exists("You confirmed the words.")
        absent("Words confirmed.")
        absent("They differ")
        absent("The words match")
        click("Continue")
        assertEquals(listOf("continue"), calls)
    }

    @Test fun d14_compare_differ_is_a_stop_state_with_only_start_again() {
        rule.setScreen { EnrollCompareScreen(S.compare(ComparePhase.Differ), EnrollCompareActions(onStartAgain = hit("again"))) }
        exists("Stop. The words differ.")
        absent("Continue")
        absent("The words match")
        absent("They differ")
        rule.onNodeWithContentDescription(QR_DESCRIPTION).assertExists()
        click("Start again")
        assertEquals(listOf("again"), calls)
    }

    @Test fun d18_compare_sets_flag_secure() {
        rule.setScreen { EnrollCompareScreen(S.compare(), EnrollCompareActions()) }
        rule.runOnIdle {
            assertTrue((rule.activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE) != 0)
        }
    }

    // ---- 5 Confirm ----

    @Test fun confirm_waiting_has_no_enabled_forward_action() {
        rule.setScreen { EnrollConfirmScreen(S.confirm(ConfirmStatus.Waiting), EnrollConfirmActions(onFinish = hit("finish"))) }
        rule.onNodeWithText("Waiting for laptop-main").assertIsNotEnabled()
        rule.onNodeWithContentDescription(QR_DESCRIPTION).assertExists()
        exists("laptop-main confirmation")
        absent("Finish")
    }

    @Test fun confirm_bad_signature_is_a_stop_with_only_start_again() {
        rule.setScreen { EnrollConfirmScreen(S.confirm(ConfirmStatus.Bad).copy(kind = io.github.petyok.sshub.core.RefusalKind.BadSignature), EnrollConfirmActions(onStartAgain = hit("again"), onFinish = hit("finish"))) }
        exists("The terminal confirmation has a bad signature.")
        absent("Finish")
        click("Start again")
        assertEquals(listOf("again"), calls)
    }

    @Test fun confirm_refusal_title_follows_the_kind() {
        rule.setScreen { EnrollConfirmScreen(S.confirm(ConfirmStatus.Bad).copy(kind = io.github.petyok.sshub.core.RefusalKind.Declined), EnrollConfirmActions()) }
        exists("The terminal declined.")
        absent("The terminal confirmation has a bad signature.")
    }

    @Test fun confirm_refusal_of_another_kind_has_the_generic_title() {
        rule.setScreen { EnrollConfirmScreen(S.confirm(ConfirmStatus.Bad), EnrollConfirmActions()) }
        exists("The terminal confirmation was refused.")
    }

    @Test fun confirm_ok_finishes() {
        rule.setScreen { EnrollConfirmScreen(S.confirm(ConfirmStatus.Ok), EnrollConfirmActions(onFinish = hit("finish"))) }
        exists("The terminal confirmation is received and its signature is good.")
        click("Finish")
        assertEquals(listOf("finish"), calls)
    }

    // ---- done ----

    @Test fun done_says_promotion_is_a_separate_step() {
        rule.setScreen { EnrolledScreen(S.enrolled(), EnrolledActions(onDone = hit("done"))) }
        exists("This phone is now known to laptop-main [7B3E-0D15] as pixel-8.")
        exists("Making this phone a full device is a separate step on the terminal. Until then the terminal knows this phone but does not yet count it as a full device.")
        click("Done")
        assertEquals(listOf("done"), calls)
    }

    // ---- Bluetooth ----

    @Test fun bt_searching_can_be_cancelled() {
        rule.setScreen { EnrollBtScreen(S.bt(BtPhase.Searching), EnrollBtActions(onBack = hit("cancel"))) }
        exists("Looking for devices nearby")
        click("Cancel")
        assertEquals(listOf("cancel"), calls)
    }

    @Test fun bt_found_user_picks_the_row_with_the_tag_and_pairs() {
        rule.setScreen {
            EnrollBtScreen(S.bt(BtPhase.Found, selected = 0), EnrollBtActions(onPair = hit("pair"), onSelect = { calls += "sel:$it" }))
        }
        exists("matches tag")
        click("SSHub-3XD9")
        click("Pair with SSHub-7K2Q")
        assertEquals(listOf("sel:1", "pair"), calls)
    }

    @Test fun bt_mismatch_disables_pair_and_says_which_tag_was_picked() {
        rule.setScreen { EnrollBtScreen(S.bt(BtPhase.Found, selected = 1), EnrollBtActions(onPair = hit("pair"))) }
        exists("Tag 3XD9 is not the tag you typed (7K2Q).")
        rule.onNodeWithText("Pair").assertIsNotEnabled()
    }

    @Test fun bt_found_without_a_selection_cannot_pair() {
        rule.setScreen { EnrollBtScreen(S.bt(BtPhase.Found), EnrollBtActions()) }
        rule.onNodeWithText("Pair").assertIsNotEnabled()
    }

    @Test fun bt_tag_is_typed_in_the_field() {
        rule.setScreen { EnrollBtScreen(S.bt(BtPhase.Searching, tag = ""), EnrollBtActions(onTag = { calls += "tag:$it" })) }
        exists("Type the 4-character tag shown on laptop-main.")
        rule.onNodeWithContentDescription("Tag shown on laptop-main").performTextInput("7K")
        assertEquals(listOf("tag:7K"), calls)
    }

    @Test fun bt_none_offers_search_again_and_other_channel() {
        rule.setScreen { EnrollBtScreen(S.bt(BtPhase.None, tag = "ZZ9Z"), EnrollBtActions(onSearchAgain = hit("again"), onOtherChannel = hit("other"))) }
        exists("No SSHub device with tag ZZ9Z found")
        click("Search again")
        click("Other channel")
        assertEquals(listOf("again", "other"), calls)
    }

    @Test fun bt_denied_asks_for_the_permission() {
        rule.setScreen { EnrollBtScreen(S.bt(BtPhase.Denied), EnrollBtActions(onAllow = hit("allow"))) }
        exists("Bluetooth permission was not granted")
        click("Allow Bluetooth")
        assertEquals(listOf("allow"), calls)
    }

    @Test fun bt_off_says_so_and_offers_another_channel() {
        rule.setScreen { EnrollBtScreen(S.bt(BtPhase.Off), EnrollBtActions(onOtherChannel = hit("other"), onSearchAgain = hit("again"))) }
        exists("Bluetooth is off")
        click("Search again")
        assertEquals(listOf("again"), calls)
    }

    @Test fun bt_connecting_waits_for_the_terminal_code() {
        rule.setScreen { EnrollBtScreen(S.bt(BtPhase.Connecting), EnrollBtActions(onBack = hit("cancel"))) }
        exists("Linking with SSHub-7K2Q")
        click("Cancel")
        assertEquals(listOf("cancel"), calls)
    }

    // ---- Paste ----

    @Test fun paste_empty_cannot_continue() {
        rule.setScreen { EnrollPasteScreen(S.paste(1, PasteVerdict.Empty), EnrollPasteActions(onUse = hit("use"), onPaste = hit("paste"), onClear = hit("clear"))) }
        exists("Waiting for text. Step 1 needs the terminal's short code.")
        rule.onNodeWithText("Use this text").assertIsNotEnabled()
        click("Paste")
        click("Clear")
        assertEquals(listOf("paste", "clear"), calls)
    }

    @Test fun paste_ok_continues() {
        rule.setScreen { EnrollPasteScreen(S.paste(1, PasteVerdict.Ok), EnrollPasteActions(onUse = hit("use"))) }
        exists("Looks like step 1 frame.")
        click("Use this text")
        assertEquals(listOf("use"), calls)
    }

    @Test fun paste_bad_names_what_the_text_is_and_blocks() {
        rule.setScreen { EnrollPasteScreen(S.paste(1, PasteVerdict.Bad), EnrollPasteActions(onUse = hit("use"))) }
        exists("Not what this step needs: this looks like the step 3 frame.")
        rule.onNodeWithText("Use this text").assertIsNotEnabled()
    }

    @Test fun paste_step_2_copies_the_phone_code_and_goes_next() {
        rule.setScreen { EnrollPasteScreen(S.paste(2), EnrollPasteActions(onNext = hit("next"), send = EnrollSendActions(onCopy = hit("copy")))) }
        exists("This step sends nothing to this phone.")
        click("Copy the text")
        click("Next")
        assertEquals(listOf("copy", "next"), calls)
    }

    @Test fun paste_step_3_and_5_labels() {
        rule.setScreen { EnrollPasteScreen(S.paste(3, PasteVerdict.Ok), EnrollPasteActions(onUse = hit("use"))) }
        exists("Nothing to copy at this step.")
        click("Check the keys")
        assertEquals(listOf("use"), calls)
    }

    @Test fun paste_step_5_checks_the_signature() {
        rule.setScreen { EnrollPasteScreen(S.paste(5, PasteVerdict.Ok), EnrollPasteActions(onUse = hit("use"))) }
        click("Check the signature")
        assertEquals(listOf("use"), calls)
    }

    // ---- Sound ----

    @Test fun sound_idle_plays_the_chosen_profile() {
        rule.setScreen { EnrollSoundScreen(S.sound(SoundPhase.Idle), EnrollSoundActions(onContinue = hit("next"), send = EnrollSendActions(onPlay = hit("play"), onProfile = { calls += "p:$it" }))) }
        exists("Ready")
        exists("of about 8 s")
        click("Slow")
        click("Play")
        click("Next")
        assertEquals(listOf("p:Slow", "play", "next"), calls)
    }

    @Test fun sound_playing_shows_progress_and_stop_and_locks_the_profile() {
        rule.setScreen { EnrollSoundScreen(S.sound(SoundPhase.Playing), EnrollSoundActions(send = EnrollSendActions(onStop = hit("stop"), onProfile = { calls += "p:$it" }))) }
        exists("Playing")
        exists("3 s")
        click("Slow")
        click("Stop")
        assertEquals(listOf("stop"), calls)
    }

    @Test fun sound_done_offers_play_again() {
        rule.setScreen { EnrollSoundScreen(S.sound(SoundPhase.Done), EnrollSoundActions(send = EnrollSendActions(onPlay = hit("play")))) }
        exists("Played")
        click("Play again")
        assertEquals(listOf("play"), calls)
    }

    @Test fun sound_slow_profile_is_about_sixteen_seconds() {
        rule.setScreen { EnrollSoundScreen(S.sound(SoundPhase.Idle, SoundProfile.Slow), EnrollSoundActions()) }
        exists("of about 16 s")
        exists("slow profile")
    }

    @Test fun sound_without_audio_disables_play() {
        rule.setScreen { EnrollSoundScreen(S.sound(SoundPhase.Idle, available = false), EnrollSoundActions()) }
        rule.onNodeWithText("Play").assertIsNotEnabled()
    }

    @Test fun sound_screen_says_the_phone_never_records() {
        rule.setScreen { EnrollSoundScreen(S.sound(), EnrollSoundActions()) }
        exists("Hold the phone near laptop-main while it plays, then tap Next. This phone never records: the terminal does the listening.")
    }

    // ---- cross cutting ----

    @Test fun d11_time_passing_never_continues_or_decides() {
        rule.mainClock.autoAdvance = false
        rule.setScreen {
            EnrollCompareScreen(S.compare(), EnrollCompareActions(onMatch = hit("match"), onDiffer = hit("differ"), onContinue = hit("continue"), onBack = hit("back")))
        }
        rule.mainClock.advanceTimeBy(30 * 60 * 1000L)
        assertTrue(calls.isEmpty())
        rule.onNodeWithText("The words match").assertIsEnabled()
        rule.onNodeWithText("They differ").assertIsEnabled()
    }

    @Test fun light_theme_renders_the_compare_screen() {
        rule.setScreen(dark = false) { EnrollCompareScreen(S.compare(), EnrollCompareActions()) }
        exists("The words match")
    }

    @Test fun font_scale_two_keeps_the_compare_buttons_reachable() {
        rule.setScreen(fontScale = 2f) { EnrollCompareScreen(S.compare(), EnrollCompareActions(onMatch = hit("match"))) }
        click("The words match")
        assertEquals(listOf("match"), calls)
    }

    @Test fun glossary_every_screen_says_step_n_of_5_with_the_glossary_names() {
        rule.setScreen { EnrollShowScreen(S.show(), EnrollShowActions()) }
        exists("Connect to laptop-main")
        exists("Step 2 of 5")
        rule.onNodeWithContentDescription("Step 2 of 5, Phone code").assertExists()
        rule.onNode(hasText("Terminal code"), useUnmergedTree = true).assertExists()
        rule.onNode(hasText("Compare the words"), useUnmergedTree = true).assertExists()
        rule.onNode(hasText("Finish"), useUnmergedTree = true).assertExists()
    }
}
