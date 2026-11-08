package uk.petruha.sshub.phone.release

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.fake.ReleaseSamples
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.release.RedActions
import uk.petruha.sshub.phone.ui.screens.release.RedScreen
import uk.petruha.sshub.phone.ui.screens.release.RedState
import uk.petruha.sshub.phone.ui.screens.release.ReleaseActions
import uk.petruha.sshub.phone.ui.screens.release.ShareReleaseScreen

/** U13-8: one plain sentence on top of every approval; the raw fields are behind "Technical details". */
class PlainSentenceTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val sentence = "Make laptop-2 a full device: it will hold one piece of every saved password. Asked by laptop-1."

    @Test fun red_shows_the_sentence_first_hides_the_raw_fields_and_never_lamport_or_seq() {
        val v = ReleaseSamples.redRevoke.copy(plain = sentence)
        rule.setScreen(true, 1f) { RedScreen(RedState(v, v.proposedAt + 5), RedActions()) }
        rule.onNodeWithText(sentence).assertExists()
        rule.onNodeWithText("lamport", substring = true).assertDoesNotExist()
        rule.onNodeWithText("seq", substring = true).assertDoesNotExist()
        rule.onNodeWithText("Changes").assertDoesNotExist()
        rule.onNodeWithText("Technical details", substring = true).performClick()
        rule.onNodeWithText("Changes").assertExists()
    }

    @Test fun red_without_a_sentence_keeps_the_fields_open() {
        val v = ReleaseSamples.redRevoke
        rule.setScreen(true, 1f) { RedScreen(RedState(v, v.proposedAt + 5), RedActions()) }
        rule.onNodeWithText("Changes").assertExists()
    }

    @Test fun release_shows_the_sentence_and_folds_the_op_rows() {
        val v = ReleaseSamples.releaseDbs.copy(plain = "Give laptop-2 your piece of the saved password 0A0A 0A0A.")
        rule.setScreen(true, 1f) { ShareReleaseScreen(ReleaseSamples.releaseState(v), ReleaseActions()) }
        rule.onNodeWithText("Give laptop-2 your piece of the saved password 0A0A 0A0A.").assertExists()
        rule.onNodeWithText("What you release").assertDoesNotExist()
    }
}
