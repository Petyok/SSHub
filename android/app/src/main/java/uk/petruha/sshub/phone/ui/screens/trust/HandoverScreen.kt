package uk.petruha.sshub.phone.ui.screens.trust

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.PhoneHandoverView
import uk.petruha.sshub.phone.core.TrustHandoff
import uk.petruha.sshub.phone.ui.components.BannerKind
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.CheckboxRow
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.KeyValue
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.RedBanner
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.theme.SshubText

data class HandoverState(
    val view: PhoneHandoverView,
    val phase: TrustPhase = TrustPhase.Review,
    /** Set when `phase == Signed`. */
    val handoff: TrustHandoff? = null,
    val busy: Boolean = false,
    val error: String? = null,
    val clearsIn: Int? = null,
    /** The owner ticked that the 8 words are identical to the terminal's screen (the old phone only). */
    val wordsMatch: Boolean = false,
) {
    /** The old phone signs only after the words were compared; the new phone has no words to compare. */
    val approveEnabled: Boolean get() = phase == TrustPhase.Review && !busy && (view.admission || wordsMatch)
}

class HandoverActions(
    val onWordsMatch: (Boolean) -> Unit,
    val onApprove: () -> Unit,
    val onDecline: () -> Unit,
    val onCopy: () -> Unit,
    val onDone: () -> Unit,
    val onBack: () -> Unit,
)

/**
 * Replacing this phone (design 5, contract 4l b). The OLD phone compares the 8 words with the terminal and signs its part; the NEW phone,
 * shown the admission frame, signs its own admission. Both signatures go through the Keystore prompt of `decideTrust`.
 */
@Composable
fun HandoverScreen(state: HandoverState, on: HandoverActions) {
    SecureWindow()
    val v = state.view
    val signed = state.phase == TrustPhase.Signed && state.handoff != null
    val closed = state.phase == TrustPhase.Closed
    val what = if (v.admission) "admission" else "handover"
    ScreenFrame(
        if (v.admission) "Admit this phone" else "Replace this phone", on.onBack,
        banner = {
            when {
                signed -> RedBanner("SIGNED", kind = BannerKind.Green, trailing = what)
                state.phase == TrustPhase.Declined -> RedBanner("DECLINED", kind = BannerKind.Neutral, trailing = what)
                closed -> RedBanner("WINDOW CLOSED", kind = BannerKind.Neutral, trailing = what)
                else -> RedBanner(if (v.admission) "ADMISSION" else "HANDOVER", kind = BannerKind.Red, trailing = "changes who holds the ring")
            }
        },
        bar = {
            when {
                signed -> HandoffBar(state.clearsIn, on.onDone, on.onCopy)
                state.phase == TrustPhase.Declined || closed -> SshubButton("Back to approvals", on.onBack, Modifier.fillMaxWidth())
                else -> BarButtons {
                    SshubButton("Decline", on.onDecline, enabled = !state.busy)
                    SshubButton(
                        if (v.admission) "Sign my admission" else "Approve", on.onApprove, kind = ButtonKind.Primary, subtitle = "sign with the phone",
                        enabled = state.approveEnabled, disabledReason = if (state.busy) "signing" else "compare the words first",
                    )
                }
            }
        },
    ) {
        when {
            signed -> {
                val h = state.handoff!!
                Text(if (v.admission) "Admission signed" else "Your part is signed", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
                Text(h.summary, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Gap()
                HandoffCard(
                    h.outgoing, what,
                    if (v.admission) "Carry this to the terminal. Then wait: the handover is complete only when the terminal says so."
                    else "Carry this to the terminal that asked. The handover completes after a second terminal and the paper phrase approve it.",
                )
                if (!v.admission) {
                    Gap()
                    WarnBox("Retire this phone yourself") {
                        Text(
                            "Until the handover completes both phones work. After the terminal says it is complete, this phone is no longer a device of your ring: remove the app data or reset it. Nothing retires it for you.",
                            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
                        )
                    }
                } else {
                    Gap()
                    Note("This phone becomes the phone of the ring when the terminal reports the handover complete.")
                }
            }
            state.phase == TrustPhase.Declined -> SshubCard {
                Text("Declined.", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                Note("Nothing was signed.")
            }
            closed -> SshubCard {
                Text("Window closed.", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                Note("The 12 minutes for this screen ended. Nothing was signed. Open the request again.")
            }
            else -> {
                Text(
                    if (v.admission) "This phone is the new phone" else "A new phone asks to replace this one",
                    style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface,
                )
                Gap()
                SshubCard {
                    KeyValue("New phone name", v.name)
                    if (v.admission) KeyValue("Handover operation", v.opHashShort)
                    v.certsCarried.takeIf { it.isNotEmpty() }?.let { KeyValue("Trust sessions that carry over", it.joinToString("\n")) }
                }
                if (!v.admission) {
                    Gap()
                    Text("Compare these 8 words with the terminal", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                    SshubCard {
                        v.words.chunked(4).forEachIndexed { i, row ->
                            val text = row.joinToString("  ")
                            Text(
                                text, Modifier.padding(vertical = 4.dp).semantics { contentDescription = "Words ${i * 4 + 1} to ${i * 4 + row.size}: ${row.joinToString(", ")}" },
                                style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface,
                            )
                        }
                    }
                    CheckboxRow("The 8 words are identical to the terminal's screen", state.wordsMatch, on.onWordsMatch)
                }
                if (v.note.isNotEmpty()) { Gap(); Note(v.note) }
                state.error?.let { Gap(); WarnBox(it, live = true) }
            }
        }
    }
}
