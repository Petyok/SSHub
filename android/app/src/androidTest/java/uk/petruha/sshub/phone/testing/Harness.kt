package uk.petruha.sshub.phone.testing

import androidx.compose.runtime.Composable
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.unit.Density
import uk.petruha.sshub.phone.core.LocalPlatform
import uk.petruha.sshub.phone.core.Platform
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.ui.theme.SshubTheme

/** Renders `content` in the app theme with a fake platform, at a given theme and font scale (spec 4.3: test at 1.0, 1.3, 2.0). */
fun ComposeContentTestRule.setScreen(
    dark: Boolean = true,
    fontScale: Float = 1f,
    platform: Platform = FakePlatform(),
    content: @Composable () -> Unit,
) = setContent {
    val d = LocalDensity.current
    CompositionLocalProvider(LocalPlatform provides platform, LocalDensity provides Density(d.density, fontScale)) {
        SshubTheme(dark = dark, reducedMotion = false) { content() }
    }
}

/** Spec 9: every clickable node is at least 48 x 48 dp. Call at the end of a screen test. */
fun ComposeContentTestRule.assertTargets48() {
    val density = androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().targetContext.resources.displayMetrics.density
    val min = 48f * density - 1f
    val bad = onAllNodes(androidx.compose.ui.test.hasClickAction()).fetchSemanticsNodes().filter { it.size.width < min || it.size.height < min }
    check(bad.isEmpty()) { "clickable nodes under 48dp: " + bad.joinToString { n ->
        val t = n.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.Text)?.joinToString("|") { it.text }
        val d = n.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.ContentDescription)?.joinToString("|")
        "[${t ?: d}] ${n.size.width}x${n.size.height}px"
    } }
}

/** Spec 4.3 / D4: no text on the screen is clipped. Reports which flag fired, for the first few offenders. */
fun ComposeContentTestRule.assertNoVisualOverflow() {
    val bad = mutableListOf<String>()
    onAllNodes(androidx.compose.ui.test.SemanticsMatcher("has text") { it.config.contains(androidx.compose.ui.semantics.SemanticsProperties.Text) })
        .fetchSemanticsNodes().forEach { n ->
            val r = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
            n.config.getOrNull(androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult)?.action?.invoke(r)
            r.filter { it.reallyClipped() }.forEach { l ->
                val t = n.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.Text)?.joinToString("|") { it.text }?.take(30)
                bad += "[$t] w=${l.didOverflowWidth} h=${l.didOverflowHeight} lines=${l.lineCount} size=${l.size} max=${l.layoutInput.constraints.maxWidth}x${l.layoutInput.constraints.maxHeight} ml=${l.layoutInput.maxLines}"
            }
        }
    check(bad.isEmpty()) { "texts with visual overflow: ${bad.size}: " + bad.take(4).joinToString(" ; ") }
}

/**
 * `hasVisualOverflow` also fires when the laid-out size is narrower than the paragraph even though the text fits its constraints
 * (seen with a density override at fontScale 2.0: a 266px label in a 996px slot). Real clipping is: the text is wider or taller than its
 * slot, or a line is ellipsized.
 */
private fun androidx.compose.ui.text.TextLayoutResult.reallyClipped(): Boolean {
    val c = layoutInput.constraints
    val tooWide = multiParagraph.width > c.maxWidth + 1 || size.width > c.maxWidth + 1
    val tooTall = c.hasBoundedHeight && size.height > c.maxHeight + 1
    val ellipsized = (0 until lineCount).any { isLineEllipsized(it) }
    return tooWide || tooTall || ellipsized
}
