package uk.petruha.sshub.phone.ui.screens.trust

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.TrustHandoff
import uk.petruha.sshub.phone.core.TrustRenewView
import uk.petruha.sshub.phone.ui.components.BannerKind
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.KeyValue
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.RedBanner
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.StopBox
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.theme.SshubText

data class TrustRenewState(
    val view: TrustRenewView,
    val phase: TrustPhase = TrustPhase.Review,
    /** Set when `phase == Signed`. */
    val handoff: TrustHandoff? = null,
    val busy: Boolean = false,
    val error: String? = null,
    val clearsIn: Int? = null,
) {
    /** Never enabled for a refusal or for evidence the phone would not sign on (the core re-checks both at signing). */
    val approveEnabled: Boolean get() = phase == TrustPhase.Review && !busy && !view.red
}

typealias TrustRenewActions = TrustExtendActions

/** The renewal of a trust session (design 6.2): what is renewed, until when, and how fresh the news from the other terminals is. */
@Composable
fun TrustRenewScreen(state: TrustRenewState, on: TrustRenewActions) {
    SecureWindow()
    val v = state.view
    val signed = state.phase == TrustPhase.Signed && state.handoff != null
    val closed = state.phase == TrustPhase.Closed
    ScreenFrame(
        "Renew a trust session", on.onBack,
        banner = {
            when {
                signed -> RedBanner("SIGNED", kind = BannerKind.Green, trailing = "renewal")
                state.phase == TrustPhase.Declined -> RedBanner("DECLINED", kind = BannerKind.Neutral, trailing = "renewal")
                closed -> RedBanner("WINDOW CLOSED", kind = BannerKind.Neutral, trailing = "renewal")
                v.red -> RedBanner("RENEWAL", kind = BannerKind.Red, trailing = "check the news below")
                else -> RedBanner("RENEWAL", kind = BannerKind.Neutral, trailing = "trust session")
            }
        },
        bar = {
            when {
                signed -> HandoffBar(state.clearsIn, on.onDone, on.onCopy)
                state.phase == TrustPhase.Declined || closed || v.refusal != null -> SshubButton("Back to approvals", on.onBack, Modifier.fillMaxWidth())
                else -> BarButtons {
                    SshubButton("Decline", on.onDecline, enabled = !state.busy)
                    SshubButton(
                        "Approve", on.onApprove, kind = ButtonKind.Primary, subtitle = "sign with the phone", enabled = state.approveEnabled,
                        disabledReason = if (state.busy) "signing" else "the news from the other terminals is too old",
                    )
                }
            }
        },
    ) {
        when {
            signed -> {
                val h = state.handoff!!
                Text("Renewal signed", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
                Text(h.summary, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Gap()
                HandoffCard(h.outgoing, "renewal", "Carry this to the terminal. The renewal ends on the date shown and never changes scope or budget.")
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
                Text("Renewal requested", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
                if (v.plain.isNotEmpty()) Text(v.plain, Modifier.padding(vertical = 4.dp), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurface)
                Text(v.headline, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Gap()
                v.refusal?.let { r ->
                    StopBox(r.title, live = true) { Text(r.text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface) }
                    Gap()
                }
                SshubCard {
                    KeyValue("Trusted device", v.delegate)
                    KeyValue("Scope", v.scope)
                    KeyValue("New end", "${isoDate(v.newEnd)} (adds ${spanWords(v.termSecs)})", danger = true)
                    KeyValue("New hosts per day", v.budget.toString())
                    KeyValue("Renewals left", v.renewalsLeft.toString())
                    KeyValue("Hosts it opened", v.openedText)
                    KeyValue(
                        "Freshest news from another terminal",
                        v.evidenceAgeSecs?.let { "${spanWords(it)} old" } ?: "none: only the trusted device itself vouches for the list",
                        danger = v.evidenceStale,
                    )
                }
                if (v.note.isNotEmpty()) { Gap(); Note(v.note) }
                state.error?.let { Gap(); WarnBox(it, live = true) }
            }
        }
    }
}
