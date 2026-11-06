package uk.petruha.sshub.phone.ui.screens.trust

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
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
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import kotlinx.coroutines.delay
import uk.petruha.sshub.phone.core.Outgoing
import uk.petruha.sshub.phone.core.TrustEpochView
import uk.petruha.sshub.phone.core.TrustFieldView
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.CheckLine
import uk.petruha.sshub.phone.ui.components.DiffBox
import uk.petruha.sshub.phone.ui.components.DiffEntry
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.Glyph
import uk.petruha.sshub.phone.ui.components.GlyphKind
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.QrView
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.TextArea
import uk.petruha.sshub.phone.ui.theme.SshubShape
import uk.petruha.sshub.phone.ui.theme.sshub

// ---- date and span text (UTC, the terminal prints UTC too) ----

internal fun isoDate(sec: Long): String = LocalDate.ofEpochDay(Math.floorDiv(sec, 86_400L)).toString()

private val MINUTE_FORMAT: DateTimeFormatter = DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm").withZone(ZoneOffset.UTC)
internal fun isoMinute(sec: Long): String = MINUTE_FORMAT.format(Instant.ofEpochSecond(sec))

private fun unit(n: Long, one: String) = if (n == 1L) "$n $one" else "$n ${one}s"

/** "2 days 3 hours", "17 hours 22 minutes", "5 minutes": read aloud as is. */
internal fun spanWords(sec: Long): String {
    val s = sec.coerceAtLeast(0)
    val d = s / 86_400; val h = s % 86_400 / 3_600; val m = s % 3_600 / 60
    return when {
        d > 0 -> unit(d, "day") + " " + unit(h, "hour")
        h > 0 -> unit(h, "hour") + " " + unit(m, "minute")
        else -> unit(m, "minute")
    }
}

/** "17 h 22 min". */
internal fun spanShort(sec: Long): String {
    val s = sec.coerceAtLeast(0)
    val d = s / 86_400; val h = s % 86_400 / 3_600; val m = s % 3_600 / 60
    return when { d > 0 -> "$d d $h h"; h > 0 -> "$h h $m min"; else -> "$m min" }
}

// ---- pieces shared by the screens of the group ----

internal enum class ChipTone { Ok, Neutral, Danger }

/** State chip: glyph and word, never colour alone (D3). */
@Composable
internal fun TrustStateChip(text: String, tone: ChipTone, modifier: Modifier = Modifier) {
    val sc = MaterialTheme.sshub
    val cs = MaterialTheme.colorScheme
    val color = when (tone) { ChipTone.Ok -> sc.ok; ChipTone.Neutral -> cs.onSurfaceVariant; ChipTone.Danger -> sc.dangerText }
    val edge = if (tone == ChipTone.Danger) sc.danger else color
    Row(
        modifier.border(1.dp, edge, SshubShape.square).padding(horizontal = 8.dp, vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        when (tone) {
            ChipTone.Ok -> Glyph(GlyphKind.Check, color, size = 14.dp)
            ChipTone.Danger -> Glyph(GlyphKind.Cross, color, size = 14.dp)
            ChipTone.Neutral -> Unit
        }
        Text(text, style = MaterialTheme.typography.labelMedium, color = color)
    }
}

/** The core's field list as one DiffBox: a field with no old value is a plain row with a red gutter (D2). */
@Composable
internal fun TrustFieldsBox(fields: List<TrustFieldView>, modifier: Modifier = Modifier) =
    DiffBox(fields.map { DiffEntry(it.key, null, it.value, mono = true) }, modifier)

/** Epoch range: its own box with a 3dp onSurface outline (the highlight of the prototype) and the two checks, always shown. */
@Composable
internal fun EpochBox(epoch: TrustEpochView, modifier: Modifier = Modifier) {
    Column(
        modifier.fillMaxWidth().border(3.dp, MaterialTheme.colorScheme.onSurface, SshubShape.field).padding(3.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        DiffBox(listOf(DiffEntry(epoch.label, null, epoch.value, mono = true)))
        Column(Modifier.padding(horizontal = 8.dp)) { epoch.checks.forEach { CheckLine(it.text, it.ok) } }
    }
}

/** Signed reply as QR (looping fountain frames at 4 per second) and as text; the text is the alternative for every QR (spec 9). */
@Composable
internal fun HandoffCard(outgoing: Outgoing, what: String, note: String, modifier: Modifier = Modifier) {
    var frame by remember(outgoing) { mutableIntStateOf(0) }
    if (outgoing.qr.size > 1) LaunchedEffect(outgoing) {
        while (true) { delay(250); frame = (frame + 1) % outgoing.qr.size }
    }
    SshubCard(modifier) {
        if (outgoing.qr.isNotEmpty()) {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                QrView(outgoing.qr[frame.coerceIn(0, outgoing.qr.size - 1)], "QR code with the signed $what. Use the text below or hold it to the terminal camera")
            }
            Gap(10.dp)
        } else Note("This answer is too large for a QR code. Use the text below.")
        TextArea(outgoing.text, {}, label = "Signed text", readOnly = true)
        Gap(10.dp)
        Note(note)
    }
}

/** Sticky bar of every hand-off: Done and Copy the text. Nothing is copied by default (D19). */
@Composable
internal fun HandoffBar(clearsIn: Int?, onDone: () -> Unit, onCopy: () -> Unit) {
    if (clearsIn != null) Text(
        "Copied, clears in $clearsIn s", Modifier.semantics { liveRegion = LiveRegionMode.Polite },
        style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    BarButtons {
        SshubButton("Done", onDone)
        SshubButton(
            "Copy the text", onCopy, kind = ButtonKind.Primary,
            subtitle = if (clearsIn != null) "paste it into the terminal" else "for a terminal without a camera",
        )
    }
}
