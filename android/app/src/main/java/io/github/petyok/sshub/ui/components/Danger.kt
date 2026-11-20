package io.github.petyok.sshub.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.Box
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.isTraversalGroup
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.petyok.sshub.ui.theme.SshubShape
import io.github.petyok.sshub.ui.theme.SshubText
import io.github.petyok.sshub.ui.theme.sshub

enum class BannerKind { Red, Neutral, Green }

/** Spec 7.10. First element under the header on every Red screen (D1). Heading for TalkBack. */
@Composable
fun RedBanner(text: String, modifier: Modifier = Modifier, kind: BannerKind = BannerKind.Red, trailing: String? = null) {
    val sc = MaterialTheme.sshub
    val cs = MaterialTheme.colorScheme
    val (fill, on) = when (kind) {
        BannerKind.Red -> sc.danger to sc.onDanger
        BannerKind.Green -> sc.ok to sc.onOk
        BannerKind.Neutral -> cs.surfaceContainerHigh to cs.onSurface
    }
    Row(
        modifier.fillMaxWidth().background(fill).heightIn(min = 48.dp).padding(horizontal = 16.dp, vertical = 8.dp)
            .semantics(mergeDescendants = true) { heading() },
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Glyph(if (kind == BannerKind.Green) GlyphKind.Check else GlyphKind.Warning, on, Modifier.clearAndSetSemantics { })
        // From font scale 1.3 the trailing text goes under the title: a word is never broken to fit both on one line (M10).
        if (fontScale() >= 1.3f) {
            Column(Modifier.weight(1f)) {
                Text(text, style = MaterialTheme.typography.titleMedium, color = on)
                if (trailing != null) Text(trailing, style = SshubText.monoBody.copy(fontSize = 13.sp), color = on)
            }
        } else {
            Text(text, Modifier.weight(1f), style = MaterialTheme.typography.titleMedium, color = on)
            if (trailing != null) Text(trailing, style = SshubText.monoBody.copy(fontSize = 13.sp), color = on)
        }
    }
}

/** `.warn` (spec 7.11): 2dp danger outline, errorContainer fill, bold title in dangerText. Not dismissible. */
@Composable
fun WarnBox(title: String, modifier: Modifier = Modifier, live: Boolean = false, body: @Composable ColumnScope.() -> Unit = {}) {
    val sc = MaterialTheme.sshub
    Row(
        modifier.fillMaxWidth().background(MaterialTheme.colorScheme.errorContainer, SshubShape.field).border(2.dp, sc.danger, SshubShape.field).padding(12.dp)
            .semantics(mergeDescendants = true) { if (live) liveRegion = LiveRegionMode.Polite },
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Glyph(GlyphKind.Warning, sc.dangerText, Modifier.clearAndSetSemantics { })
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(title, style = MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.Bold), color = sc.dangerText)
            body()
        }
    }
}

/** `.stop` (spec 7.11): 3dp outline. Mismatch screens use it (D14); the only forward action is Cancel or Back. */
@Composable
fun StopBox(title: String, modifier: Modifier = Modifier, live: Boolean = false, body: @Composable ColumnScope.() -> Unit = {}) {
    val sc = MaterialTheme.sshub
    Column(
        modifier.fillMaxWidth().background(MaterialTheme.colorScheme.errorContainer, MaterialTheme.shapes.medium).border(3.dp, sc.danger, MaterialTheme.shapes.medium).padding(16.dp)
            .semantics(mergeDescendants = true) { if (live) liveRegion = LiveRegionMode.Polite },
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Glyph(GlyphKind.Stop, sc.dangerText, Modifier.clearAndSetSemantics { })
            Text(title, style = MaterialTheme.typography.titleLarge, color = sc.dangerText)
        }
        body()
    }
}

/** One changing field: key, old value (struck), new value (bold). `from == null` is a plain row with a red gutter. `danger = false` draws the row plain, without red. */
data class DiffEntry(val key: String, val from: String?, val to: String, val mono: Boolean = false, val danger: Boolean = true)

