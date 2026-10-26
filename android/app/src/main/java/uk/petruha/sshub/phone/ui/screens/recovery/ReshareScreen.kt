package uk.petruha.sshub.phone.ui.screens.recovery

import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.Outgoing
import uk.petruha.sshub.phone.core.ReshareStage
import uk.petruha.sshub.phone.core.ReshareView
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.KeyValue
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SquareState
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.components.fontScale

/**
 * `reshare` is the reshare that was adopted and published (stage Published); `candidate` is one just opened and not yet adopted.
 * `outgoing` is the signed result of the one adopt-and-publish call, present only right after it.
 */
data class ReshareState(
    val loaded: Boolean = false,
    val stage: ReshareStage = ReshareStage.Empty,
    val generation: Int = 0,
    val epoch: Long = 0,
    val holderNumber: Int = 0,
    val ready: Int = 0,
    val total: Int = 0,
    val reshare: ReshareView? = null,
    val candidate: ReshareView? = null,
    val outgoing: Outgoing? = null,
    val copied: Boolean = false,
    val clearsIn: Int? = null,
    val busy: Boolean = false,
    val error: String? = null,
)

data class ReshareActions(
    val onBack: () -> Unit = {},
    val onRotate: () -> Unit = {},
    /** Opens the scan screen: the reshare ops come from a terminal as QR or pasted text. */
    val onScan: () -> Unit = {},
    /** "Adopt and publish the new shares": adopts and signs in one call. */
    val onAdoptPublish: () -> Unit = {},
    val onCopy: () -> Unit = {},
    val onExport: () -> Unit = {},
    val onDone: () -> Unit = {},
)

private const val ROTATE_FIRST = "rotate wrap keys first"

/** Spec 12 `reshare`: wrap keys, rotate, adopt a reshare and publish the new shares in one signing, and what resharing cannot do. */
@Composable
fun ReshareScreen(state: ReshareState, on: ReshareActions) {
    SecureWindow()
    val published = state.stage == ReshareStage.Published
    val outgoing = state.outgoing
    val bar: (@Composable () -> Unit)? = when {
        published && outgoing != null -> ({ CopyBar(state.copied, on.onCopy, on.onDone) })
        published -> ({
            BarButtons {
                SshubButton("Export my ops", on.onExport, kind = ButtonKind.Outline)
                SshubButton("Done", on.onDone, kind = ButtonKind.Primary)
            }
        })
        else -> null
    }
    ScreenFrame("Reshare and wrap keys", on.onBack, bar = bar) {
        Facts(state)
        if (state.error != null) {
            Gap()
            WarnBox(state.error, live = true)
        }
        if (state.stage == ReshareStage.Empty) {
            Gap()
            SshubCard {
                Text("No reshare is waiting.", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                Note("A reshare starts after a holder is revoked on a terminal. Rotate wrap keys first when it does.")
            }
        } else {
            Steps(state, on)
            val shown = state.reshare
            if (shown != null) {
                SectionHeading("Contributors", trailing = "${shown.contributors.size} counted")
                SshubCard { ContributorLines(shown.contributors, shown.note) }
            }
            if (published && outgoing != null) {
                SectionHeading("Signed ops")
                HandoffCard(
                    outgoing, "QR code with the signed new shares of this phone chain. Use the text below if the terminal has no camera",
                    "Hold the QR code to a terminal, or paste the text into it. It carries the new shares, sealed, and the head of this phone chain.",
                    state.copied, state.clearsIn,
                )
            }
        }
        Note("This phone only receives a new share: it never contributes a sub-share and keeps none of the old ones.", Modifier.padding(vertical = 12.dp))
        WarnBox("A ring of two terminals plus this phone cannot revoke a terminal by resharing.") {
            Text(
                "A revocation needs at least two other terminal holders to contribute. The paper phrase is offline and cannot contribute either. Add a third terminal first.",
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
            )
        }
    }
}

@Composable
private fun Facts(s: ReshareState) {
    SshubCard {
        val cells = listOf("wrap key gen" to s.generation.toString(), "epoch" to s.epoch.toString(), "holder no." to "#${s.holderNumber}")
        if (fontScale() >= 1.3f) cells.forEach { (k, v) -> StatCell(k, v, Modifier.fillMaxWidth()) }
        else Row(Modifier.fillMaxWidth()) { cells.forEach { (k, v) -> StatCell(k, v, Modifier.weight(1f)) } }
        HorizontalDivider(Modifier.padding(vertical = 6.dp), color = MaterialTheme.colorScheme.outlineVariant)
        KeyValue("credentials ready", "${s.ready} of ${s.total}")
    }
}

@Composable
private fun Steps(s: ReshareState, on: ReshareActions) {
    val rotated = s.stage >= ReshareStage.Rotated
    val published = s.stage == ReshareStage.Published
    val done = listOf(rotated, published)
    val current = done.indexOfFirst { !it }
    fun square(i: Int) = if (done[i]) SquareState.Fill else if (i == current) SquareState.Current else SquareState.Default
    fun status(i: Int) = if (done[i]) "done" else if (i == current) "current step" else "waiting"
    SectionHeading("Steps", trailing = "${done.count { it }} of 2 done")
    val fill = Modifier.fillMaxWidth()

    StepRow(
        square(0), "1 Rotate wrap keys", status(0),
        if (rotated) "Done: generation ${s.generation}. Old generations stay readable: they derive from the same 24 words."
        else "Old generations stay readable: they derive from the same 24 words.",
        first = true, last = false,
    ) {
        if (!rotated) SshubButton("Rotate wrap keys", on.onRotate, fill, ButtonKind.Primary, "old generations stay readable", enabled = !s.busy)
    }

    val candidate = s.candidate
    StepRow(
        square(1), "2 Adopt and publish the new shares", status(1),
        if (published) "Done: reshare for epoch ${s.reshare?.epoch ?: s.epoch} adopted, ${s.ready} credentials ready. The new shares, sealed, are in this phone chain for a terminal to take."
        else "Scan or paste the reshare ops a terminal shows for this phone. Adopting signs the new shares into this phone chain in one step.",
        first = false, last = true,
    ) {
        if (!published) {
            if (candidate != null) {
                SshubCard {
                    Text("Reshare for epoch ${candidate.epoch}", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                    KeyValue("holder no.", "#${candidate.holderNumber}")
                    KeyValue("credentials", "${candidate.credentialsReady} of ${candidate.credentialsTotal}")
                    ContributorLines(candidate.contributors, candidate.note)
                }
                SshubButton(
                    "Adopt and publish the new shares", on.onAdoptPublish, fill, if (rotated) ButtonKind.Primary else ButtonKind.Outline,
                    "sign with the phone, sealed into this phone chain", enabled = rotated && candidate.adoptable && !s.busy,
                    disabledReason = if (!rotated) ROTATE_FIRST else candidate.note,
                )
                SshubButton("Scan another reshare", on.onScan, fill, ButtonKind.Outline)
            } else {
                SshubButton(
                    "Adopt a reshare", on.onScan, fill, if (rotated) ButtonKind.Primary else ButtonKind.Outline,
                    "scan or paste the reshare ops", enabled = rotated, disabledReason = ROTATE_FIRST,
                )
            }
        }
    }
}
