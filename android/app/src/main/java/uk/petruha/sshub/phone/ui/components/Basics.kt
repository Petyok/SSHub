package uk.petruha.sshub.phone.ui.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.ui.theme.SshubShape
import uk.petruha.sshub.phone.ui.theme.SshubText
import uk.petruha.sshub.phone.ui.theme.sshub

enum class ButtonKind { Primary, Outline, Danger }

/** Spec 7.7. Min 56dp high, radius 12, 1.5dp border. A disabled button is still readable and carries stateDescription. */
@Composable
fun SshubButton(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    kind: ButtonKind = ButtonKind.Outline,
    subtitle: String? = null,
    enabled: Boolean = true,
    disabledReason: String? = null,
) {
    val cs = MaterialTheme.colorScheme
    val sc = MaterialTheme.sshub
    val (fill, edge, text) = when {
        !enabled -> Triple(cs.surfaceContainerHigh, cs.outlineVariant, cs.onSurfaceVariant)
        kind == ButtonKind.Primary -> Triple(cs.primary, sc.focusText, cs.onPrimary)
        kind == ButtonKind.Danger -> Triple(Color.Transparent, sc.danger, sc.dangerText)
        else -> Triple(Color.Transparent, cs.outline, cs.onSurface)
    }
    val src = remember { MutableInteractionSource() }
    Column(
        modifier
            .heightIn(min = 56.dp)
            .background(fill, MaterialTheme.shapes.medium)
            .border(1.5.dp, edge, MaterialTheme.shapes.medium)
            .clickable(enabled = enabled, interactionSource = src, indication = androidx.compose.material3.ripple(), role = Role.Button, onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 8.dp)
            .semantics(mergeDescendants = true) {
                if (!enabled) { disabled(); stateDescription = "Disabled" + (disabledReason?.let { ", $it" } ?: "") }
            },
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(label, style = SshubText.button, color = text, textAlign = TextAlign.Center)
        if (subtitle != null) Text(subtitle, style = SshubText.buttonSub, color = text.copy(alpha = 0.85f), textAlign = TextAlign.Center)
    }
}

/** Section heading `.h`: 20 above, 8 below. The trailing text wraps under the heading when both do not fit (M10). */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun SectionHeading(text: String, modifier: Modifier = Modifier, first: Boolean = false, trailing: String? = null) {
    FlowRow(
        modifier.fillMaxWidth().padding(top = if (first) 0.dp else 20.dp, bottom = 8.dp),
        horizontalArrangement = Arrangement.SpaceBetween, verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text(text, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface, modifier = Modifier.semantics { heading() })
        if (trailing != null) Text(trailing, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** Header bar (spec 7.9): 56dp min, back arrow 48x48, Inter SemiBold 18. */
@Composable
fun SshubHeader(title: String, onBack: (() -> Unit)?, modifier: Modifier = Modifier, trailing: @Composable (() -> Unit)? = null) {
    Column(modifier.fillMaxWidth().windowInsetsPadding(WindowInsets.statusBars)) {
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            if (onBack != null) {
                Box(
                    Modifier.size(48.dp).padding(end = 0.dp).clickable(role = Role.Button, onClick = onBack).semantics { contentDescription = "Back" },
                    contentAlignment = Alignment.CenterStart,
                ) { Glyph(GlyphKind.Back, MaterialTheme.colorScheme.onSurface) }
            }
            // From font scale 1.3 the trailing text goes under the title so a name like "laptop-main" is never broken to make room (M10).
            if (fontScale() >= 1.3f) {
                Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
                    Text(title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurface, modifier = Modifier.semantics { heading() })
                    trailing?.invoke()
                }
            } else {
                Text(title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurface, modifier = Modifier.weight(1f).semantics { heading() })
                trailing?.invoke()
            }
        }
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
    }
}

/**
 * Standard screen frame: header, optional banner slot (D1: directly under the header), scrolling content, sticky action bar.
 * The content gets bottom padding equal to the MEASURED bar height so the last field is never hidden (spec 4.3).
 * Width is capped at 600dp and centred (spec 5).
 */
@Composable
fun ScreenFrame(
    title: String,
    onBack: (() -> Unit)?,
    modifier: Modifier = Modifier,
    banner: (@Composable () -> Unit)? = null,
    bar: (@Composable () -> Unit)? = null,
    headerTrailing: @Composable (() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    Box(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background), contentAlignment = Alignment.TopCenter) {
        var barPx by remember { mutableIntStateOf(0) }
        Column(Modifier.fillMaxSize().widthIn(max = 600.dp)) {
            SshubHeader(title, onBack, trailing = headerTrailing)
            banner?.invoke()
            Column(
                Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp)
                    .padding(bottom = with(LocalDensity.current) { barPx.toDp() }),
                content = content,
            )
        }
        if (bar != null) {
            Column(
                Modifier.align(Alignment.BottomCenter).widthIn(max = 600.dp).fillMaxWidth()
                    .background(MaterialTheme.colorScheme.background)
                    .onSizeChanged { barPx = it.height }
                    .windowInsetsPadding(WindowInsets.navigationBars).windowInsetsPadding(WindowInsets.ime),
            ) {
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) { bar() }
            }
        }
    }
}