/** Spec 7.3. Static, never collapsible, no ellipsis (D4/D5). One merged semantics node per field (D2). */
@Composable
fun DiffBox(fields: List<DiffEntry>, modifier: Modifier = Modifier) {
    val sc = MaterialTheme.sshub
    Column(
        modifier.fillMaxWidth().clip(SshubShape.field).border(2.dp, if (fields.any { it.danger }) sc.danger else MaterialTheme.colorScheme.outlineVariant, SshubShape.field).semantics { isTraversalGroup = true },
    ) {
        fields.forEachIndexed { i, f ->
            Column(
                Modifier.semantics(mergeDescendants = true) {
                    contentDescription = if (f.from != null) "Field ${f.key} changes from ${f.from} to ${f.to}" else "Field ${f.key}, new value ${f.to}"
                },
            ) {
                if (i > 0) androidx.compose.material3.HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                Text(
                    f.key, Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainerLow).padding(start = 46.dp, top = 6.dp, bottom = 3.dp, end = 12.dp),
                    style = SshubText.fieldKey, color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                val cs = MaterialTheme.colorScheme
                val plain = cs.surfaceContainerHigh
                if (f.from != null) DiffLine(f.from, glyph = "-", fill = if (f.danger) sc.diffOld else plain, gutter = if (f.danger) sc.diffOldGutter else cs.outlineVariant, glyphColor = if (f.danger) sc.dangerText else cs.onSurfaceVariant, text = cs.onSurfaceVariant, strike = true, bold = false, mono = f.mono)
                if (f.from != null) DiffLine(f.to, glyph = "+", fill = if (f.danger) sc.diffNew else plain, gutter = if (f.danger) sc.diffNewGutter else cs.outlineVariant, glyphColor = if (f.danger) sc.onDanger else cs.onSurface, text = cs.onSurface, strike = false, bold = true, mono = f.mono)
                else DiffLine(f.to, glyph = "", fill = plain, gutter = if (f.danger) sc.danger else cs.outlineVariant, glyphColor = sc.onDanger, text = cs.onSurface, strike = false, bold = true, mono = f.mono)
            }
        }
    }
}

@Composable
private fun DiffLine(value: String, glyph: String, fill: Color, gutter: Color, glyphColor: Color, text: Color, strike: Boolean, bold: Boolean, mono: Boolean) {
    Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min).background(fill)) {
        Box(Modifier.width(34.dp).fillMaxHeight().background(gutter).clearAndSetSemantics { }, contentAlignment = Alignment.Center) {
            Text(glyph, style = MaterialTheme.typography.titleMedium, color = glyphColor)
        }
        Text(
            value, Modifier.weight(1f).heightIn(min = 36.dp).padding(start = 10.dp, end = 12.dp, top = 6.dp, bottom = 6.dp),
            style = (if (mono) SshubText.monoBody else MaterialTheme.typography.bodyMedium).let { if (bold) it.copy(fontWeight = FontWeight.Bold) else it },
            color = text, textDecoration = if (strike) TextDecoration.LineThrough else null,
        )
    }
}

/** A check line: green check or red cross, and always the words (D3, D8). */
@Composable
fun CheckLine(text: String, ok: Boolean, modifier: Modifier = Modifier) {
    val sc = MaterialTheme.sshub
    Row(
        modifier.fillMaxWidth().heightIn(min = 32.dp).padding(vertical = 2.dp).semantics(mergeDescendants = true) { contentDescription = (if (ok) "Passed: " else "Failed: ") + text },
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Glyph(if (ok) GlyphKind.Check else GlyphKind.Cross, if (ok) sc.ok else sc.dangerText, Modifier.clearAndSetSemantics { }, size = 20.dp)
        Text(text, style = MaterialTheme.typography.bodyMedium, color = if (ok) MaterialTheme.colorScheme.onSurface else sc.dangerText)
    }
}
