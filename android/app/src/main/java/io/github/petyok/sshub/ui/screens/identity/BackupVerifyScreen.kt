package io.github.petyok.sshub.ui.screens.identity

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.petyok.sshub.ui.components.ButtonKind
import io.github.petyok.sshub.ui.components.Gap
import io.github.petyok.sshub.ui.components.MonoField
import io.github.petyok.sshub.ui.components.Note
import io.github.petyok.sshub.ui.components.ScreenFrame
import io.github.petyok.sshub.ui.components.SecureWindow
import io.github.petyok.sshub.ui.components.SshubButton
import io.github.petyok.sshub.ui.components.WaitingDots

/** Number of words on paper; the check asks for two of them. */
const val BACKUP_WORDS = 24

/**
 * Two distinct word numbers (1..24, ascending). `random(n)` returns a number in 0 until n; tests drive it.
 * The second draw is from 23 and skips over the first, so the pair is uniform without a retry loop.
 */
fun pickCheckWords(random: (Int) -> Int): List<Int> {
    val a = random(BACKUP_WORDS) + 1
    var b = random(BACKUP_WORDS - 1) + 1
    if (b >= a) b++
    return listOf(minOf(a, b), maxOf(a, b))
}

/** Case and surrounding spaces do not matter when a typed word is compared with the real one. */
fun sameWord(typed: String, real: String): Boolean = typed.trim().equals(real.trim(), ignoreCase = true)

enum class VerifyPhase { Authenticating, Ready, Canceled, Failed }

data class BackupVerifyState(
    /** The two numbers asked for, ascending. */
    val numbers: List<Int> = listOf(1, 2),
    val typed: List<String> = listOf("", ""),
    /** Numbers that were typed wrong at the last check. Cleared as soon as that box is edited. */
    val wrong: Set<Int> = emptySet(),
    val phase: VerifyPhase = VerifyPhase.Authenticating,
    val message: String? = null,
)

class BackupVerifyActions(
    val onTyped: (index: Int, text: String) -> Unit = { _, _ -> },
    val onCheck: () -> Unit = {},
    val onShowAgain: () -> Unit = {},
    val onAuthenticate: () -> Unit = {},
    /** Null: there is nothing to go back to (first run). */
    val onBack: (() -> Unit)? = null,
)

/** Before the first approval the phone asks for two of the 24 words, so a wrong paper copy shows now and not when the phone is lost. */
@Composable
fun BackupVerifyScreen(state: BackupVerifyState, on: BackupVerifyActions) {
    SecureWindow()
    val ready = state.phase == VerifyPhase.Ready
    ScreenFrame(
        title = "Check your recovery words",
        onBack = on.onBack,
        bar = {
            SshubButton(
                "Check", on.onCheck, Modifier.fillMaxWidth(), kind = ButtonKind.Primary,
                enabled = ready && state.typed.all { it.isNotBlank() },
                disabledReason = if (!ready) "the system prompt is not answered yet" else "type both words",
            )
        },
    ) {
        Text(
            "Before the first approval, type two of the 24 words. If you wrote them down correctly this takes a minute; if not, you find out now, not when the phone is lost.",
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
        )
        Gap(14.dp)
        when (state.phase) {
            VerifyPhase.Authenticating -> WaitingDots(word = "waiting for the system prompt")
            VerifyPhase.Canceled, VerifyPhase.Failed -> {
                Note(state.message ?: "The check starts after you confirm with your fingerprint, face or screen lock.")
                Gap(8.dp)
                SshubButton("Show the system prompt", on.onAuthenticate, Modifier.fillMaxWidth())
            }
            VerifyPhase.Ready -> {
                state.numbers.forEachIndexed { i, n ->
                    Text("Word no. $n", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                    MonoField(
                        state.typed[i], { on.onTyped(i, it) }, label = "Word no. $n",
                        error = if (n in state.wrong) "That is not word no. $n. Look at your paper again." else null,
                    )
                    Gap(12.dp)
                }
                if (state.wrong.isNotEmpty()) SshubButton("Show the words again", on.onShowAgain, Modifier.fillMaxWidth())
            }
        }
    }
}
