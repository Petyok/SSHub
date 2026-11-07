package uk.petruha.sshub.phone.identity

import android.text.InputType
import android.view.inputmethod.EditorInfo
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.fake.IdentitySamples
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.identity.RestoreActions
import uk.petruha.sshub.phone.ui.screens.identity.RestoreScreen

/** M12: the 24 word boxes and the words paste box ask the keyboard not to learn: the input variation is "password". */
class WordsKeyboardTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun focusedEditorInfo(): EditorInfo {
        rule.waitForIdle()
        var info: EditorInfo? = null
        rule.activityRule.scenario.onActivity { a ->
            val v = a.currentFocus
            assertNotNull("a text field has the focus", v)
            info = EditorInfo().also { v!!.onCreateInputConnection(it) }
        }
        return info!!
    }

    private fun variation(i: EditorInfo) = i.inputType and InputType.TYPE_MASK_VARIATION

    @Test fun a_word_box_uses_the_password_variation() {
        rule.setScreen { RestoreScreen(IdentitySamples.restoreEmpty, RestoreActions()) }
        rule.onNodeWithContentDescription("Word 1").performClick()
        val i = focusedEditorInfo()
        assertEquals(InputType.TYPE_CLASS_TEXT, i.inputType and InputType.TYPE_MASK_CLASS)
        assertEquals(InputType.TYPE_TEXT_VARIATION_PASSWORD, variation(i))
    }

    @Test fun the_words_paste_box_uses_the_password_variation() {
        rule.setScreen { RestoreScreen(IdentitySamples.restoreEmpty, RestoreActions()) }
        rule.onNodeWithContentDescription("Pasted words").performScrollTo().performClick()
        assertEquals(InputType.TYPE_TEXT_VARIATION_PASSWORD, variation(focusedEditorInfo()))
    }
}
