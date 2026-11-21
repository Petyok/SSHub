package io.github.petyok.sshub.ui.screens.identity

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.petyok.sshub.ui.components.ButtonKind
import io.github.petyok.sshub.ui.components.ScreenFrame
import io.github.petyok.sshub.ui.components.SshubButton
import io.github.petyok.sshub.ui.components.StopBox

/**
 * Stop screen for an unreadable saved state (security M2). The core started an empty one; the old bytes are already copied to an
 * app private file (`state.bin.lost`) and NOTHING has been written. Every flow stays closed until the user chooses.
 */
@Composable
fun StateLostScreen(onContinue: () -> Unit, onLock: () -> Unit) {
    ScreenFrame(
        title = "Saved state not readable",
        onBack = null,
        bar = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                SshubButton("Lock and leave it as it is", onLock, Modifier.fillMaxWidth(), kind = ButtonKind.Primary, subtitle = "nothing is replaced")
                SshubButton("Continue as a restored phone", onContinue, Modifier.fillMaxWidth(), subtitle = "no signing until the chain is taken twice")
            }
        },
    ) {
        StopBox("This phone's saved state could not be read") {
            Text(
                "The file may be damaged, or it belongs to another key. This phone has started with an empty state. Nothing has been overwritten yet. " +
                    "A copy of the unreadable file stays on this phone (app private) in case support can use it.",
                style = MaterialTheme.typography.bodyMedium,
            )
            Text(
                "If you continue, this phone starts as a restored phone: it does not sign until it has taken its chain from two terminals, " +
                    "the trust sessions and short codes it knew are not listed, and terminals met before are met again.",
                style = MaterialTheme.typography.bodyMedium,
            )
        }
    }
}
