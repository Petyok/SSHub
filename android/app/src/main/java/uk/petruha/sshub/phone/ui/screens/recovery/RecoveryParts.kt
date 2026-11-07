package uk.petruha.sshub.phone.ui.screens.recovery

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.height
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import uk.petruha.sshub.phone.core.Outgoing
import uk.petruha.sshub.phone.core.ReshareContributor
import uk.petruha.sshub.phone.core.ReshareHolder
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.ChainSquare
import uk.petruha.sshub.phone.ui.components.CheckLine
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.Glyph
import uk.petruha.sshub.phone.ui.components.GlyphKind
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.QrView
import uk.petruha.sshub.phone.ui.components.SquareState
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.TextArea
import uk.petruha.sshub.phone.ui.theme.SshubText
import uk.petruha.sshub.phone.ui.theme.sshub

/** "6 d 3 h", "5 h 12 min", "12 min", "ended". Display only: nothing is decided from it. */
fun formatRemaining(seconds: Long): String {
    if (seconds <= 0) return "ended"
    val d = seconds / 86_400
    val h = seconds % 86_400 / 3_600
    val m = seconds % 3_600 / 60
    return when {
        d > 0 -> "$d d $h h"
        h > 0 -> "$h h $m min"
        else -> "${maxOf(m, 1)} min"
    }
}

fun formatAgo(seconds: Long): String = when {
    seconds < 3_600 -> "${maxOf(seconds / 60, 1)} min ago"
    seconds < 48 * 3_600 -> "${seconds / 3_600} h ago"
    else -> "${seconds / 86_400} d ago"
}

/** One big value over a small key (`.grid3` cell). One merged node: "phone key generation 3". */
@Composable
internal fun StatCell(key: String, value: String, modifier: Modifier = Modifier) {
    Column(modifier.padding(vertical = 4.dp).semantics(mergeDescendants = true) {}) {
        Text(value, style = MaterialTheme.typography.headlineSmall, color = MaterialTheme.colorScheme.onSurface)
        Text(key, style = SshubText.fieldKey, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** A step of the vertical chain: square on a line, mono title, a description, and an optional action below. */
@Composable
internal fun StepRow(
    square: SquareState,
    title: String,
    status: String,
    description: String,
    first: Boolean,
    last: Boolean,
    content: @Composable ColumnScope.() -> Unit = {},
) {
    val line = MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        Modifier.fillMaxWidth().drawBehind {
            val x = 16.dp.toPx()
            drawLine(line, Offset(x, if (first) 22.dp.toPx() else 0f), Offset(x, if (last) 22.dp.toPx() else size.height), 2.dp.toPx())
        },
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        ChainSquare(square, Modifier.clearAndSetSemantics { })
        Column(Modifier.weight(1f).padding(bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(title, Modifier.semantics { stateDescription = status }, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurface)
            Text(description, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            content()
        }
    }
}

/**
 * Contributor check lines: signature, whether this phone has met the contributor, and whether it holds a session key for it.
 * A cross and the words when a check fails; no session key is a failing line (this phone cannot read that contributor's shares).
 */
@Composable
internal fun ContributorLines(contributors: List<ReshareContributor>, note: String) {
    contributors.forEach { c ->
        val who = "${c.name} [${c.id}]"
        CheckLine("$who: " + if (c.verified) "signature verified" else "signature not checked", c.verified)
        CheckLine("$who: " + if (c.known) "known to this phone" else "not known to this phone", c.known)
        CheckLine("$who: " + if (c.hasSession) "session key for this phone" else "no session key for this phone", c.hasSession)
    }
    Note(note, Modifier.padding(top = 6.dp))
}

/** Who holds a share afterwards (the full devices), one merged node each (this phone marked), and whether a storage key is published with the shares. */
@Composable
internal fun HolderLines(holders: List<ReshareHolder>, storageKeyNew: Boolean) {
    holders.forEach { h ->
        val line = "${h.name} [${h.id}]" + if (h.isThisPhone) " (this phone)" else ""
        Text(
            line, Modifier.fillMaxWidth().heightIn(min = 32.dp).padding(vertical = 4.dp).semantics(mergeDescendants = true) { contentDescription = "Full device afterwards: $line" },
            style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface,
        )
    }
    Note(
        if (storageKeyNew) "A storage key of this sharing round is published with the new shares." else "A storage key of this sharing round was already published.",
        Modifier.padding(top = 6.dp),
    )
}

/** Signed bytes handed to a terminal: looping QR, the same data as text, the sensitive-copy notice and the "Copied" line (D19). */
@Composable
internal fun HandoffCard(
    outgoing: Outgoing,
    qrDescription: String,
    note: String,
    copied: Boolean,
    clearsIn: Int?,
    modifier: Modifier = Modifier,
) {
    var frame by remember(outgoing) { mutableIntStateOf(0) }
    if (outgoing.qr.size > 1) LaunchedEffect(outgoing) {
        while (true) { delay(250); frame = (frame + 1) % outgoing.qr.size }
    }
    SshubCard(modifier) {
        if (outgoing.qr.isNotEmpty()) {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                QrView(outgoing.qr[frame % outgoing.qr.size], qrDescription)
            }
            Gap()
        }
        TextArea(outgoing.text, {}, "Signed text", readOnly = true)
        Gap(10.dp)
        Note(note)
        Note("Copying marks the text as sensitive. It leaves the clipboard after 60 s.", Modifier.padding(top = 6.dp))
        if (copied && clearsIn != null) {
            Row(
                Modifier.padding(top = 8.dp).semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite },
                horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Glyph(GlyphKind.Check, MaterialTheme.sshub.ok, Modifier.clearAndSetSemantics { }, size = 18.dp)
                Text("Copied, clears in $clearsIn s", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
            }
        }
    }
}

/** Sticky bar of every hand-off screen: Done and the single amber copy action. */
@Composable
internal fun CopyBar(copied: Boolean, onCopy: () -> Unit, onDone: () -> Unit) {
    BarButtons {
        SshubButton("Done", onDone, kind = ButtonKind.Outline)
        SshubButton(
            if (copied) "Copied" else "Copy the text", onCopy, kind = ButtonKind.Primary,
            subtitle = if (copied) "paste it into the terminal" else "for a terminal without a camera",
        )
    }
}
