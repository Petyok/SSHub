package uk.petruha.sshub.phone.ui.screens.trust

import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.TrustExtendView
import uk.petruha.sshub.phone.core.TrustHandoff
import uk.petruha.sshub.phone.ui.components.BannerKind
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.DiffBox
import uk.petruha.sshub.phone.ui.components.DiffEntry
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.KeyValue
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.RedBanner
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.StopBox
import uk.petruha.sshub.phone.ui.components.TickScale
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.theme.SshubText

data class TrustExtendState(
    val view: TrustExtendView,
    val phase: TrustPhase = TrustPhase.Review,
    /** Set when `phase == Signed`. */
    val handoff: TrustHandoff? = null,
    val busy: Boolean = false,
    val error: String? = null,
    val clearsIn: Int? = null,
) {
    val approveEnabled: Boolean get() = phase == TrustPhase.Review && !busy && view.refusal == null
}

class TrustExtendActions(
    val onApprove: () -> Unit,
    val onDecline: () -> Unit,
    val onCopy: () -> Unit,
    val onDone: () -> Unit,
    val onBack: () -> Unit,
)

/** Spec 12 `trust-extend`: variants ok, notmine, revoked, again (refusals never have an Approve), declined, signed. */
@Composable
fun TrustExtendScreen(state: TrustExtendState, on: TrustExtendActions) {
    SecureWindow()
    val v = state.view
    val signed = state.phase == TrustPhase.Signed && state.handoff != null
    val closed = state.phase == TrustPhase.Closed
    ScreenFrame(
        "Budget extension", on.onBack,
        banner = {
            when {
                signed -> RedBanner("SIGNED", kind = BannerKind.Green, trailing = "budget extension")
                state.phase == TrustPhase.Declined -> RedBanner("DECLINED", kind = BannerKind.Neutral, trailing = "budget extension")
                closed -> RedBanner("WINDOW CLOSED", kind = BannerKind.Neutral, trailing = "budget extension")
                else -> RedBanner("BUDGET EXTENSION", kind = BannerKind.Neutral, trailing = "certificate")
            }
        },
        bar = {
            when {
                signed -> HandoffBar(state.clearsIn, on.onDone, on.onCopy)
                state.phase == TrustPhase.Declined -> SshubButton("Back to approvals", on.onBack, Modifier.fillMaxWidth())
                closed || v.refusal != null -> SshubButton("Back to approvals", on.onBack, Modifier.fillMaxWidth())
                else -> BarButtons {
                    SshubButton("Decline", on.onDecline, enabled = !state.busy)
                    SshubButton(
                        "Approve", on.onApprove, kind = ButtonKind.Primary, subtitle = "sign with the phone", enabled = state.approveEnabled,
                        disabledReason = if (state.busy) "signing" else null,
                    )
                }
            }
        },
    ) {
        when {
            signed -> {
                val h = state.handoff!!
                Text("Budget extension signed", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
                Text(h.summary, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Gap()
                HandoffCard(
                    h.outgoing, "budget extension",
                    "Carry this to the terminal. The extension covers its own 24 hours only and never changes scope or term.",
                )
            }
            state.phase == TrustPhase.Declined -> SshubCard {
                Text("Declined.", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                Note("Nothing was signed.")
            }
            closed -> SshubCard {
                Text("Window closed.", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                Note("The approval window of 12 minutes ended. Nothing was signed. Open the request again.")
            }
            else -> {
                Text("Budget extension requested", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
                Text(v.headline, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Gap()
                v.refusal?.let { r ->
                    StopBox(r.title, live = true) { Text(r.text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface) }
                    Gap()
                }
                v.again?.let { a ->
                    SshubCard {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(spanShort(a.remainingSeconds), style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurface)
                            Text("until the next one", Modifier.padding(start = 8.dp), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        TickScale(
                            24, 6, 1f - a.remainingSeconds.toFloat() / a.windowSeconds.toFloat(),
                            "Possible again in ${spanWords(a.remainingSeconds)}",
                            Modifier.padding(top = 8.dp), labels = Triple("last extension", "24 hours", "possible again"),
                        )
                    }
                    Gap()
                }
                SshubCard { v.fields.forEach { KeyValue(it.key, it.value, danger = it.danger) } }
                if (v.refusal == null) {
                    Gap()
                    DiffBox(listOf(DiffEntry(v.diffKey, v.diffFrom.takeIf { it.isNotEmpty() }, v.diffTo, mono = true, danger = v.diffDanger)))
                    Gap()
                    Note(v.note)
                }
                state.error?.let { Gap(); WarnBox(it, live = true) }
            }
        }
    }
}
