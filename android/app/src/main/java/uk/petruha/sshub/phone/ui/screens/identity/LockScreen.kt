package uk.petruha.sshub.phone.ui.screens.identity

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.DiffBox
import uk.petruha.sshub.phone.ui.components.DiffEntry
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.Glyph
import uk.petruha.sshub.phone.ui.components.GlyphKind
import uk.petruha.sshub.phone.ui.components.MonoField
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.StopBox
import uk.petruha.sshub.phone.ui.components.WaitingDots
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.theme.sshub

enum class LockReason { Started, ScreenOff, Manual }

/** `Waiting` while the system prompt is up; the others are the `VaultResult`s that end it without a secret. */
enum class LockPhase { Waiting, Canceled, LockedOut, Invalidated, Failed }

data class LockState(
    val phase: LockPhase = LockPhase.Waiting,
    val reason: LockReason = LockReason.Started,
    val message: String? = null,
)

class LockActions(
    val onUnlock: () -> Unit = {},
    val onRestore: () -> Unit = {},
)

/** Lock screen (spec 12 `lock`). The app draws no sheet: Android draws the BiometricPrompt, this screen waits for it and explains how it ended. */
@Composable
fun LockScreen(state: LockState, on: LockActions) {
    val invalid = state.phase == LockPhase.Invalidated
    ScreenFrame(
        title = "SSHub locked",
        onBack = null,
        bar = {
            if (invalid) {
                SshubButton("Restore from 24 words", on.onRestore, Modifier.fillMaxWidth(), kind = ButtonKind.Primary, subtitle = "the only way back in")
            } else {
                SshubButton(
                    if (state.phase == LockPhase.LockedOut || state.phase == LockPhase.Failed) "Try again" else "Unlock", on.onUnlock, Modifier.fillMaxWidth(),
                    kind = ButtonKind.Primary, subtitle = "fingerprint, face or screen lock",
                )
                SshubButton("Restore from 24 words instead", on.onRestore, Modifier.fillMaxWidth())
            }
        },
    ) {
        Column(Modifier.fillMaxWidth().padding(top = 24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
            AppMark()
            Gap(4.dp)
            Text("SSHub", style = MaterialTheme.typography.headlineSmall, color = MaterialTheme.colorScheme.onSurface)
            Text("locked", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Gap(6.dp)
            Text(
                when (state.reason) {
                    LockReason.Started -> "Unlock to open the app. Nothing is kept in memory while it is locked."
                    LockReason.ScreenOff -> "Locked because the screen turned off. Every key was wiped from memory."
                    LockReason.Manual -> "Locked on request. Every key was wiped from memory."
                },
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center,
            )
        }
        Gap(24.dp)
        when (state.phase) {
            LockPhase.Waiting -> {
                WaitingDots(word = "waiting for the system prompt")
                Gap(8.dp)
                Note("Android shows the prompt. Confirm with your fingerprint, face or screen lock.")
            }
            LockPhase.Canceled -> Note("Unlock was canceled. The app stays locked.")
            LockPhase.LockedOut -> WarnBox("Android blocked the unlock for now", live = true) {
                Text(
                    "Too many wrong tries. Wait a while and try again; Android counts the tries, the app does not. " +
                        "The 24 words are the other way in: they restore this phone, and the chain is taken again from a terminal.",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
                )
            }
            LockPhase.Invalidated -> StopBox("The key is gone", live = true) {
                Text(
                    "Android removed the key because the fingerprints or the screen lock changed. This protects the key and cannot be undone. " +
                        "The 24 words restore this phone: the same device, the same id. The phone then takes its chain from a terminal before it signs.",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
                )
            }
            LockPhase.Failed -> WarnBox("The key could not be opened", live = true) {
                Text(state.message ?: "Try again.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
            }
        }
    }
}

data class ReplaceKeyState(val typed: String = "") {
    val canErase: Boolean get() = typed.trim() == WORD

    companion object { const val WORD = "ERASE" }
}

class ReplaceKeyActions(
    val onTyped: (String) -> Unit = {},
    val onCancel: () -> Unit = {},
    val onErase: () -> Unit = {},
)

/** Restore over an existing key (spec 12 `lock-restore`, D13 style gate): lists what is erased, ERASE must be typed. */
@Composable
fun ReplaceKeyScreen(state: ReplaceKeyState, on: ReplaceKeyActions) {
    val mismatch = state.typed.length >= ReplaceKeyState.WORD.length && !state.canErase
    ScreenFrame(
        title = "Replace the key",
        onBack = on.onCancel,
        bar = {
            BarButtons {
                SshubButton("Cancel", on.onCancel)
                SshubButton("Erase and continue", on.onErase, kind = ButtonKind.Danger, enabled = state.canErase, disabledReason = "type ERASE first")
            }
        },
    ) {
        WarnBox("Replace the key stored on this phone") {
            Text(
                "Use this only if you cannot unlock. A lost key cannot be reset: the 24 words are the only way back in.",
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
            )
        }
        SectionHeading("Erased from this phone", trailing = "cannot be undone")
        DiffBox(
            listOf(
                DiffEntry("Enrollments", "every terminal this phone met", "erased"),
                DiffEntry("Short-code list", "the list kept for the old key", "erased"),
                DiffEntry("Stored key", "the key in the Android Keystore", "replaced by the key of the 24 words"),
            ),
        )
        SectionHeading("After the restore")
        SshubCard {
            Bullet(GlyphKind.Warning, "The phone signs nothing yet. It has lost the record of its own chain. Home shows a red card until you take the chain from a terminal.", color = MaterialTheme.sshub.dangerText)
            Note("With this phone's own words it is the same device as before: same id, same keys. No recovery and no revocation are needed. Wipe the old phone.")
        }
        SectionHeading("Type ERASE to continue")
        MonoField(
            state.typed, on.onTyped, placeholder = "ERASE", capitalize = true, label = "Type ERASE to confirm",
            error = if (mismatch) "does not match" else null,
        )
    }
}
