package io.github.petyok.sshub.ui.screens.release

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.petyok.sshub.core.OpenedRefused
import io.github.petyok.sshub.ui.components.BarButtons
import io.github.petyok.sshub.ui.components.ButtonKind
import io.github.petyok.sshub.ui.components.DiffBox
import io.github.petyok.sshub.ui.components.DiffEntry
import io.github.petyok.sshub.ui.components.Note
import io.github.petyok.sshub.ui.components.RedBanner
import io.github.petyok.sshub.ui.components.ScreenFrame
import io.github.petyok.sshub.ui.components.SectionHeading
import io.github.petyok.sshub.ui.components.SshubButton
import io.github.petyok.sshub.ui.components.StopBox
import io.github.petyok.sshub.ui.components.WarnBox

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
