package uk.petruha.sshub.phone.ui.theme

import android.content.Context
import android.database.ContentObserver
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp

// Tokens of docs: host-sync-plan/android-design/spec-A.md section 2. This is the ONLY file that may contain Color(0x...).
// CI greps for it (android/ci/lint.sh).

@Immutable
class SshubColors(
    val danger: Color, val onDanger: Color, val dangerText: Color,
    val dangerBg: Color, val dangerBg2: Color,
    val diffOld: Color, val diffOldGutter: Color, val diffNew: Color, val diffNewGutter: Color,
    val ok: Color, val onOk: Color,
    val focus: Color, val onFocus: Color, val focusText: Color,
    val tickLit: Color, val tickDim: Color, val boundary: Color, val keyEdge: Color,
)

val DarkSshub = SshubColors(
    danger = Color(0xFFFF6B5E), onDanger = Color(0xFF1C0707), dangerText = Color(0xFFFF8A80),
    dangerBg = Color(0xFF3A1D20), dangerBg2 = Color(0xFF58242A),
    diffOld = Color(0xFF3A1D20), diffOldGutter = Color(0xFF58242A),
    diffNew = Color(0xFF58242A), diffNewGutter = Color(0xFFFF6B5E),
    ok = Color(0xFF5FC98A), onOk = Color(0xFF07180E),
    focus = Color(0xFFE0A84A), onFocus = Color(0xFF1A1305), focusText = Color(0xFFE0A84A),
    tickLit = Color(0xFFE8EAEC), tickDim = Color(0xFF6F7381), boundary = Color(0xFF6F7381),
    keyEdge = Color(0xFF0B0C0F),
)
val LightSshub = SshubColors(
    danger = Color(0xFFB3261E), onDanger = Color(0xFFFFFFFF), dangerText = Color(0xFFA8211A),
    dangerBg = Color(0xFFFDECEB), dangerBg2 = Color(0xFFF8CDC8),
    diffOld = Color(0xFFFDECEB), diffOldGutter = Color(0xFFF8CDC8),
    diffNew = Color(0xFFF8CDC8), diffNewGutter = Color(0xFFB3261E),
    ok = Color(0xFF1D7A46), onOk = Color(0xFFFFFFFF),
    focus = Color(0xFFE0A84A), onFocus = Color(0xFF1A1305), focusText = Color(0xFF7D5200),
    tickLit = Color(0xFF17181C), tickDim = Color(0xFF808078), boundary = Color(0xFF808078),
    keyEdge = Color(0xFFB4B4AB),
)
val LocalSshubColors = staticCompositionLocalOf<SshubColors> { error("SshubTheme missing") }
/** Spec 8: true when the system animator scale is 0. Every animation spec reads it. */
val LocalReducedMotion = staticCompositionLocalOf { false }

