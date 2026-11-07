package uk.petruha.sshub.phone.ui.screens.release

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.OpenedRefused
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.DiffBox
import uk.petruha.sshub.phone.ui.components.DiffEntry
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.RedBanner
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.StopBox
import uk.petruha.sshub.phone.ui.components.WarnBox

class RefusalActions(val onBack: () -> Unit = {}, val onScanAgain: () -> Unit = {})

/**
 * Refusal (spec 12 `refusal`). The text comes from the core value. There is no Approve button, ever (D14, D20).
 * A mismatch (`reason.mismatch`) is a stop box with both compared values in full, and Back is the only action.
 */
@Composable
fun RefusalScreen(refused: OpenedRefused, on: RefusalActions) {
    val r = refused.reason
    ScreenFrame(
        title = "Refused", onBack = on.onBack,
        banner = { RedBanner("REFUSED", trailing = r.label) },
        bar = {
            if (r.mismatch) SshubButton("Back", on.onBack, Modifier.fillMaxWidth())
            else BarButtons {
                SshubButton("Back", on.onBack)
                SshubButton("Scan again", on.onScanAgain, kind = ButtonKind.Primary)
            }
        },
    ) {
        if (r.mismatch) {
            StopBox(r.title) {
                Text(r.text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
            }
        } else {
            Text(r.title, style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
            Note(r.text, Modifier.padding(top = 6.dp))
        }
        if (r.compared.isNotEmpty()) {
            if (r.mismatch) SectionHeading("Compared values", trailing = "${r.compared.size} values")
            DiffBox(r.compared.map { DiffEntry(it.key, null, it.value, mono = true) }, Modifier.padding(top = if (r.mismatch) 0.dp else 12.dp))
        }
        if (refused.checks.isNotEmpty()) ChecksCard(refused.checks)
        WarnBox("No Approve button is offered.", Modifier.padding(top = 12.dp)) {
            Text("Nothing was signed.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
        }
    }
}
