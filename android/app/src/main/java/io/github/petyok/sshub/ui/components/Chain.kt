package io.github.petyok.sshub.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.isTraversalGroup
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import io.github.petyok.sshub.ui.theme.SshubText
import io.github.petyok.sshub.ui.theme.sshub

enum class SquareState { Default, Me, Stale, Ok, Current, Fill, No }

/** Spec 7.1. 32dp square, 2dp outline, radius 6, optional glyph. Not a button. */
@Composable
fun ChainSquare(state: SquareState, modifier: Modifier = Modifier, index: String? = null) {
    val c = MaterialTheme.sshub
    val s = MaterialTheme.colorScheme
    val fill: Color; val edge: Color; val glyphColor: Color; val glyph: GlyphKind?; val dashed: Boolean
    when (state) {
        SquareState.Me -> { fill = c.focus; edge = c.focusText; glyphColor = c.onFocus; glyph = GlyphKind.Phone; dashed = false }
        SquareState.Stale -> { fill = Color.Transparent; edge = s.onSurfaceVariant; glyphColor = s.onSurfaceVariant; glyph = null; dashed = true }
        SquareState.Ok -> { fill = c.ok; edge = c.ok; glyphColor = c.onOk; glyph = GlyphKind.Check; dashed = false }
        SquareState.Current -> { fill = c.focus; edge = c.focusText; glyphColor = c.onFocus; glyph = null; dashed = false }
        SquareState.Fill -> { fill = s.onSurface; edge = s.onSurface; glyphColor = s.background; glyph = GlyphKind.Check; dashed = false }
        SquareState.No -> { fill = Color.Transparent; edge = c.danger; glyphColor = c.dangerText; glyph = GlyphKind.Cross; dashed = false }
        SquareState.Default -> { fill = Color.Transparent; edge = s.onSurface; glyphColor = s.onSurface; glyph = null; dashed = false }
    }
    Box(
        modifier.size(32.dp).drawBehind {
            val r = CornerRadius(6.dp.toPx())
            if (fill != Color.Transparent) drawRoundRect(fill, cornerRadius = r)
            val w = 2.dp.toPx()
            drawRoundRect(
                edge, Offset(w / 2, w / 2), Size(size.width - w, size.height - w), r,
                style = Stroke(w, pathEffect = if (dashed) PathEffect.dashPathEffect(floatArrayOf(4.dp.toPx(), 3.dp.toPx())) else null),
            )
        },
        contentAlignment = Alignment.Center,
    ) {
        if (glyph != null) Glyph(glyph, glyphColor, size = 18.dp, stroke = 3.dp)
        else if (index != null) Text(index, style = MaterialTheme.typography.labelMedium, color = glyphColor)
    }
}

