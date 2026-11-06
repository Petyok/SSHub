package uk.petruha.sshub.phone.recovery

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.fake.RecoverySamples
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.components.groupedUid
import uk.petruha.sshub.phone.ui.components.hexGroups
import uk.petruha.sshub.phone.ui.components.spokenHex
import uk.petruha.sshub.phone.ui.screens.recovery.ExportActions
import uk.petruha.sshub.phone.ui.screens.recovery.ExportOpsScreen
import uk.petruha.sshub.phone.ui.screens.recovery.ExportState

class ExportOpsScreenTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun show(state: ExportState, on: ExportActions = ExportActions(), dark: Boolean = true, fontScale: Float = 1f) =
        rule.setScreen(dark, fontScale) { ExportOpsScreen(state, on) }

    @Test fun lists_the_newest_eight_ops_and_counts_them() {
        show(RecoverySamples.exportState)
        rule.onNodeWithText("Newest entries").assertExists()
        rule.onNodeWithText("8 of 41 ops").assertExists()
        RecoverySamples.exportRows.forEach { r ->
            val danger = if (r.danger) ", danger" else ""
            rule.onNodeWithContentDescription("Operation ${r.seq}, ${r.kind}, ${r.time}$danger, hash ${spokenHex(hexGroups(r.hash))}").assertExists()
        }
        // L6: a dangerous op carries the word, a plain one does not
        rule.onAllNodesWithText("danger").assertCountEquals(RecoverySamples.exportRows.count { it.danger })
        // D4: the hash is shown in groups of 4
        rule.onNodeWithText(groupedUid(RecoverySamples.exportRows.first().hash), useUnmergedTree = true).assertExists()
        rule.onAllNodes(hasContentDescription("Operation ", substring = true)).assertCountEquals(8)
    }

    @Test fun shows_qr_text_and_the_sensitive_notice() {
        show(RecoverySamples.exportState)
        rule.onNode(hasContentDescription("QR code with the newest signed ops", substring = true)).assertExists()
        rule.onNodeWithContentDescription("Signed text", substring = true).assertExists()
        rule.onNodeWithText("Copying marks the text as sensitive. It leaves the clipboard after 60 s.").assertExists()
        rule.onNodeWithText("The newest 8 signed entries", substring = true).assertExists()
    }

    @Test fun copy_and_done_call_back() {
        var copied = 0
        var done = 0
        show(RecoverySamples.exportState, ExportActions(onCopy = { copied++ }, onDone = { done++ }))
        rule.onNodeWithText("for a terminal without a camera").assertExists()
        rule.button("Copy the text").performClick()
        rule.button("Done").performClick()
        assertEquals(1, copied)
        assertEquals(1, done)
    }

    @Test fun after_copy_it_says_copied_and_when_the_clipboard_clears() {
        show(RecoverySamples.exportState.copy(copied = true, clearsIn = 60))
        rule.button("Copied").assertExists()
        rule.onNodeWithText("paste it into the terminal").assertExists()
        rule.onNodeWithText("Copied, clears in 60 s").assertExists()
    }

    @Test fun nothing_is_said_about_the_clipboard_before_a_copy() {
        show(RecoverySamples.exportState)
        rule.onNodeWithText("Copied, clears in", substring = true).assertDoesNotExist()
    }

    @Test fun no_ops_yet_is_an_empty_state_with_only_done() {
        var done = 0
        show(RecoverySamples.exportEmpty, ExportActions(onDone = { done++ }))
        rule.onNodeWithText("No signed entries").assertExists()
        rule.onNodeWithText("Copy the text").assertDoesNotExist()
        rule.button("Done").performClick()
        assertEquals(1, done)
    }

    @Test fun looping_qr_with_several_frames_still_renders() {
        show(RecoverySamples.exportState.copy(outgoing = RecoverySamples.outgoing(3, frames = 3)))
        rule.onNode(hasContentDescription("QR code with the newest signed ops", substring = true)).assertExists()
    }

    @Test fun dark_theme() {
        show(RecoverySamples.exportState, dark = true)
        rule.onNodeWithText("Export my ops as text").assertExists()
        rule.assertTargets48()
    }

    @Test fun light_theme() {
        show(RecoverySamples.exportState, dark = false)
        rule.assertTargets48()
    }

    @Test fun font_scale_2_has_no_overflow() {
        show(RecoverySamples.exportState.copy(copied = true, clearsIn = 60), fontScale = 2f)
        rule.assertNoVisualOverflow()
        rule.assertTargets48()
    }
}
