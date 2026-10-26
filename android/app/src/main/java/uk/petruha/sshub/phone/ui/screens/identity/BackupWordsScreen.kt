package uk.petruha.sshub.phone.ui.screens.identity

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.CheckboxRow
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.Glyph
import uk.petruha.sshub.phone.ui.components.GlyphKind
import uk.petruha.sshub.phone.ui.components.HoldCover
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.WaitingDots
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.components.WordGrid

enum class BackupPhase { Authenticating, Ready, Canceled, Failed }

data class BackupState(
    /** Empty until the system prompt succeeded. Never saved in instance state (spec 11). */
    val words: List<String> = emptyList(),
    val phase: BackupPhase = BackupPhase.Authenticating,
    val paperChecked: Boolean = false,
    /** Shown right after the key was created: there is no way back to the first run screen. */
    val firstRun: Boolean = false,
    val message: String? = null,
)

class BackupActions(
    val onPaperChecked: (Boolean) -> Unit = {},
    val onDone: () -> Unit = {},
    val onAuthenticate: () -> Unit = {},
    val onBack: () -> Unit = {},
)

/**
 * The 24 words (spec 12 `backup`, D16, D18): behind the hold-to-reveal cover, secure window, no copy affordance anywhere.
 * The words reach this screen only after a fresh system prompt.
 */
@Composable
fun BackupWordsScreen(state: BackupState, on: BackupActions) {
    SecureWindow()
    val ready = state.phase == BackupPhase.Ready && state.words.isNotEmpty()
    ScreenFrame(
        title = "Recovery words",
        onBack = if (state.firstRun) null else on.onBack,
        bar = {
            CheckboxRow("I wrote them on paper", state.paperChecked, on.onPaperChecked)
            SshubButton(
                "Done", on.onDone, Modifier.fillMaxWidth(), kind = ButtonKind.Primary,
                enabled = ready && state.paperChecked,
                disabledReason = if (!ready) "the words are not shown yet" else "tick that you wrote the words on paper",
            )
        },
    ) {
        WarnBox("Write these 24 words on paper.") {
            Text(
                "These words are the key. Anyone who has them can act as this phone. Write them on paper, in order. Do not photograph them.",
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
            )
        }
        Gap(12.dp)
        if (ready) {
            HoldCover { WordGrid(state.words) }
        } else when (state.phase) {
            BackupPhase.Authenticating -> WaitingDots(word = "waiting for the system prompt")
            BackupPhase.Canceled, BackupPhase.Failed, BackupPhase.Ready -> {
                Note(state.message ?: "The words stay hidden until you confirm with your fingerprint, face or screen lock.")
                Gap(8.dp)
                SshubButton("Show the system prompt", on.onAuthenticate, Modifier.fillMaxWidth())
            }
        }
        Gap(12.dp)
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Top) {
            Glyph(GlyphKind.Warning, MaterialTheme.colorScheme.onSurfaceVariant, Modifier.clearAndSetSemantics { }, size = 18.dp)
            Text(
                "This screen blocks screenshots and screen recording. There is no copy button: the words never go to the clipboard.",
                Modifier.weight(1f), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
