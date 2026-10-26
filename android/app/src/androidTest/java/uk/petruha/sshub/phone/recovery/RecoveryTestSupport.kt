package uk.petruha.sshub.phone.recovery

import android.view.WindowManager
import androidx.compose.ui.test.assert
import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.SemanticsNodeInteractionsProvider
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import org.junit.Assert.assertEquals

/** A button is found by its label; labels used on one screen are unique. */
fun SemanticsNodeInteractionsProvider.button(label: String): SemanticsNodeInteraction = onNode(hasText(label))

/** A disabled button is still present, not enabled, and names the reason (spec 9). */
fun SemanticsNodeInteraction.assertDisabled(reason: String): SemanticsNodeInteraction {
    assertIsNotEnabled()
    return assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Disabled, $reason"))
}

/** D18: FLAG_SECURE of the activity window is set (or clear) once the screen is idle. */
fun ComposeContentTestRule.assertSecure(activity: ComponentActivity, secure: Boolean = true) {
    runOnIdle {
        val set = (activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE) != 0
        assertEquals("FLAG_SECURE", secure, set)
    }
}
