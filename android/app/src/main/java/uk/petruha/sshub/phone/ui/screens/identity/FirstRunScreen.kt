package uk.petruha.sshub.phone.ui.screens.identity

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.GlyphKind
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.WaitingDots
import uk.petruha.sshub.phone.ui.components.WarnBox

enum class FirstRunPhase { Ready, Creating, NoSecureLock, Failed }

data class FirstRunState(val phase: FirstRunPhase = FirstRunPhase.Ready, val error: String? = null)

class FirstRunActions(
    val onCreate: () -> Unit = {},
    val onRestore: () -> Unit = {},
    val onOpenSecuritySettings: () -> Unit = {},
)

/** First run (spec 12 `firstrun`): what the app is, how the key is kept, create or restore. No PIN fields: the system lock is the gate. */
@Composable
fun FirstRunScreen(state: FirstRunState, on: FirstRunActions) {
    val creating = state.phase == FirstRunPhase.Creating
    ScreenFrame(
        title = "SSHub",
        onBack = null,
        bar = {
            SshubButton(
                if (creating) "Creating the key..." else "Create a new key", on.onCreate, Modifier.fillMaxWidth(),
                kind = ButtonKind.Primary, subtitle = "wrapped by the Android Keystore",
                enabled = !creating, disabledReason = "the key is being created",
            )
            SshubButton("Restore from 24 words", on.onRestore, Modifier.fillMaxWidth(), enabled = !creating, disabledReason = "the key is being created")
        },
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(14.dp), verticalAlignment = Alignment.CenterVertically) {
            AppMark()
            Column {
                Text("SSHub", style = MaterialTheme.typography.headlineSmall, color = MaterialTheme.colorScheme.onSurface)
                Text("phone companion, first run", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        Gap(14.dp)
        SshubCard {
            Text("what this app is", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Bullet(GlyphKind.Check, "A share holder of your SSHub ring: one share of every Quorum credential. It approves dangerous changes and releases shares.")
            Bullet(GlyphKind.Check, "No account and no network. It works in airplane mode.")
            Gap(6.dp)
            Text("what it is not", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Bullet(GlyphKind.Cross, "Not a sync peer and not a password manager: no copy of your hosts, no password to read.")
        }
        SectionHeading("Screen lock of this phone", trailing = "no PIN in the app")
        Note("The secret of this phone is wrapped by a key inside the Android Keystore. Every unlock asks for your fingerprint, face or screen lock. The app has no PIN of its own.")
        Gap(8.dp)
        Note("A phone without a screen lock cannot hold the key. Set one first; the app never stores the key unprotected.")
        when (state.phase) {
            FirstRunPhase.NoSecureLock -> {
                Gap(16.dp)
                WarnBox("No screen lock on this phone", live = true) {
                    Text("Set a PIN, pattern, password or biometric in Android settings, come back and create the key.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
                    SshubButton("Open security settings", on.onOpenSecuritySettings, Modifier.fillMaxWidth())
                }
            }
            FirstRunPhase.Failed -> {
                Gap(16.dp)
                WarnBox("The key was not created", live = true) {
                    Text(state.error ?: "Try again.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
                }
            }
            FirstRunPhase.Creating -> { Gap(16.dp); WaitingDots(word = "creating the key") }
            FirstRunPhase.Ready -> Unit
        }
    }
}