/** Sticky bar buttons side by side while they fit, stacked once the font scale makes labels long (spec 4.3). */
@Composable
fun BarButtons(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    androidx.compose.ui.layout.Layout(content, modifier.fillMaxWidth()) { measurables, c ->
        val gap = 12.dp.roundToPx()
        val n = measurables.size.coerceAtLeast(1)
        val each = (c.maxWidth - gap * (n - 1)) / n
        // Intrinsic heights decide the mode; every child is then measured exactly once.
        val side = fontScaleFor(this) < 1.3f && measurables.all { it.maxIntrinsicHeight(each) <= 72.dp.roundToPx() }
        if (side) {
            val placeables = measurables.map { it.measure(androidx.compose.ui.unit.Constraints(minWidth = each, maxWidth = each)) }
            val h = placeables.maxOfOrNull { it.height } ?: 0
            layout(c.maxWidth, h) { var x = 0; placeables.forEach { it.place(x, 0); x += each + gap } }
        } else {
            val placeables = measurables.map { it.measure(androidx.compose.ui.unit.Constraints(minWidth = c.maxWidth, maxWidth = c.maxWidth)) }
            val h = placeables.sumOf { it.height } + gap * (placeables.size - 1).coerceAtLeast(0)
            layout(c.maxWidth, h) { var y = 0; placeables.forEach { it.place(0, y); y += it.height + gap } }
        }
    }
}
private fun fontScaleFor(d: androidx.compose.ui.unit.Density) = d.fontScale

/** Bordered card, radius 12, 14dp padding (spec 5). */
@Composable
fun SshubCard(modifier: Modifier = Modifier, onClick: (() -> Unit)? = null, content: @Composable ColumnScope.() -> Unit) {
    val m = modifier.fillMaxWidth()
        .background(MaterialTheme.colorScheme.surfaceContainerLow, MaterialTheme.shapes.medium)
        .border(1.dp, MaterialTheme.colorScheme.outlineVariant, MaterialTheme.shapes.medium)
    Column(
        (if (onClick != null) m.clickable(role = Role.Button, onClick = onClick) else m).heightIn(min = 56.dp).padding(14.dp),
        verticalArrangement = Arrangement.Center, content = content,
    )
}

/** List row (spec 5): title, one secondary line, one trailing item; the whole row is the target. */
@Composable
fun SshubRow(
    title: String,
    modifier: Modifier = Modifier,
    secondary: String? = null,
    trailing: @Composable (() -> Unit)? = null,
    mono: Boolean = false,
    onClick: (() -> Unit)? = null,
) {
    SshubCard(modifier, onClick) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(title, style = if (mono) MaterialTheme.typography.bodyLarge else MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                if (secondary != null) Text(secondary, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            trailing?.invoke()
            if (onClick != null && trailing == null) Glyph(GlyphKind.Chevron, MaterialTheme.colorScheme.onSurfaceVariant, Modifier.clearA11y())
        }
    }
}

fun Modifier.clearA11y(): Modifier = this.semantics(mergeDescendants = false) { }

@Composable
fun Note(text: String, modifier: Modifier = Modifier) =
    Text(text, modifier, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)

@Composable
fun Gap(h: Dp = 12.dp) = Spacer(Modifier.size(h))

/** A key and its value. `danger` (the core's flag) draws the row with a warning glyph and the danger colour, never colour alone (D3). */
@Composable
fun KeyValue(key: String, value: String, modifier: Modifier = Modifier, mono: Boolean = true, valueColor: Color = MaterialTheme.colorScheme.onSurface, danger: Boolean = false) {
    Column(
        modifier.fillMaxWidth().padding(vertical = 6.dp)
            .semantics(mergeDescendants = true) { if (danger) contentDescription = "Danger: $key: $value" },
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            if (danger) Glyph(GlyphKind.Warning, MaterialTheme.sshub.dangerText, Modifier.size(16.dp), size = 16.dp)
            Text(key, style = SshubText.fieldKey, color = if (danger) MaterialTheme.sshub.dangerText else MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Text(value, style = if (mono) SshubText.monoBody else MaterialTheme.typography.bodyMedium, color = if (danger) MaterialTheme.sshub.dangerText else valueColor)
    }
}

/** Small dashed-outline label (`.chip`). */
@Composable
fun Chip(text: String, modifier: Modifier = Modifier) {
    Text(
        text, modifier.border(BorderStroke(1.dp, MaterialTheme.colorScheme.onSurfaceVariant), SshubShape.square).padding(horizontal = 8.dp, vertical = 2.dp),
        style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/** Counter pill (`.cnt`, nav badge): 20dp high, secondary fill. Zero is not shown by callers. */
@Composable
fun CountBadge(n: Int, modifier: Modifier = Modifier, muted: Boolean = false) {
    Box(
        modifier.heightIn(min = 20.dp).widthIn(min = 20.dp)
            .background(if (muted) MaterialTheme.colorScheme.outlineVariant else MaterialTheme.colorScheme.secondary, androidx.compose.foundation.shape.RoundedCornerShape(10.dp))
            .padding(horizontal = 6.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(n.toString(), style = MaterialTheme.typography.labelMedium.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = if (muted) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSecondary)
    }
}
