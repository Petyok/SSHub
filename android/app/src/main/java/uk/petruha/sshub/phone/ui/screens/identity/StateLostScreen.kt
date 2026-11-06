package uk.petruha.sshub.phone.ui.screens.identity

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.StopBox

/**
 * Stop screen for an unreadable saved state (security M2). The core started an empty one; the old bytes are already copied to
 * `state.bin.lost` and NOTHING has been written. Every flow stays closed until the user chooses.
 */
@Composable
fun StateLostScreen(onContinue: () -> Unit, onLock: () -> Unit) {
    ScreenFrame(
        title = "Saved state not readable",
        onBack = null,
        bar = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                SshubButton("Lock and keep the old file", onLock, Modifier.fillMaxWidth(), kind = ButtonKind.Primary, subtitle = "nothing is replaced")
                SshubButton("Continue with an empty state", onContinue, Modifier.fillMaxWidth(), subtitle = "the old file stays as state.bin.lost")
            }
        },
    ) {
        StopBox("This phone's saved state could not be read") {
            Text(
                "The file may be damaged, or it belongs to another key. This phone has started with an empty state: the chain head, the issued " +
                    "certificates and the short codes it knew are not loaded. Nothing has been overwritten yet. A copy of the unreadable file is kept as state.bin.lost.",
                style = MaterialTheme.typography.bodyMedium,
            )
        }
    }
}
