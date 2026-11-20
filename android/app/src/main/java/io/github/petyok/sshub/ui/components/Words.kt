package io.github.petyok.sshub.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.collectionInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.CollectionInfo
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import io.github.petyok.sshub.ui.theme.SshubShape
import io.github.petyok.sshub.ui.theme.SshubText
import io.github.petyok.sshub.ui.theme.sshub

/** Spec 7.4 keycap: 4dp bottom edge, radius 10. `big` is the 26sp SAS/binding word. */
@Composable
fun WordKeycap(word: String, modifier: Modifier = Modifier, big: Boolean = false, selected: Boolean = false) {
    val sc = MaterialTheme.sshub
    val cs = MaterialTheme.colorScheme
    val fill = if (selected) cs.primaryContainer else cs.surfaceContainerHigh
    val edge = if (selected) sc.focusText else cs.outlineVariant
    Box(
        modifier.heightIn(min = if (big) 68.dp else 56.dp).drawBehind {
            val r = CornerRadius(10.dp.toPx())
            drawRoundRect(sc.keyEdge, cornerRadius = r)
            drawRoundRect(fill, size = Size(size.width, size.height - 4.dp.toPx()), cornerRadius = r)
            drawRoundRect(edge, size = Size(size.width, size.height - 4.dp.toPx()), cornerRadius = r, style = androidx.compose.ui.graphics.drawscope.Stroke(1.dp.toPx()))
        }.padding(start = 4.dp, end = 4.dp, bottom = 4.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(word, style = if (big) SshubText.bigWord else SshubText.keycap, color = if (selected) cs.onPrimaryContainer else cs.onSurface, textAlign = TextAlign.Center)
    }
}

/** The four SAS or binding words as a 2x2 grid, one column from font scale 1.3 (D15, spec 4.3). Group read aloud as one string. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun BigWords(words: List<String>, modifier: Modifier = Modifier) {
    val stack = fontScale() >= 1.3f
    Column(
        modifier.fillMaxWidth().semantics { contentDescription = words.joinToString(", ") },
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        DisableSelection {
            if (stack) words.forEach { WordKeycap(it, Modifier.fillMaxWidth().clearAndSetSemantics { }, big = true) }
            else words.chunked(2).forEach { pair ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    pair.forEach { WordKeycap(it, Modifier.weight(1f).clearAndSetSemantics { }, big = true) }
                }
            }
        }
    }
}

/** The 24 backup words (spec 7.4): 3 columns, 2 from font scale 1.3, 1 from 2.0. Not selectable. */
@Composable
fun WordGrid(words: List<String>, modifier: Modifier = Modifier) {
    val cols = when { fontScale() >= 2.0f -> 1; fontScale() >= 1.3f -> 2; else -> 3 }
    val cs = MaterialTheme.colorScheme
    Column(
        modifier.fillMaxWidth().background(cs.surfaceContainerLow, MaterialTheme.shapes.small).border(1.dp, cs.outlineVariant, MaterialTheme.shapes.small).padding(10.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        DisableSelection {
            words.withIndex().chunked(cols).forEach { rowWords ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    rowWords.forEach { (i, w) ->
                        Row(
                            Modifier.weight(1f).heightIn(min = 44.dp).background(cs.surfaceContainerHigh, MaterialTheme.shapes.small).padding(horizontal = 8.dp, vertical = 6.dp)
                                .semantics(mergeDescendants = true) { contentDescription = "Word ${i + 1}, $w" },
                            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp),
                        ) {
                            Text("${i + 1}", Modifier.widthIn(min = 18.dp), style = MaterialTheme.typography.labelMedium, color = cs.onSurfaceVariant)
                            Text(w, style = SshubText.monoBody.copy(fontWeight = FontWeight.Bold), color = cs.onSurface)
                        }
                    }
                    repeat(cols - rowWords.size) { Box(Modifier.weight(1f)) }
                }
            }
        }
    }
}

/** Hash prefix: first 12 hex digits in 3 groups, or all groups when `full` (D7). Mono, never split inside a group. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun HexGroups(hex: String, modifier: Modifier = Modifier, groups: Int? = 3, onClick: (() -> Unit)? = null, label: String = "Hash") {
    val all = hexGroups(hex)
    val shown = if (groups == null) all else all.take(groups)
    val cs = MaterialTheme.colorScheme
    val m = modifier.fillMaxWidth().semantics(mergeDescendants = true) {
        contentDescription = "$label ${spokenHex(shown)}"
    }
    FlowRow(
        if (onClick != null) m.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onClick) else m,
        horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp),
        maxItemsInEachRow = if (fontScale() >= 1.3f) 2 else 4,
    ) {
        shown.forEach { g ->
            Text(
                g.uppercase(), Modifier.background(cs.surfaceContainerHigh, SshubShape.square).padding(horizontal = 8.dp, vertical = 6.dp).clearAndSetSemantics { },
                style = SshubText.hex, color = cs.onSurfaceVariant,
            )
        }
    }
}

/** Spec 7.5 fingerprint grid: 16 cells, each a Checkbox; the user marks groups while comparing. Never completes anything by itself. */
@Composable
fun FingerprintGrid(hex: String, checked: Set<Int>, onToggle: (Int) -> Unit, modifier: Modifier = Modifier) {
    val sc = MaterialTheme.sshub
    val cs = MaterialTheme.colorScheme
    val all = hexGroups(hex)
    val cols = if (fontScale() >= 2.0f) 1 else if (fontScale() >= 1.3f) 2 else 4
    Column(
        modifier.fillMaxWidth().semantics { collectionInfo = CollectionInfo(rowCount = (all.size + cols - 1) / cols, columnCount = cols) },
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        all.withIndex().chunked(cols).forEach { r ->
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                r.forEach { (i, g) ->
                    val on = i in checked
                    Row(
                        Modifier.weight(1f).heightIn(min = 48.dp)
                            .background(if (on) sc.ok else androidx.compose.ui.graphics.Color.Transparent, SshubShape.square)
                            .border(1.dp, if (on) sc.ok else sc.boundary, SshubShape.square)
                            .toggleable(on, role = Role.Checkbox) { onToggle(i) }
                            .semantics { contentDescription = "Group ${i + 1} of ${all.size}, ${spokenHex(listOf(g))}"; stateDescription = if (on) "checked" else "not checked" }
                            .padding(4.dp),
                        horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically,
                    ) {
                        if (on) Glyph(GlyphKind.Check, sc.onOk, Modifier.clearAndSetSemantics { }, size = 14.dp, stroke = 2.dp)
                        Text(g.uppercase(), style = SshubText.hex, color = if (on) sc.onOk else cs.onSurface)
                    }
                }
                repeat(cols - r.size) { Box(Modifier.weight(1f)) }
            }
        }
    }
}
