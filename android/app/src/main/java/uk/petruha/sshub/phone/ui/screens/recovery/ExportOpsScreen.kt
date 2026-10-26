package uk.petruha.sshub.phone.ui.screens.recovery

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.ExportOpRow
import uk.petruha.sshub.phone.core.Outgoing
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.EmptyState
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.spokenHex
import uk.petruha.sshub.phone.ui.theme.SshubText

/** `outgoing` is null when there are no rows. `clearsIn` is the clipboard port's countdown, null when nothing of ours is on it. */
data class ExportState(
    val loaded: Boolean = false,
    val rows: List<ExportOpRow> = emptyList(),
    val total: Int = 0,
    val outgoing: Outgoing? = null,
    val copied: Boolean = false,
    val clearsIn: Int? = null,
)

data class ExportActions(
    val onBack: () -> Unit = {},
    val onCopy: () -> Unit = {},
    val onDone: () -> Unit = {},
)

/** Spec 12 `export`: the newest 8 signed ops as a list, as QR and as text. Words and shares are not on this screen, so no secure window. */
@Composable
fun ExportOpsScreen(state: ExportState, on: ExportActions) {
    val outgoing = state.outgoing
    ScreenFrame(
        "Export my ops as text", on.onBack,
        bar = {
            if (outgoing != null) CopyBar(state.copied, on.onCopy, on.onDone)
            else SshubButton("Done", on.onDone, Modifier.fillMaxWidth(), ButtonKind.Outline)
        },
    ) {
        if (outgoing == null) {
            if (state.loaded) EmptyState("No signed entries", "This phone has not signed anything yet. There is nothing to export.")
            return@ScreenFrame
        }
        Note("The newest 8 signed entries of this phone's chain, as text for a terminal without a camera (paste it into the terminal) or as a QR code.")
        Gap(16.dp)
        SectionHeading("Newest entries", first = true, trailing = "${state.rows.size} of ${state.total} ops")
        SshubCard {
            state.rows.forEachIndexed { i, r ->
                if (i > 0) HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                OpRow(r)
            }
        }
        Gap(16.dp)
        HandoffCard(
            outgoing, "QR code with the newest signed ops of this phone chain. Use the text below if the terminal has no camera",
            "A terminal without a camera: copy the text and paste it into the terminal. A terminal with a camera can scan the QR code.",
            state.copied, state.clearsIn,
        )
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun OpRow(r: ExportOpRow) {
    Row(
        Modifier.fillMaxWidth().padding(vertical = 8.dp).semantics(mergeDescendants = true) {
            contentDescription = "Operation ${r.seq}, ${r.kind}, ${r.time}" + (if (r.hash.isNotEmpty()) ", hash ${spokenHex(listOf(r.hash))}" else "")
        },
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(r.seq.toString(), Modifier.widthIn(min = 32.dp), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurface)
        Column(Modifier.weight(1f)) {
            Text(r.kind, style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(r.time, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (r.hash.isNotEmpty()) Text(r.hash, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}
