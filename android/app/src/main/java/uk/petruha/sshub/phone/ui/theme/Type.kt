package uk.petruha.sshub.phone.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import uk.petruha.sshub.phone.R

// Spec 4. Fonts are bundled in res/font (JetBrains Mono 2.304, Inter 4.1, SIL OFL 1.1); nothing is downloaded.
val JetBrainsMono = FontFamily(
    Font(R.font.jetbrains_mono_regular, FontWeight.Normal),
    Font(R.font.jetbrains_mono_medium, FontWeight.Medium),
    Font(R.font.jetbrains_mono_bold, FontWeight.Bold),
    Font(R.font.jetbrains_mono_extrabold, FontWeight.ExtraBold),
)
val Inter = FontFamily(
    Font(R.font.inter_regular, FontWeight.Normal),
    Font(R.font.inter_medium, FontWeight.Medium),
    Font(R.font.inter_semibold, FontWeight.SemiBold),
    Font(R.font.inter_bold, FontWeight.Bold),
)

// Ligatures are off on every mono style: a `->` or `==` inside an id would be wrong.
private fun mono(w: FontWeight, size: Int, lh: Int, ls: Float = 0f) = TextStyle(
    fontFamily = JetBrainsMono, fontWeight = w, fontSize = size.sp, lineHeight = lh.sp,
    letterSpacing = ls.sp, fontFeatureSettings = "calt 0, liga 0",
)
private fun inter(w: FontWeight, size: Int, lh: Int) = TextStyle(
    fontFamily = Inter, fontWeight = w, fontSize = size.sp, lineHeight = lh.sp,
)

val SshubTypography = Typography(
    displaySmall = mono(FontWeight.Bold, 26, 32, 1.0f),
    headlineSmall = mono(FontWeight.Bold, 22, 28, 0.9f),
    titleLarge = inter(FontWeight.Bold, 20, 26),
    titleMedium = mono(FontWeight.ExtraBold, 17, 24, 1.0f),
    titleSmall = inter(FontWeight.SemiBold, 18, 24),
    labelLarge = inter(FontWeight.SemiBold, 15, 20),
    bodyLarge = mono(FontWeight.Bold, 16, 22),
    bodyMedium = inter(FontWeight.Normal, 14, 20),
    bodySmall = inter(FontWeight.Normal, 13, 18),
    labelMedium = mono(FontWeight.Normal, 12, 16),
    labelSmall = mono(FontWeight.Normal, 12, 16, 0.2f),
)

object SshubText {
    val monoBody = mono(FontWeight.SemiBold, 14, 20)
    val button = mono(FontWeight.Bold, 15, 20)
    val buttonSub = mono(FontWeight.Normal, 12, 16)
    val input = mono(FontWeight.Bold, 20, 26, 1.6f)
    val hex = mono(FontWeight.Medium, 16, 22)
    val fieldKey = mono(FontWeight.Normal, 12, 16)
    val bigWord = mono(FontWeight.Bold, 26, 32, 1.0f)
    val keycap = mono(FontWeight.Bold, 22, 28, 0.9f)
    val deviceName = mono(FontWeight.Bold, 16, 22)
    val secondaryMono = mono(FontWeight.Normal, 13, 18)
}
