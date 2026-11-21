package io.github.petyok.sshub.ui.screens.trust

import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.petyok.sshub.core.TrustExtendView
import io.github.petyok.sshub.core.TrustHandoff
import io.github.petyok.sshub.ui.components.BannerKind
import io.github.petyok.sshub.ui.components.BarButtons
import io.github.petyok.sshub.ui.components.ButtonKind
import io.github.petyok.sshub.ui.components.DiffBox
import io.github.petyok.sshub.ui.components.DiffEntry
import io.github.petyok.sshub.ui.components.Gap
import io.github.petyok.sshub.ui.components.KeyValue
import io.github.petyok.sshub.ui.components.Note
import io.github.petyok.sshub.ui.components.RedBanner
import io.github.petyok.sshub.ui.components.ScreenFrame
import io.github.petyok.sshub.ui.components.SecureWindow
import io.github.petyok.sshub.ui.components.SshubButton
import io.github.petyok.sshub.ui.components.SshubCard
import io.github.petyok.sshub.ui.components.StopBox
import io.github.petyok.sshub.ui.components.TickScale
import io.github.petyok.sshub.ui.components.WarnBox
import io.github.petyok.sshub.ui.theme.SshubText

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
                else -> RedBanner("BUDGET EXTENSION", kind = BannerKind.Neutral, trailing = "trust session")
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
                Note("The 12 minutes for this screen ended. Nothing was signed. Open the request again.")
            }
            else -> {
                Text("Budget extension requested", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
                if (v.plain.isNotEmpty()) Text(v.plain, Modifier.padding(vertical = 4.dp), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurface)
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