private val DarkScheme = darkColorScheme(
    primary = Color(0xFFE0A84A), onPrimary = Color(0xFF1A1305),
    primaryContainer = Color(0xFFE0A84A), onPrimaryContainer = Color(0xFF1A1305),
    inversePrimary = Color(0xFFE0A84A),
    secondary = Color(0xFFE8EAEC), onSecondary = Color(0xFF14151A),
    secondaryContainer = Color(0xFF25262D), onSecondaryContainer = Color(0xFFE8EAEC),
    tertiary = Color(0xFF5FC98A), onTertiary = Color(0xFF07180E),
    tertiaryContainer = Color(0xFF25262D), onTertiaryContainer = Color(0xFFE8EAEC),
    error = Color(0xFFFF6B5E), onError = Color(0xFF1C0707),
    errorContainer = Color(0xFF3A1D20), onErrorContainer = Color(0xFFE8EAEC),
    background = Color(0xFF14151A), onBackground = Color(0xFFE8EAEC),
    surface = Color(0xFF14151A), onSurface = Color(0xFFE8EAEC),
    surfaceVariant = Color(0xFF25262D), onSurfaceVariant = Color(0xFFA6AAB3),
    surfaceTint = Color(0xFF14151A),
    inverseSurface = Color(0xFFE8EAEC), inverseOnSurface = Color(0xFF14151A),
    outline = Color(0xFFA6AAB3), outlineVariant = Color(0xFF363842),
    scrim = Color(0xFF000000),
    surfaceDim = Color(0xFF14151A), surfaceBright = Color(0xFF1C1D23),
    surfaceContainerLowest = Color(0xFF14151A), surfaceContainerLow = Color(0xFF1C1D23),
    surfaceContainer = Color(0xFF1C1D23), surfaceContainerHigh = Color(0xFF25262D),
    surfaceContainerHighest = Color(0xFF25262D),
)
private val LightScheme = lightColorScheme(
    primary = Color(0xFFE0A84A), onPrimary = Color(0xFF1A1305),
    primaryContainer = Color(0xFFE0A84A), onPrimaryContainer = Color(0xFF1A1305),
    inversePrimary = Color(0xFF7D5200),
    secondary = Color(0xFF17181C), onSecondary = Color(0xFFF3F3F0),
    secondaryContainer = Color(0xFFEBEBE6), onSecondaryContainer = Color(0xFF17181C),
    tertiary = Color(0xFF1D7A46), onTertiary = Color(0xFFFFFFFF),
    tertiaryContainer = Color(0xFFEBEBE6), onTertiaryContainer = Color(0xFF17181C),
    error = Color(0xFFB3261E), onError = Color(0xFFFFFFFF),
    errorContainer = Color(0xFFFDECEB), onErrorContainer = Color(0xFF17181C),
    background = Color(0xFFF3F3F0), onBackground = Color(0xFF17181C),
    surface = Color(0xFFF3F3F0), onSurface = Color(0xFF17181C),
    surfaceVariant = Color(0xFFEBEBE6), onSurfaceVariant = Color(0xFF545861),
    surfaceTint = Color(0xFFF3F3F0),
    inverseSurface = Color(0xFF17181C), inverseOnSurface = Color(0xFFF3F3F0),
    outline = Color(0xFF545861), outlineVariant = Color(0xFFCDCDC5),
    scrim = Color(0xFF000000),
    surfaceDim = Color(0xFFF3F3F0), surfaceBright = Color(0xFFFFFFFF),
    surfaceContainerLowest = Color(0xFFF3F3F0), surfaceContainerLow = Color(0xFFFFFFFF),
    surfaceContainer = Color(0xFFFFFFFF), surfaceContainerHigh = Color(0xFFEBEBE6),
    surfaceContainerHighest = Color(0xFFEBEBE6),
)

val SshubShapes = Shapes(
    extraSmall = RoundedCornerShape(4.dp), small = RoundedCornerShape(8.dp),
    medium = RoundedCornerShape(12.dp), large = RoundedCornerShape(16.dp),
    extraLarge = RoundedCornerShape(24.dp),
)
object SshubShape {
    val square = RoundedCornerShape(6.dp)
    val field = RoundedCornerShape(10.dp)
}

/** Dark by default; `dark` is resolved by the caller from the in-app choice (system / dark / light). */
@Composable
fun SshubTheme(dark: Boolean, reducedMotion: Boolean = rememberReducedMotion(), content: @Composable () -> Unit) {
    CompositionLocalProvider(
        LocalSshubColors provides if (dark) DarkSshub else LightSshub,
        LocalReducedMotion provides reducedMotion,
    ) {
        MaterialTheme(
            colorScheme = if (dark) DarkScheme else LightScheme,
            typography = SshubTypography, shapes = SshubShapes, content = content,
        )
    }
}

val MaterialTheme.sshub: SshubColors @Composable get() = LocalSshubColors.current

/** Observes Settings.Global.ANIMATOR_DURATION_SCALE (spec 8). */
@Composable
fun rememberReducedMotion(): Boolean {
    val ctx = LocalContext.current
    var reduced by remember { mutableStateOf(animatorScaleIsZero(ctx)) }
    DisposableEffect(ctx) {
        val obs = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) { reduced = animatorScaleIsZero(ctx) }
        }
        ctx.contentResolver.registerContentObserver(Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE), false, obs)
        onDispose { ctx.contentResolver.unregisterContentObserver(obs) }
    }
    return reduced
}

private fun animatorScaleIsZero(ctx: Context): Boolean =
    Settings.Global.getFloat(ctx.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f
