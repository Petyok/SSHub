package uk.petruha.sshub.phone.components

import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.ui.components.QrMatrix
import uk.petruha.sshub.phone.ui.components.QrView
import uk.petruha.sshub.phone.ui.components.qrModulePx

class QrViewTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val matrix = QrMatrix(25, BooleanArray(25 * 25) { it % 3 == 0 })
    private val total = 25 + 8

    @Test
    fun modulePxIsWholeFloor() {
        assertEquals(9, qrModulePx(300, 33))
        assertEquals(9, qrModulePx(329, 33))
        assertEquals(10, qrModulePx(330, 33))
        assertEquals(1, qrModulePx(10, 33))
    }

    private fun fillsWidthAt(density: Float) {
        rule.setContent {
            CompositionLocalProvider(LocalDensity provides Density(density)) {
                Box(Modifier.width(300.dp)) { QrView(matrix, "qr") }
            }
        }
        val w = rule.onNodeWithContentDescription("qr").fetchSemanticsNode().size.width
        val module = qrModulePx(w, total)
        assertEquals(300 * density, w.toFloat(), 1f)
        // The drawn matrix is a whole number of modules and leaves less than one module of slack.
        assertTrue(w - module * total in 0 until module)
        // Same bound in millimetres (160 dpi = 25.4 mm per 160 px at density 1).
        val mm = { px: Int -> px / (density * 160f) * 25.4f }
        assertTrue(mm(w) - mm(module * total) < mm(module) + 0.01f)
    }

    @Test fun fillsWidthAtDensity2() = fillsWidthAt(2f)

    @Test fun fillsWidthAtDensity3() = fillsWidthAt(3f)

    @Test
    fun screenStaysBrightAndAwakeUntilLastQrLeaves() {
        var one by mutableStateOf(true)
        var two by mutableStateOf(true)
        rule.setContent {
            if (one) QrView(matrix, "one")
            if (two) QrView(matrix, "two")
        }
        val window = rule.activity.window
        fun flag() = window.attributes.flags and WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON != 0
        rule.waitForIdle()
        assertTrue(flag())
        assertEquals(1f, window.attributes.screenBrightness, 0f)
        one = false
        rule.waitForIdle()
        assertTrue(flag())
        assertEquals(1f, window.attributes.screenBrightness, 0f)
        two = false
        rule.waitForIdle()
        assertTrue(!flag())
        assertEquals(WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE, window.attributes.screenBrightness, 0f)
    }

    @Test
    fun theScreenHoldEndsAfterTheBoundSoTheNormalTimeoutAndAutoLockApplyAgain() {
        rule.setContent { QrView(matrix, "timed", holdMs = 400) }
        val window = rule.activity.window
        fun flag() = window.attributes.flags and WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON != 0
        rule.waitForIdle()
        assertTrue(flag())
        rule.mainClock.advanceTimeBy(1_000) // the effect's delay runs on the test clock
        rule.waitForIdle()
        assertTrue("the hold must end after the bound", !flag())
        assertEquals(WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE, window.attributes.screenBrightness, 0f)
    }

    @Test
    fun inLandscapeTheCodeIsNoTallerThanTheScreenCap() {
        rule.setContent {
            val cfg = android.content.res.Configuration(androidx.compose.ui.platform.LocalConfiguration.current).also { it.screenHeightDp = 360 }
            CompositionLocalProvider(androidx.compose.ui.platform.LocalConfiguration provides cfg) {
                Box(Modifier.width(700.dp)) { QrView(matrix, "wide") }
            }
        }
        val node = rule.onNodeWithContentDescription("wide").fetchSemanticsNode()
        val density = rule.activity.resources.displayMetrics.density
        assertTrue("height ${node.size.height} must be at most 360 dp", node.size.height <= 360 * density + 1)
        assertEquals("still square", node.size.width, node.size.height)
    }
}
