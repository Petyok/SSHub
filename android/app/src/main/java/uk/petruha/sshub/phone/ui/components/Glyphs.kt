package uk.petruha.sshub.phone.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** Canvas icons, 2dp stroke, rounded (spec 7.9). No icon font, no vector assets: nothing to fetch, one colour parameter. */
enum class GlyphKind { Check, Cross, Warning, Stop, Phone, Back, Chevron, Ring, Approvals, Enroll, Security, Scan, Copy, Close, Info, Lock, Bluetooth, Sound, Paste, Qr }

@Composable
fun Glyph(kind: GlyphKind, color: Color, modifier: Modifier = Modifier, size: Dp = 24.dp, stroke: Dp = 2.dp) {
    Canvas(modifier.size(size)) { drawGlyph(kind, color, stroke.toPx()) }
}

fun DrawScope.drawGlyph(kind: GlyphKind, color: Color, sw: Float) {
    val w = size.width
    val h = size.height
    val st = Stroke(sw, cap = StrokeCap.Round, join = StrokeJoin.Round)
    fun p(x: Float, y: Float) = Offset(x * w, y * h)
    fun poly(vararg pts: Pair<Float, Float>, close: Boolean = false) {
        val path = Path()
        pts.forEachIndexed { i, (x, y) -> if (i == 0) path.moveTo(x * w, y * h) else path.lineTo(x * w, y * h) }
        if (close) path.close()
        drawPath(path, color, style = st)
    }
    when (kind) {
        GlyphKind.Check -> poly(0.2f to 0.55f, 0.42f to 0.76f, 0.8f to 0.28f)
        GlyphKind.Cross -> { poly(0.25f to 0.25f, 0.75f to 0.75f); poly(0.75f to 0.25f, 0.25f to 0.75f) }
        GlyphKind.Close -> { poly(0.25f to 0.25f, 0.75f to 0.75f); poly(0.75f to 0.25f, 0.25f to 0.75f) }
        GlyphKind.Warning -> {
            poly(0.5f to 0.12f, 0.92f to 0.86f, 0.08f to 0.86f, close = true)
            drawLine(color, p(0.5f, 0.38f), p(0.5f, 0.6f), sw, StrokeCap.Round)
            drawCircle(color, sw * 0.7f, p(0.5f, 0.74f))
        }
        GlyphKind.Stop -> {
            poly(0.32f to 0.08f, 0.68f to 0.08f, 0.92f to 0.32f, 0.92f to 0.68f, 0.68f to 0.92f, 0.32f to 0.92f, 0.08f to 0.68f, 0.08f to 0.32f, close = true)
            drawLine(color, p(0.5f, 0.3f), p(0.5f, 0.56f), sw, StrokeCap.Round)
            drawCircle(color, sw * 0.7f, p(0.5f, 0.72f))
        }
        GlyphKind.Info -> {
            drawCircle(color, w * 0.42f, p(0.5f, 0.5f), style = st)
            drawLine(color, p(0.5f, 0.45f), p(0.5f, 0.7f), sw, StrokeCap.Round)
            drawCircle(color, sw * 0.7f, p(0.5f, 0.3f))
        }
        GlyphKind.Phone -> {
            drawRoundRect(color, Offset(w * 0.26f, h * 0.08f), Size(w * 0.48f, h * 0.84f), androidx.compose.ui.geometry.CornerRadius(w * 0.1f), style = st)
            drawLine(color, p(0.43f, 0.8f), p(0.57f, 0.8f), sw, StrokeCap.Round)
        }
        GlyphKind.Back -> { poly(0.62f to 0.2f, 0.3f to 0.5f, 0.62f to 0.8f); drawLine(color, p(0.3f, 0.5f), p(0.84f, 0.5f), sw, StrokeCap.Round) }
        GlyphKind.Chevron -> poly(0.38f to 0.2f, 0.68f to 0.5f, 0.38f to 0.8f)
        GlyphKind.Ring -> {
            for ((x, y) in listOf(0.3f to 0.3f, 0.7f to 0.3f, 0.5f to 0.72f)) drawRoundRect(color, Offset(w * (x - 0.12f), h * (y - 0.12f)), Size(w * 0.24f, h * 0.24f), androidx.compose.ui.geometry.CornerRadius(w * 0.05f), style = st)
        }
        GlyphKind.Approvals -> { poly(0.18f to 0.5f, 0.4f to 0.72f, 0.82f to 0.26f); drawRoundRect(color, Offset(w * 0.1f, h * 0.1f), Size(w * 0.8f, h * 0.8f), androidx.compose.ui.geometry.CornerRadius(w * 0.12f), style = st) }
        GlyphKind.Enroll -> { drawLine(color, p(0.5f, 0.2f), p(0.5f, 0.8f), sw, StrokeCap.Round); drawLine(color, p(0.2f, 0.5f), p(0.8f, 0.5f), sw, StrokeCap.Round) }
        GlyphKind.Security -> poly(0.5f to 0.08f, 0.86f to 0.22f, 0.86f to 0.52f, 0.5f to 0.92f, 0.14f to 0.52f, 0.14f to 0.22f, close = true)
        GlyphKind.Scan -> {
            poly(0.1f to 0.35f, 0.1f to 0.1f, 0.35f to 0.1f); poly(0.65f to 0.1f, 0.9f to 0.1f, 0.9f to 0.35f)
            poly(0.9f to 0.65f, 0.9f to 0.9f, 0.65f to 0.9f); poly(0.35f to 0.9f, 0.1f to 0.9f, 0.1f to 0.65f)
        }
        GlyphKind.Copy -> {
            drawRoundRect(color, Offset(w * 0.3f, h * 0.3f), Size(w * 0.55f, h * 0.6f), androidx.compose.ui.geometry.CornerRadius(w * 0.08f), style = st)
            poly(0.2f to 0.68f, 0.2f to 0.12f, 0.6f to 0.12f)
        }
        GlyphKind.Lock -> {
            drawRoundRect(color, Offset(w * 0.2f, h * 0.44f), Size(w * 0.6f, h * 0.46f), androidx.compose.ui.geometry.CornerRadius(w * 0.08f), style = st)
            poly(0.32f to 0.44f, 0.32f to 0.3f, 0.4f to 0.14f, 0.6f to 0.14f, 0.68f to 0.3f, 0.68f to 0.44f)
        }
        GlyphKind.Bluetooth -> poly(0.3f to 0.3f, 0.7f to 0.7f, 0.5f to 0.9f, 0.5f to 0.1f, 0.7f to 0.3f, 0.3f to 0.7f)
        GlyphKind.Sound -> { poly(0.12f to 0.4f, 0.3f to 0.4f, 0.5f to 0.2f, 0.5f to 0.8f, 0.3f to 0.6f, 0.12f to 0.6f, close = true); poly(0.68f to 0.35f, 0.76f to 0.5f, 0.68f to 0.65f); poly(0.8f to 0.22f, 0.92f to 0.5f, 0.8f to 0.78f) }
        GlyphKind.Paste -> {
            drawRoundRect(color, Offset(w * 0.2f, h * 0.18f), Size(w * 0.6f, h * 0.72f), androidx.compose.ui.geometry.CornerRadius(w * 0.08f), style = st)
            drawLine(color, p(0.38f, 0.1f), p(0.62f, 0.1f), sw, StrokeCap.Round)
        }
        GlyphKind.Qr -> {
            for ((x, y) in listOf(0.1f to 0.1f, 0.6f to 0.1f, 0.1f to 0.6f)) drawRect(color, Offset(w * x, h * y), Size(w * 0.3f, h * 0.3f), style = st)
            drawCircle(color, sw * 0.8f, p(0.72f, 0.72f)); drawCircle(color, sw * 0.8f, p(0.88f, 0.88f))
        }
    }
}
