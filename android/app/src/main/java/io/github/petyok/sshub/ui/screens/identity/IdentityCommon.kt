package io.github.petyok.sshub.ui.screens.identity

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.dp
import java.time.Instant
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import io.github.petyok.sshub.core.VaultResult
import io.github.petyok.sshub.ui.components.ChainSquare
import io.github.petyok.sshub.ui.components.ChoiceRow
import io.github.petyok.sshub.ui.components.Glyph
import io.github.petyok.sshub.ui.components.GlyphKind
import io.github.petyok.sshub.ui.components.SegmentedHeader
import io.github.petyok.sshub.ui.components.SquareState

/** The font scale the layout decisions follow. `LocalDensity` is what Compose really scales text with (tests set it, the system sets both). */
@Composable
internal fun textScale(): Float = LocalDensity.current.fontScale

/** The app mark: three squares, this phone amber. Decorative. */
@Composable
internal fun AppMark(modifier: Modifier = Modifier) {
    Row(modifier.clearAndSetSemantics { }, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        ChainSquare(SquareState.Default); ChainSquare(SquareState.Default); ChainSquare(SquareState.Me)
    }
}

/** A check or cross, then wrapping prose. The glyph is decorative, the sentence carries the meaning. */
@Composable
internal fun Bullet(glyph: GlyphKind, text: String, modifier: Modifier = Modifier, color: Color = MaterialTheme.colorScheme.onSurface) {
    Row(modifier.fillMaxWidth().heightIn(min = 32.dp).padding(vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.Top) {
        Glyph(glyph, color, Modifier.clearAndSetSemantics { }, size = 20.dp)
        Text(text, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = color)
    }
}

/** Radio group: a segmented switch while the labels fit, stacked rows from font scale 1.3 (spec 4.3). */
@Composable
internal fun <T> ChoiceGroup(options: List<Pair<String, T>>, selected: T, onSelect: (T) -> Unit, modifier: Modifier = Modifier) {
    if (textScale() >= 1.3f) {
        Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
            options.forEach { (label, value) -> ChoiceRow(label, value == selected, { onSelect(value) }) }
        }
    } else {
        SegmentedHeader(
            options.map { Pair<String, Int?>(it.first, null) },
            options.indexOfFirst { it.second == selected }.coerceAtLeast(0),
            { onSelect(options[it].second) },
            modifier,
        )
    }
}

private val utc = DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm'Z'").withZone(ZoneOffset.UTC)

/** `2026-10-23 14:02Z`, always UTC so it reads the same on the terminal. */
internal fun formatUtc(unixSeconds: Long): String = utc.format(Instant.ofEpochSecond(unixSeconds))

/** Literal words for a vault result that is not a success. */
internal fun vaultMessage(result: VaultResult): String = when (result) {
    VaultResult.NoSecureLock -> "This phone has no screen lock."
    VaultResult.Canceled -> "The system prompt was canceled."
    VaultResult.LockedOut -> "Android blocked the unlock for now after too many wrong tries."
    VaultResult.Invalidated -> "Android removed the key because the screen lock was removed or reset."
    is VaultResult.Failed -> result.message
    is VaultResult.Secret -> "The key was released when it was not asked for."
    VaultResult.Stored -> "The key was stored when it was not asked for."
}

internal fun autoLockLabel(seconds: Int): String = if (seconds < 60) "$seconds s" else "${seconds / 60} min"
