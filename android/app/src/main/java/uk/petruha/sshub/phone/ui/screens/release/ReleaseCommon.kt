package uk.petruha.sshub.phone.ui.screens.release

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import uk.petruha.sshub.phone.core.ReleaseCheck
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.CheckLine
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.QrMatrix
import uk.petruha.sshub.phone.ui.components.QrView
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.StopBox
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.theme.SshubText

/** The 12 minute window (D10): visible mm:ss, spoken in 10 second steps through a polite live region. */
@Composable
internal fun WindowLine(left: Long, modifier: Modifier = Modifier) {
    Row(
        modifier.fillMaxWidth().heightIn(min = 32.dp).padding(bottom = 8.dp)
            .clearAndSetSemantics { contentDescription = spokenWindow(left); liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text("approval window", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(mmss(left), style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
    }
}

/** Shown instead of the whole screen when the 12 minutes ran out. Back is the only action (the bar says so). */
@Composable
internal fun WindowClosedBox() {
    StopBox("Window closed", live = true) {
        Text(
            "The approval window of 12 minutes ran out. Nothing was signed. Go back to the list and open the request again.",
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
        )
    }
}

@Composable
internal fun ErrorBox(message: String) {
    WarnBox("Nothing was signed", live = true) {
        Text(message, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
    }
}

/** A heading with "N of M pass" and one CheckLine per check, in a card (D8: a failing check is a red line with a cross and words). */
@Composable
internal fun ChecksCard(checks: List<ReleaseCheck>) {
    SectionHeading("Checks", trailing = "${checks.count { it.ok }} of ${checks.size} pass")
    SshubCard { checks.forEach { CheckLine(it.text, it.ok) } }
}

/** Looping fountain QR, 4 frames per second. A single frame does not loop. */
@Composable
internal fun QrLoop(frames: List<QrMatrix>, description: String) {
    if (frames.isEmpty()) {
        Note("This reply is too large for a QR code. Use the text.")
        return
    }
    var i by remember(frames) { mutableIntStateOf(0) }
    if (frames.size > 1) LaunchedEffect(frames) { while (true) { delay(250); i = (i + 1) % frames.size } }
    Box(Modifier.fillMaxWidth().padding(vertical = 8.dp), contentAlignment = Alignment.Center) { QrView(frames[i % frames.size], description) }
}

/** The text alternative of a QR (spec 9) and the only copy on these screens: explicit, sensitive, with the visible clear line (D19). */
@Composable
internal fun CopyAsText(copied: Boolean, clearsIn: Int?, onCopy: () -> Unit) {
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SshubButton("Copy as text", onCopy, Modifier.fillMaxWidth(), kind = ButtonKind.Outline)
        if (copied) Text(
            "Copied, clears in ${clearsIn ?: 60} s", Modifier.semantics { liveRegion = LiveRegionMode.Polite },
            style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

internal const val QR_DESCRIPTION = "QR code with the signed reply. Use the text below or ask the other device to read it"
