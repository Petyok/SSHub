package uk.petruha.sshub.phone.ui.components

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.invisibleToUser
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import kotlinx.coroutines.delay
import uk.petruha.sshub.phone.ui.theme.LocalReducedMotion
import uk.petruha.sshub.phone.ui.theme.SshubText
import uk.petruha.sshub.phone.ui.theme.sshub

/** `QR` modules, true = dark. Rendered black on white with a 4 module quiet zone in both themes (spec 7.13). */
class QrMatrix(val size: Int, private val bits: BooleanArray) {
    operator fun get(x: Int, y: Int) = bits[y * size + x]
}

@Composable
fun QrView(matrix: QrMatrix, description: String, modifier: Modifier = Modifier, side: androidx.compose.ui.unit.Dp = 176.dp) {
    Canvas(
        modifier.size(side).semantics { contentDescription = description },
    ) {
        val quiet = 4
        val total = matrix.size + 2 * quiet
        // whole device pixels per module (the terminal camera needs crisp edges)
        val px = kotlin.math.floor(size.width / total).coerceAtLeast(1f)
        val origin = (size.width - px * total) / 2f
        drawRect(Color.White)
        for (y in 0 until matrix.size) for (x in 0 until matrix.size) if (matrix[x, y]) {
            drawRect(Color.Black, Offset(origin + (x + quiet) * px, origin + (y + quiet) * px), androidx.compose.ui.geometry.Size(px, px))
        }
    }
}

/** Hold-to-reveal cover (spec 7.13, D16). Reveals only while pressed; re-covers on release and ON_PAUSE; TalkBack gets "Reveal for 10 seconds". */
@Composable
fun HoldCover(modifier: Modifier = Modifier, onRevealChanged: (Boolean) -> Unit = {}, label: String = "Hold to reveal", content: @Composable () -> Unit) {
    var revealed by remember { mutableStateOf(false) }
    var timed by remember { mutableStateOf(false) }
    fun set(v: Boolean) { revealed = v; onRevealChanged(v) }
    val owner = LocalLifecycleOwner.current
    DisposableEffect(owner) {
        val o = LifecycleEventObserver { _, e -> if (e == Lifecycle.Event.ON_PAUSE) { timed = false; set(false) } }
        owner.lifecycle.addObserver(o); onDispose { owner.lifecycle.removeObserver(o) }
    }
    LaunchedEffect(timed) { if (timed) { set(true); delay(10_000); set(false); timed = false } }
    val cs = MaterialTheme.colorScheme
    Box(
        modifier.fillMaxWidth().pointerInput(Unit) {
            awaitEachGesture { awaitFirstDown(); set(true); waitForUpOrCancellation(); if (!timed) set(false) }
        }.semantics {
            customActions = listOf(CustomAccessibilityAction("Reveal for 10 seconds") { timed = true; true })
        },
    ) {
        Box(Modifier.semantics { if (!revealed) invisibleToUser() }) { content() }
        if (!revealed) Box(
            Modifier.matchParentSize().background(cs.surfaceContainerHigh, MaterialTheme.shapes.medium)
                .border(1.dp, cs.onSurfaceVariant, MaterialTheme.shapes.medium)
                .drawBehind {
                    var x = -size.height
                    while (x < size.width) { drawLine(cs.outlineVariant, Offset(x, size.height), Offset(x + size.height, 0f), 2.dp.toPx()); x += 12.dp.toPx() }
                }
                .semantics { contentDescription = "Words hidden. $label." },
            contentAlignment = Alignment.Center,
        ) {
            Text(label, Modifier.background(cs.surfaceContainerHigh, MaterialTheme.shapes.small).padding(horizontal = 12.dp, vertical = 8.dp), style = MaterialTheme.typography.bodyLarge, color = cs.onSurface)
        }
    }
}

/** Pulsing three dots + the word "waiting". Static when reduced motion (spec 8). */
@Composable
fun WaitingDots(modifier: Modifier = Modifier, word: String = "waiting") {
    val reduced = LocalReducedMotion.current
    val t = rememberInfiniteTransition(label = "dots")
    Row(modifier.semantics(mergeDescendants = true) { contentDescription = word }, horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
        for (i in 0 until 3) {
            val a = if (reduced) 1f else t.animateFloat(0.25f, 1f, infiniteRepeatable(tween(1200, delayMillis = i * 200, easing = LinearEasing), RepeatMode.Reverse), label = "d$i").value
            Box(Modifier.size(8.dp).background(MaterialTheme.colorScheme.onSurface.copy(alpha = a), androidx.compose.foundation.shape.CircleShape).clearAndSetSemantics { })
        }
        Text(word, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** Camera viewfinder frame: corner brackets and a scan line (static in the middle when reduced motion). */
@Composable
fun ViewfinderFrame(modifier: Modifier = Modifier, content: @Composable () -> Unit = {}) {
    val reduced = LocalReducedMotion.current
    val sc = MaterialTheme.sshub
    val t = rememberInfiniteTransition(label = "scan")
    val y by t.animateFloat(0.1f, 0.9f, infiniteRepeatable(tween(2000, easing = FastOutSlowInEasing), RepeatMode.Reverse), label = "line")
    val frac = if (reduced) 0.5f else y
    Box(
        modifier.fillMaxWidth().aspectRatio(1f).background(Color.Black, MaterialTheme.shapes.small).drawBehind {
            val a = 28.dp.toPx(); val w = 3.dp.toPx(); val c = Color.White
            fun corner(x: Float, y: Float, dx: Float, dy: Float) { drawLine(c, Offset(x, y), Offset(x + dx * a, y), w); drawLine(c, Offset(x, y), Offset(x, y + dy * a), w) }
            val m = 12.dp.toPx()
            corner(m, m, 1f, 1f); corner(size.width - m, m, -1f, 1f); corner(m, size.height - m, 1f, -1f); corner(size.width - m, size.height - m, -1f, -1f)
            drawLine(sc.focus, Offset(m * 2, size.height * frac), Offset(size.width - m * 2, size.height * frac), 2.dp.toPx())
        },
        contentAlignment = Alignment.Center,
    ) { content() }
}

/** Empty state (spec 7.13): three squares (third dashed), a title, a line, an optional action. */
@Composable
fun EmptyState(title: String, body: String, modifier: Modifier = Modifier, action: (@Composable () -> Unit)? = null) {
    Column(modifier.fillMaxWidth().padding(vertical = 48.dp, horizontal = 16.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), modifier = Modifier.clearAndSetSemantics { }) {
            ChainSquare(SquareState.Default); ChainSquare(SquareState.Default); ChainSquare(SquareState.Stale)
        }
        Text(title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurface)
        Text(body, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
        action?.invoke()
    }
}
