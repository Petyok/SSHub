package io.github.petyok.sshub.identity

import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.assertHasNoClickAction
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import io.github.petyok.sshub.core.ThemeChoice
import io.github.petyok.sshub.fake.IdentitySamples
import io.github.petyok.sshub.testing.assertNoVisualOverflow
import io.github.petyok.sshub.testing.assertTargets48
import io.github.petyok.sshub.testing.setScreen
import io.github.petyok.sshub.ui.screens.identity.AboutActions
import io.github.petyok.sshub.ui.screens.identity.AboutSecurityScreen
import io.github.petyok.sshub.ui.screens.identity.ChainHeadActions
import io.github.petyok.sshub.ui.screens.identity.ChainHeadPhase
import io.github.petyok.sshub.ui.screens.identity.ChainHeadState
import io.github.petyok.sshub.ui.screens.identity.FingerprintCompare
import io.github.petyok.sshub.ui.screens.identity.SecurityMenuActions
import io.github.petyok.sshub.ui.screens.identity.SecurityMenuScreen
import io.github.petyok.sshub.ui.screens.identity.TakeChainScreen
import io.github.petyok.sshub.ui.screens.identity.compareFingerprint
import io.github.petyok.sshub.ui.screens.identity.normalizeFingerprint

class TakeChainScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun main_path_scan_the_list_then_paste_text() {
        var scan = 0; var paste = 0
        rule.setScreen { TakeChainScreen(IdentitySamples.chainHeadStart, ChainHeadActions(onScan = { scan++ }, onOpenPaste = { paste++ })) }
        rule.onNodeWithText("This phone does not sign yet.").assertExists()
        rule.onNodeWithText("One list is not enough.").assertExists()
        rule.onNodeWithText("Scan the list").performClick()
        rule.onNodeWithText("Paste text").performClick()
        assertEquals(1, scan); assertEquals(1, paste)
        rule.assertTargets48()
    }

    @Test fun scan_variant_one_list_read_needs_a_second() {
        rule.setScreen { TakeChainScreen(IdentitySamples.chainHeadOneList, ChainHeadActions()) }
        rule.onNodeWithText("List read, one more needed.").assertExists()
        rule.onNodeWithText(IdentitySamples.SUMMARY).assertExists()
        rule.onNodeWithText(IdentitySamples.NOTE_NEXT).assertExists()
        rule.onNodeWithText("Take the next list").assertExists()
        rule.onAllNodesWithText("Adopt", substring = true).assertCountEquals(0)
        rule.onNodeWithText("Checks on the last list").assertExists()
        rule.onNodeWithText("4 of 4 pass").assertExists()
    }

    @Test fun the_second_list_must_come_from_a_different_terminal() {
        rule.setScreen { TakeChainScreen(IdentitySamples.chainHeadStart, ChainHeadActions()) }
        rule.onNodeWithText("Take a second list from a different terminal. The same terminal twice does not count.", substring = true).assertExists()
        rule.onAllNodesWithText("from the same one later", substring = true).assertCountEquals(0)
    }

    @Test fun the_sender_of_each_list_is_shown_in_groups_of_four_or_said_to_be_missing() {
        val s = IdentitySamples.chainHeadAgree.copy(senders = listOf("7b3e0d15aa", null))
        rule.setScreen { TakeChainScreen(s, ChainHeadActions()) }
        rule.onNodeWithText("List from terminal 7b3e 0d15 aa").assertExists()
        rule.onNodeWithText("This list does not say which terminal made it.").assertExists()
        rule.onNodeWithText("not proof of who sent it", substring = true).assertExists()
    }

    @Test fun agree_variant_adopts_with_the_seq_in_the_label() {
        var adopt = 0; var home = 0
        rule.setScreen { TakeChainScreen(IdentitySamples.chainHeadAgree, ChainHeadActions(onAdopt = { adopt++ }, onHome = { home++ })) }
        rule.onNodeWithText("Two lists agree").assertExists()
        rule.onNodeWithText("Compare with a terminal first.").assertExists()
        rule.onNodeWithText(IdentitySamples.SUMMARY).assertExists()
        rule.onNodeWithText(IdentitySamples.NOTE_AGREED).assertExists()
        rule.onNodeWithText("Adopt: continue after seq 41").assertIsEnabled().performClick()
        rule.onNodeWithText("Not yet").performClick()
        assertEquals(1, adopt); assertEquals(1, home)
        rule.assertTargets48()
    }

    @Test fun single_source_card_shows_the_count_and_age_and_the_tick_gates_adopt() {
        var ticks = 0; var adopt = 0
        rule.setScreen { TakeChainScreen(IdentitySamples.chainHeadSingle, ChainHeadActions(onAdopt = { adopt++ }, onTickSingle = { ticks++ })) }
        rule.onNodeWithText("One list only").assertExists()
        rule.onAllNodesWithText("Two lists agree").assertCountEquals(0)
        rule.onNodeWithText("The terminal says your ring has 3 terminals.", substring = true).assertExists()
        rule.onNodeWithText("signed 2 hours 0 minutes ago", substring = true).assertExists()
        rule.onNodeWithText("Your ring has 3 terminals and I can reach only this one").performScrollTo().performClick()
        assertEquals(1, ticks)
        rule.onNodeWithText("Adopt: continue after seq 41").assertIsNotEnabled()
        rule.onAllNodesWithText("trusted", substring = true).assertCountEquals(0)
    }

    @Test fun single_source_ticked_enables_adopt() {
        var adopt = 0
        rule.setScreen { TakeChainScreen(IdentitySamples.chainHeadSingle.copy(singleTicked = true), ChainHeadActions(onAdopt = { adopt++ })) }
        rule.onNodeWithText("Adopt: continue after seq 41").assertIsEnabled().performClick()
        assertEquals(1, adopt)
    }

    @Test fun single_source_tick_text_for_a_one_terminal_ring() {
        rule.setScreen { TakeChainScreen(IdentitySamples.chainHeadSingleOneTerminal, ChainHeadActions()) }
        rule.onNodeWithText("Your ring has one terminal").assertExists()
    }

    @Test fun paste_variant_check_needs_a_long_enough_text() {
        var typed = ""; var check = 0
        rule.setScreen {
            var text by remember { mutableStateOf("") }
            TakeChainScreen(
                IdentitySamples.chainHeadPaste.copy(pasteText = text),
                ChainHeadActions(onPasteText = { text = it; typed = it }, onCheckPaste = { check++ }),
            )
        }
        rule.onNodeWithText("Check the list").assertIsNotEnabled()
        rule.onNodeWithContentDescription("Pasted ops list").performTextInput("LIST 0123456789abcdef")
        assertEquals("LIST 0123456789abcdef", typed)
        rule.onNodeWithText("Check the list").assertIsEnabled().performClick()
        assertEquals(1, check)
    }

    @Test fun ok_variant_taken_shows_the_banner_and_the_page_warning() {
        var home = 0
        rule.setScreen { TakeChainScreen(IdentitySamples.chainHeadTaken, ChainHeadActions(onHome = { home++ })) }
        rule.onNodeWithText("CHAIN TAKEN").assertExists()
        rule.onNodeWithText("Chain taken. Continuing after seq 41").assertExists()
        rule.onNodeWithText("One copy only").assertExists()
        rule.onNodeWithText("Go to Home").performClick()
        assertEquals(1, home)
    }

    @Test fun ok_variant_never_signed_text() {
        rule.setScreen { TakeChainScreen(IdentitySamples.chainHeadNeverSigned, ChainHeadActions()) }
        rule.onNodeWithText("Confirmed. This key never signed anything.").assertExists()
        rule.onNodeWithText("no seq", substring = true).assertDoesNotExist()
    }

    @Test fun fail_variant_is_a_stop_box_without_any_adopt_action() {
        var again = 0
        rule.setScreen { TakeChainScreen(IdentitySamples.chainHeadRefused, ChainHeadActions(onTryAnother = { again++ })) }
        rule.onNodeWithText("This list was refused").assertExists()
        rule.onAllNodesWithText("Adopt", substring = true).assertCountEquals(0)
        rule.onAllNodesWithText("Approve", substring = true).assertCountEquals(0)
        rule.onNodeWithText("1 of 4 pass").assertExists()
        rule.onNodeWithText("The ops form one chain: not checked").assertExists()
        rule.onNodeWithText("Try another list").performClick()
        assertEquals(1, again)
    }

    @Test fun d13_never_signed_is_disabled_until_the_typed_id_matches() {
        var never = 0
        rule.setScreen {
            var typed by remember { mutableStateOf("") }
            TakeChainScreen(IdentitySamples.chainHeadStart.copy(typedId = typed), ChainHeadActions(onTypedId = { typed = it }, onNeverSigned = { never++ }))
        }
        rule.onNodeWithText("This key never signed anything").assertIsNotEnabled()
        rule.onNodeWithContentDescription("Type the device id to confirm").performTextInput("4F2A-91C0")
        rule.onNodeWithText("This key never signed anything").assertIsNotEnabled()
        assertEquals(0, never)
    }

    @Test fun d13_the_gate_in_the_state_is_exact_and_case_blind() {
        val s = IdentitySamples.chainHeadStart
        assertFalse(s.copy(typedId = "").canNeverSigned)
        assertFalse(s.copy(typedId = "4F2A-91C0").canNeverSigned)
        assertTrue(s.copy(typedId = "4f2a-91c7").canNeverSigned)
        assertTrue(s.copy(typedId = " 4F2A-91C7 ").canNeverSigned)
        assertFalse(ChainHeadState(typedId = "").canNeverSigned)
    }

    @Test fun never_signed_button_enables_with_the_right_id() {
        var never = 0
        rule.setScreen { TakeChainScreen(IdentitySamples.chainHeadStart.copy(typedId = "4F2A-91C7"), ChainHeadActions(onNeverSigned = { never++ })) }
        rule.onNodeWithText("This key never signed anything").assertIsEnabled().performClick()
        assertEquals(1, never)
    }

    @Test fun empty_summary_and_note_render_no_line() {
        val bare = IdentitySamples.chainHeadOneList.copy(chain = IdentitySamples.chain(false, summary = "", note = ""))
        rule.setScreen { TakeChainScreen(bare, ChainHeadActions()) }
        rule.onNodeWithText("List read, one more needed.").assertExists()
        rule.onAllNodesWithText("Newest op in this list", substring = true).assertCountEquals(0)
        rule.onAllNodesWithText("Take a second list", substring = true).assertCountEquals(0)
    }

    @Test fun empty_note_on_agreed_lists_has_no_warning_box() {
        val agreed = IdentitySamples.chainHeadAgree.copy(chain = IdentitySamples.chain(true, summary = "", note = ""))
        rule.setScreen { TakeChainScreen(agreed, ChainHeadActions()) }
        rule.onNodeWithText("Two lists agree").assertExists()
        rule.onAllNodesWithText("Compare with a terminal first.").assertCountEquals(0)
        rule.onAllNodesWithText("Newest op in this list", substring = true).assertCountEquals(0)
    }

    @Test fun a_notice_shows_for_an_ignored_frame() {
        rule.setScreen { TakeChainScreen(IdentitySamples.chainHeadStart.copy(notice = "Ignored: not an ops list"), ChainHeadActions()) }
        rule.onNodeWithText("Ignored: not an ops list").assertExists()
    }

    @Test fun light_theme_renders() {
        rule.setScreen(dark = false) { TakeChainScreen(IdentitySamples.chainHeadAgree, ChainHeadActions()) }
        rule.onNodeWithText("Two lists agree").assertExists()
    }

    @Test fun font_scale_2_has_no_visual_overflow_in_every_variant() {
        val all = listOf(
            IdentitySamples.chainHeadStart, IdentitySamples.chainHeadOneList, IdentitySamples.chainHeadAgree,
            IdentitySamples.chainHeadPaste, IdentitySamples.chainHeadTaken, IdentitySamples.chainHeadRefused,
        )
        var i by mutableStateOf(0)
        rule.setScreen(fontScale = 2f) { TakeChainScreen(all[i], ChainHeadActions()) }
        for (n in all.indices) {
            i = n
            rule.waitForIdle()
            rule.assertNoVisualOverflow()
            rule.assertTargets48()
        }
    }
}

class SecurityMenuScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun main_path_every_row_calls_back() {
        val hit = mutableListOf<String>()
        rule.setScreen {
            SecurityMenuScreen(
                IdentitySamples.menu,
                SecurityMenuActions(
                    onTrustList = { hit += "trust" }, onCodes = { hit += "codes" }, onReshare = { hit += "reshare" }, onRecovery = { hit += "recovery" }, onNewPhone = { hit += "newphone" },
                    onBackup = { hit += "backup" }, onExport = { hit += "export" }, onAbout = { hit += "about" }, onLock = { hit += "lock" },
                ),
            )
        }
        for (t in listOf("Trust sessions", "Short codes I issued", "Update the shares", "I am getting a new phone", "I lost my phone", "Recovery words", "Export my ops as text", "About and security", "Lock now")) {
            rule.onNodeWithText(t).performScrollTo().performClick()
        }
        assertEquals(listOf("trust", "codes", "reshare", "newphone", "recovery", "backup", "export", "about", "lock"), hit)
        rule.assertTargets48()
    }

    @Test fun theme_choice_marks_the_current_one_and_calls_back() {
        var chosen: ThemeChoice? = null
        rule.setScreen { SecurityMenuScreen(IdentitySamples.menu, SecurityMenuActions(onTheme = { chosen = it })) }
        rule.onNodeWithText("Dark").assertIsSelected()
        rule.onNodeWithText("Light").performScrollTo().performClick()
        assertEquals(ThemeChoice.Light, chosen)
        rule.onNodeWithText("System").performScrollTo().performClick()
        assertEquals(ThemeChoice.System, chosen)
    }

    @Test fun auto_lock_choice_offers_30_seconds_to_5_minutes() {
        var chosen = 0
        rule.setScreen { SecurityMenuScreen(IdentitySamples.menu, SecurityMenuActions(onAutoLock = { chosen = it })) }
        rule.onNodeWithText("1 min").assertIsSelected()
        rule.onNodeWithText("30 s").performScrollTo().performClick()
        assertEquals(30, chosen)
        rule.onNodeWithText("5 min").performScrollTo().performClick()
        assertEquals(300, chosen)
    }

    @Test fun backup_row_says_when_the_paper_copy_is_confirmed() {
        rule.setScreen { SecurityMenuScreen(IdentitySamples.menu.copy(backupConfirmed = true), SecurityMenuActions()) }
        rule.onNodeWithText("24 words, confirmed on paper. Shown after a fresh unlock.").assertExists()
    }

    @Test fun d17_blocked_shows_a_stop_box_and_closes_every_flow_but_about_and_lock() {
        var trust = 0; var about = 0; var lock = 0
        rule.setScreen {
            SecurityMenuScreen(IdentitySamples.menuBlocked, SecurityMenuActions(onTrustList = { trust++ }, onAbout = { about++ }, onLock = { lock++ }))
        }
        rule.onNodeWithText("Signing fingerprint differs").assertExists()
        rule.onNodeWithText("Get the app again from the release notes", substring = true).assertExists()
        rule.onNodeWithText("you will need your 24 words", substring = true).assertExists()
        for (t in listOf("Trust sessions", "Short codes I issued", "Update the shares", "I am getting a new phone", "I lost my phone", "Recovery words", "Export my ops as text")) {
            rule.onNodeWithText(t).assertHasNoClickAction()
        }
        rule.onNodeWithText("About and security").performScrollTo().assertHasClickAction().performClick()
        rule.onNodeWithText("Lock now").performScrollTo().performClick()
        assertEquals(0, trust); assertEquals(1, about); assertEquals(1, lock)
    }

    @Test fun light_theme_renders() {
        rule.setScreen(dark = false) { SecurityMenuScreen(IdentitySamples.menu, SecurityMenuActions()) }
        rule.onNodeWithText("Security").assertExists()
    }

    @Test fun font_scale_2_has_no_visual_overflow() {
        rule.setScreen(fontScale = 2f) { SecurityMenuScreen(IdentitySamples.menuBlocked, SecurityMenuActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }

    @Test fun font_scale_2_choices_stack_as_rows() {
        var chosen: ThemeChoice? = null
        rule.setScreen(fontScale = 2f) { SecurityMenuScreen(IdentitySamples.menu, SecurityMenuActions(onTheme = { chosen = it })) }
        rule.onNodeWithText("Light").performScrollTo().performClick()
        assertEquals(ThemeChoice.Light, chosen)
    }
}

class AboutSecurityScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun main_path_shows_the_version_and_16_groups_and_compare_toggles() {
        var toggled = 0
        rule.setScreen { AboutSecurityScreen(IdentitySamples.about, AboutActions(onToggleCompare = { toggled++ })) }
        rule.onNodeWithText("0.1.0 (build 1)", substring = true).assertExists()
        rule.onNodeWithContentDescription("Group 1 of 16, E 3 2 8").assertExists()
        rule.onNodeWithContentDescription("Group 16 of 16, 9 D 7 C").assertExists()
        rule.onNodeWithText("Compare with terminal").performScrollTo().performClick()
        assertEquals(1, toggled)
        rule.assertTargets48()
    }

    @Test fun marking_a_group_calls_back() {
        var marked = -1
        rule.setScreen { AboutSecurityScreen(IdentitySamples.aboutComparingEqual, AboutActions(onToggleGroup = { marked = it })) }
        rule.onNodeWithContentDescription("Group 5 of 16, 7 8 C 8").performClick()
        assertEquals(4, marked)
        rule.onNodeWithText("3 of 16 groups marked", substring = true).assertExists()
    }

    @Test fun equal_value_reads_matches_terminal_with_a_check_line() {
        rule.setScreen { AboutSecurityScreen(IdentitySamples.aboutComparingEqual, AboutActions()) }
        rule.onNodeWithText("matches terminal").assertExists()
        rule.onNodeWithText("All 16 groups equal the value of the terminal.").assertExists()
        rule.onAllNodesWithText("Signing fingerprint mismatch").assertCountEquals(0)
        rule.onNodeWithText("Stop comparing").assertExists()
    }

    @Test fun incomplete_value_counts_digits_and_decides_nothing() {
        rule.setScreen { AboutSecurityScreen(IdentitySamples.aboutComparingPartial, AboutActions()) }
        rule.onNodeWithText("20 of 64 digits typed.").assertExists()
        rule.onAllNodesWithText("Signing fingerprint mismatch").assertCountEquals(0)
        rule.onAllNodesWithText("matches terminal").assertCountEquals(0)
    }

    @Test fun reference_input_calls_back() {
        var got = ""
        rule.setScreen { AboutSecurityScreen(IdentitySamples.about.copy(comparing = true), AboutActions(onReference = { got = it })) }
        rule.onNodeWithContentDescription("Fingerprint printed by the terminal").performTextInput("e328 9279")
        assertEquals("e328 9279", got)
    }

    @Test fun d17_a_mismatching_fingerprint_is_a_stop_box_with_both_values_and_no_way_forward() {
        rule.setScreen { AboutSecurityScreen(IdentitySamples.aboutComparingDiffer, AboutActions()) }
        rule.onNodeWithText("Signing fingerprint mismatch").assertExists()
        rule.onNodeWithText("DIFFERS from terminal").assertExists()
        rule.onNodeWithText("Groups that differ: 4").assertExists()
        rule.onNodeWithContentDescription("App fingerprint", substring = true).assertExists()
        rule.onNodeWithContentDescription("Terminal fingerprint", substring = true).assertExists()
        // no "continue anyway", and the compare toggle that could hide the stop box is gone
        rule.onAllNodesWithText("Stop comparing").assertCountEquals(0)
        rule.onAllNodesWithText("Compare with terminal").assertCountEquals(0)
        rule.onAllNodesWithText("continue", substring = true, ignoreCase = true).assertCountEquals(0)
    }

    @Test fun d17_the_block_button_needs_the_tick_and_the_warning_says_what_closing_costs() {
        var confirmed = false
        rule.setScreen {
            AboutSecurityScreen(
                IdentitySamples.aboutComparingDiffer.copy(confirmed = confirmed),
                AboutActions(onConfirmed = { confirmed = it }),
            )
        }
        rule.onNodeWithText("Closing the app means installing it again.", substring = true).assertExists()
        rule.onNodeWithText("you will need your 24 words", substring = true).assertExists()
        rule.onNodeWithText("Get the app again from the release notes", substring = true).assertExists()
        rule.onNodeWithText("These differ: close the app").performScrollTo().assertIsNotEnabled()
        rule.onNodeWithText("I compared the value digit by digit").performScrollTo().performClick()
        assertTrue(confirmed)
    }

    @Test fun d17_a_ticked_block_button_calls_the_block() {
        var block = 0
        rule.setScreen { AboutSecurityScreen(IdentitySamples.aboutComparingDiffer.copy(confirmed = true), AboutActions(onBlock = { block++ })) }
        rule.onNodeWithText("These differ: close the app").performScrollTo().assertIsEnabled().performClick()
        assertEquals(1, block)
    }

    @Test fun d17_while_blocked_a_matching_value_offers_the_way_out() {
        var open = 0
        rule.setScreen { AboutSecurityScreen(IdentitySamples.aboutComparingEqual.copy(blocked = true), AboutActions(onUnblock = { open++ })) }
        rule.onNodeWithText("Closing the app means installing it again.", substring = true).assertExists()
        rule.onNodeWithText("I typed it wrong earlier: open the app again").performScrollTo().performClick()
        assertEquals(1, open)
    }

    @Test fun d17_while_blocked_and_differing_there_is_no_way_out_and_no_block_button() {
        rule.setScreen { AboutSecurityScreen(IdentitySamples.aboutComparingDiffer.copy(blocked = true), AboutActions()) }
        rule.onAllNodesWithText("I typed it wrong earlier", substring = true).assertCountEquals(0)
        rule.onAllNodesWithText("These differ: close the app").assertCountEquals(0)
    }

    @Test fun d17_a_refused_system_lock_is_shown_as_text() {
        rule.setScreen { AboutSecurityScreen(IdentitySamples.aboutComparingDiffer.copy(message = "The system prompt was canceled."), AboutActions()) }
        rule.onNodeWithText("The system prompt was canceled.", substring = true).performScrollTo().assertExists()
    }

    @Test fun d17_compare_logic() {
        val app = IdentitySamples.FINGERPRINT
        assertEquals(FingerprintCompare.None, compareFingerprint(app, ""))
        assertEquals(FingerprintCompare.Incomplete, compareFingerprint(app, app.take(10)))
        assertEquals(FingerprintCompare.Invalid, compareFingerprint(app, "zz" + app.drop(2)))
        assertEquals(FingerprintCompare.Equal, compareFingerprint(app, app.uppercase()))
        assertEquals(FingerprintCompare.Equal, compareFingerprint(app, app.chunked(4).joinToString(" ")))
        assertEquals(FingerprintCompare.Equal, compareFingerprint(app, app.chunked(2).joinToString(":")))
        assertEquals(FingerprintCompare.Differ, compareFingerprint(app, IdentitySamples.OTHER_FINGERPRINT))
        assertEquals(FingerprintCompare.Differ, compareFingerprint(app, app + "0"))
        assertEquals(app, normalizeFingerprint(app.chunked(4).joinToString(" ").uppercase()))
    }

    @Test fun licences_row_shows_both_bundled_texts() {
        var toggled = 0
        rule.setScreen { AboutSecurityScreen(IdentitySamples.about, AboutActions(onToggleLicences = { toggled++ })) }
        rule.onAllNodesWithText("licence text of", substring = true).assertCountEquals(0)
        rule.onNodeWithText("Licences").performScrollTo().performClick()
        assertEquals(1, toggled)
    }

    @Test fun open_licences_list_both_texts_with_copyright_and_source() {
        rule.setScreen { AboutSecurityScreen(IdentitySamples.aboutLicences, AboutActions()) }
        rule.onNodeWithText("licence text of JetBrainsMono-OFL.txt").assertExists()
        rule.onNodeWithText("licence text of Inter-LICENSE.txt").assertExists()
        rule.onNodeWithText("Copyright 2020 The JetBrains Mono Project Authors").assertExists()
        rule.onNodeWithText("https://github.com/rsms/inter").assertExists()
    }

    @Test fun strongbox_state_is_stated_both_ways() {
        rule.setScreen { AboutSecurityScreen(IdentitySamples.about.copy(strongBox = true), AboutActions()) }
        rule.onNodeWithText("StrongBox, a separate security chip").assertExists()
    }

    @Test fun no_strongbox_is_stated() {
        rule.setScreen { AboutSecurityScreen(IdentitySamples.about.copy(strongBox = false), AboutActions()) }
        rule.onNodeWithText("Trusted execution environment, no StrongBox chip on this phone").assertExists()
    }

    @Test fun states_what_the_app_is_not() {
        rule.setScreen { AboutSecurityScreen(IdentitySamples.about, AboutActions()) }
        rule.onNodeWithText("What this app is not").assertExists()
        rule.onNodeWithText("Not a password manager.", substring = true).assertExists()
    }

    @Test fun recovery_row_opens_the_words() {
        var backup = 0
        rule.setScreen { AboutSecurityScreen(IdentitySamples.about, AboutActions(onBackup = { backup++ })) }
        rule.onNodeWithText("Recovery words").performScrollTo().performClick()
        assertEquals(1, backup)
    }

    @Test fun d18_the_window_is_secure_because_a_full_hash_is_shown() {
        rule.setScreen { AboutSecurityScreen(IdentitySamples.about, AboutActions()) }
        rule.waitForIdle()
        var flags = 0
        rule.runOnUiThread { flags = rule.activity.window.attributes.flags }
        assertTrue("FLAG_SECURE", flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
    }

    @Test fun light_theme_renders_the_mismatch() {
        rule.setScreen(dark = false) { AboutSecurityScreen(IdentitySamples.aboutComparingDiffer, AboutActions()) }
        rule.onNodeWithText("Signing fingerprint mismatch").assertExists()
    }

    @Test fun font_scale_2_has_no_visual_overflow_with_the_mismatch_and_the_licences() {
        rule.setScreen(fontScale = 2f) { AboutSecurityScreen(IdentitySamples.aboutComparingDiffer.copy(licencesOpen = true, licences = IdentitySamples.licences), AboutActions()) }
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }
}
