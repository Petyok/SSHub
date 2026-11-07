package uk.petruha.sshub.phone.testing

import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.onRoot
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
    snapshot()
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
    assertNoBrokenWords()
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

/**
 * A word broken inside a token ("RED / OPERATI / ON", "laptop-mai / n") is a defect at the largest font size: the line breaks where no
 * whitespace is, although the token alone would have fitted a line. A token longer than every line (a 64 digit hash in plain text)
 * has to break somewhere and is not reported.
 */
fun ComposeContentTestRule.assertNoBrokenWords() {
    val bad = mutableListOf<String>()
    onAllNodes(androidx.compose.ui.test.SemanticsMatcher("has text") { it.config.contains(androidx.compose.ui.semantics.SemanticsProperties.Text) })
        .fetchSemanticsNodes().forEach { n ->
            val r = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
            n.config.getOrNull(androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult)?.action?.invoke(r)
            r.forEach { l -> bad += l.brokenTokens() }
        }
    check(bad.isEmpty()) { "words broken inside a token: " + bad.take(6).joinToString(" ; ") }
}

private fun androidx.compose.ui.text.TextLayoutResult.brokenTokens(): List<String> {
    val text = layoutInput.text.text
    val widest = (0 until lineCount).maxOfOrNull { getLineEnd(it) - getLineStart(it) } ?: return emptyList()
    return (0 until lineCount - 1).mapNotNull { i ->
        val o = getLineEnd(i)
        // After a hyphen is a legal break point; inside a word is not.
        if (o <= 0 || o >= text.length || text[o - 1].isWhitespace() || text[o].isWhitespace() || text[o - 1] == '-') return@mapNotNull null
        var a = o; while (a > 0 && !text[a - 1].isWhitespace()) a--
        var b = o; while (b < text.length && !text[b].isWhitespace()) b++
        text.substring(a, b).takeIf { it.length <= widest }
    }
}

/**
 * Writes a PNG of the whole screen to <additionalTestOutputDir>/shots/<TestClass>.<method>.png (AGP pulls it; android/ci/build.sh copies it to /out).
 * Called by assertTargets48, which every screen test ends with, so each tested state leaves a picture to compare with the
 * prototype screenshots. A failure to capture never fails a test.
 */
fun ComposeContentTestRule.snapshot() {
    runCatching {
        val frame = Throwable().stackTrace.firstOrNull { it.className.startsWith("uk.petruha.sshub.phone.") && !it.className.contains(".testing.") }
            ?: return
        val name = frame.className.substringAfterLast('.') + "." + frame.methodName
        // AGP pulls additionalTestOutputDir into app/build/outputs/connected_android_test_additional_output after the run.
        val out = androidx.test.platform.app.InstrumentationRegistry.getArguments().getString("additionalTestOutputDir")
        val dir = java.io.File(out ?: return).resolve("shots").apply { mkdirs() }
        val bmp = onRoot().captureToImage().asAndroidBitmap()
        java.io.File(dir, "$name.png").outputStream().use { bmp.compress(android.graphics.Bitmap.CompressFormat.PNG, 90, it) }
    }
}