/** One row of the vertical chain: square, a 2dp line through the row, title (mono 16 bold), second line, optional chip. */
@Composable
fun ChainRow(
    state: SquareState,
    title: String,
    secondary: String,
    first: Boolean,
    last: Boolean,
    modifier: Modifier = Modifier,
    chip: String? = null,
    description: String,
) {
    val line = MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        modifier.fillMaxWidth().heightIn(min = 76.dp).drawBehind {
            val x = 16.dp.toPx()
            val y0 = if (first) 22.dp.toPx() else 0f
            val y1 = if (last) 22.dp.toPx() else size.height
            drawLine(line, Offset(x, y0), Offset(x, y1), 2.dp.toPx())
        }.semantics(mergeDescendants = true) { contentDescription = description },
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        ChainSquare(state, Modifier.clearAndSetSemantics { })
        Column(Modifier.padding(top = 2.dp, bottom = 12.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(title, style = SshubText.deviceName, color = MaterialTheme.colorScheme.onSurface)
                if (chip != null) Chip(chip)
            }
            Text(secondary, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** Three proofs, always all visible (D6). Spec 7.6. */
enum class ProofKind { YourTurn, Signed, Waiting, Refused }
data class ProofItem(val name: String, val caption: String, val kind: ProofKind)

@Composable
fun ProofChain(proofs: List<ProofItem>, modifier: Modifier = Modifier) {
    val line = MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        modifier.fillMaxWidth().drawBehind {
            val col = size.width / proofs.size
            drawLine(line, Offset(col / 2, 15.dp.toPx()), Offset(size.width - col / 2, 15.dp.toPx()), 2.dp.toPx())
        }.semantics { isTraversalGroup = true },
    ) {
        proofs.forEachIndexed { i, p ->
            val word = when (p.kind) { ProofKind.YourTurn -> "your turn"; ProofKind.Signed -> "signed"; ProofKind.Waiting -> "waiting"; ProofKind.Refused -> "refused" }
            Column(
                Modifier.weight(1f).semantics(mergeDescendants = true) { contentDescription = "Proof ${i + 1} of ${proofs.size}, ${p.name}, $word" },
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                ChainSquare(when (p.kind) { ProofKind.YourTurn -> SquareState.Current; ProofKind.Signed -> SquareState.Ok; ProofKind.Waiting -> SquareState.Default; ProofKind.Refused -> SquareState.No })
                Text(p.name, Modifier.padding(top = 6.dp), style = SshubText.secondaryMono.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = MaterialTheme.colorScheme.onSurface, textAlign = TextAlign.Center)
                Text(p.caption, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
                Text(word, style = MaterialTheme.typography.labelMedium.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = if (p.kind == ProofKind.Refused) MaterialTheme.sshub.dangerText else MaterialTheme.colorScheme.onSurface, textAlign = TextAlign.Center)
            }
        }
    }
}

/** The one proof this phone knows: its own. A square and the words; the other two proofs are not tracked here. */
@Composable
fun OwnProof(kind: ProofKind, modifier: Modifier = Modifier) {
    val word = when (kind) { ProofKind.YourTurn -> "your turn"; ProofKind.Signed -> "signed"; ProofKind.Waiting -> "waiting"; ProofKind.Refused -> "refused" }
    Row(
        modifier.fillMaxWidth().semantics(mergeDescendants = true) { contentDescription = "Proof of this phone, $word" },
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        ChainSquare(when (kind) { ProofKind.YourTurn -> SquareState.Current; ProofKind.Signed -> SquareState.Ok; ProofKind.Waiting -> SquareState.Default; ProofKind.Refused -> SquareState.No })
        Column {
            Text("this phone", style = SshubText.secondaryMono.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = MaterialTheme.colorScheme.onSurface)
            Text(word, style = MaterialTheme.typography.labelMedium.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = if (kind == ProofKind.Refused) MaterialTheme.sshub.dangerText else MaterialTheme.colorScheme.onSurface)
        }
    }
}

/** Spec 7.2. 380x44 drawing scaled to the width; ticks left of the needle are dim (elapsed), the rest lit. */
@Composable
fun TickScale(
    intervals: Int,
    major: Int,
    fraction: Float,
    description: String,
    modifier: Modifier = Modifier,
    labels: Triple<String, String, String>? = null,
) {
    val c = MaterialTheme.sshub
    Column(modifier.fillMaxWidth().semantics(mergeDescendants = true) { contentDescription = description }) {
        Canvas(Modifier.fillMaxWidth().height(44.dp)) {
            val u = size.width / 380f
            val needleX = (fraction.coerceIn(0f, 1f) * 380f).coerceIn(4f, 376f)
            for (i in 0..intervals) {
                val x = 1f + i * (380f - 2f) / intervals
                val tall = i % major == 0
                val h = (if (tall) 22.dp else 12.dp).toPx()
                drawLine(if (x < needleX) c.tickDim else c.tickLit, Offset(x * u, size.height), Offset(x * u, size.height - h), 1.5.dp.toPx())
            }
            val nx = needleX * u
            drawLine(c.focusText, Offset(nx, 6.dp.toPx()), Offset(nx, size.height), 5.dp.toPx())
            drawLine(c.focus, Offset(nx, 6.dp.toPx()), Offset(nx, size.height), 3.dp.toPx())
            val head = Path().apply { moveTo(nx - 6.dp.toPx(), 0f); lineTo(nx + 6.dp.toPx(), 0f); lineTo(nx, 8.dp.toPx()); close() }
            drawPath(head, c.focus)
            drawPath(head, c.focusText, style = Stroke(1.dp.toPx()))
        }
        if (labels != null) {
            val l = listOf(labels.first, labels.second, labels.third)
            val style = MaterialTheme.typography.labelMedium
            val color = MaterialTheme.colorScheme.onSurfaceVariant
            // The three labels share the width; from font scale 1.3 they stack so none runs into another (M10).
            if (fontScale() >= 1.3f) Column(Modifier.fillMaxWidth().padding(top = 4.dp)) { l.filter { it.isNotEmpty() }.forEach { Text(it, style = style, color = color) } }
            else Row(Modifier.fillMaxWidth().padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(l[0], Modifier.weight(1f), style = style, color = color, textAlign = TextAlign.Start)
                Text(l[1], Modifier.weight(1f), style = style, color = color, textAlign = TextAlign.Center)
                Text(l[2], Modifier.weight(1f), style = style, color = color, textAlign = TextAlign.End)
            }
        }
    }
}

/** Wall clock ticking at `everyMs` for countdowns; tests pass `LocalClock`-free fractions instead. */
@Composable
fun rememberNowMillis(everyMs: Long = 1000, now: () -> Long = System::currentTimeMillis): Long {
    var t by remember { mutableLongStateOf(now()) }
    LaunchedEffect(everyMs) { while (true) { delay(everyMs); t = now() } }
    return t
}
