package uk.petruha.sshub.phone.ui.screens.release

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.ProofState
import uk.petruha.sshub.phone.core.nameAndId
import uk.petruha.sshub.phone.ui.components.BannerKind
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.DiffBox
import uk.petruha.sshub.phone.ui.components.DiffEntry
import uk.petruha.sshub.phone.ui.components.HexGroups
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.OwnProof
import uk.petruha.sshub.phone.ui.components.ProofKind
import uk.petruha.sshub.phone.ui.components.RedBanner
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.StopBox
import uk.petruha.sshub.phone.ui.components.TickScale
import uk.petruha.sshub.phone.ui.theme.SshubText
import uk.petruha.sshub.phone.ui.theme.sshub

class RedActions(
    val onBack: () -> Unit = {},
    val onApprove: () -> Unit = {},
    val onDecline: () -> Unit = {},
    val onTyped: (String) -> Unit = {},
    val onToggleHash: () -> Unit = {},
    val onCopy: () -> Unit = {},
    val onScanId: () -> Unit = {},
)

/** Red operation, shown to a person as a big change (spec 12 `red`, rules D1 to D12, D18). Stateless: every number comes from `state`. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun RedScreen(state: RedState, on: RedActions) {
    SecureWindow()
    val v = state.view
    val declined = state.phase == ApprovalPhase.Declined
    ScreenFrame(
        title = "Big change",
        onBack = on.onBack,
        banner = {
            if (declined) RedBanner("DECLINED", kind = BannerKind.Neutral, trailing = v.kind)
            else RedBanner("RED OPERATION", trailing = v.kind)
        },
        bar = { RedBar(state, on) },
    ) {
        if (state.closed) {
            WindowClosedBox()
            return@ScreenFrame
        }
        if (state.phase == ApprovalPhase.Review || state.phase == ApprovalPhase.Signing) ExpiryLine(state.timerLeft, if (state.windowIsTimer) spokenWindow(state.timerLeft) else spokenDuration(state.timerLeft) + " left", Modifier.padding(bottom = 8.dp), danger = state.timerLeft <= 0L)
        // One plain sentence first (U13-8); the op's own headline and the raw fields sit under "Technical details". No lamport, no seq.
        Text(v.plain.ifEmpty { v.title }, style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
        Text(
            "proposed by ${nameAndId(v.proposerName, v.proposerId)}", Modifier.padding(top = 4.dp, bottom = 12.dp),
            style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        if (v.mustType != null && state.phase == ApprovalPhase.Review) {
            Note("This proposer never met this phone: type its id in the field below the approvals to enable Approve.", Modifier.padding(bottom = 12.dp))
        }

        // Age on the 3 day window. Recomputed once a minute (spec 7.2): `now` is cut to the minute.
        val total = (v.lapsesAt - v.proposedAt).coerceAtLeast(1L)
        val age = (state.now - v.proposedAt).coerceAtLeast(0L).let { it - it % 60 }
        val left = (total - age).coerceAtLeast(0L)
        val lapseText = if (state.lapsed) "lapsed" else null
        SshubCard {
            FlowRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(durationText(age) + " ago", style = MaterialTheme.typography.headlineSmall, color = MaterialTheme.colorScheme.onSurface)
                lapseText?.let { Text(it, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.sshub.dangerText) }
            }
            TickScale(
                intervals = 72, major = 12, fraction = age.toFloat() / total.toFloat(),
                description = "Age ${spokenDuration(age)} of ${spokenDuration(total)}" + (lapseText?.let { ", $it" } ?: ""),
                modifier = Modifier.padding(top = 8.dp), labels = Triple("proposed", "3 day window", "lapses"),
            )
        }
        if (state.lapsed) {
            StopBox("Lapsed", Modifier.padding(top = 12.dp)) {
                Text(
                    "This notice is older than 3 days. It cannot be approved. Ask the terminal to propose it again.",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
                )
            }
        }

        Note(v.note, Modifier.padding(top = 8.dp))
        TechnicalDetails("${v.fields.size} fields, ${v.fields.count { it.danger }} flagged", startOpen = v.plain.isEmpty()) {
            if (v.plain.isNotEmpty()) Text(v.title, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
            SectionHeading("Changes", trailing = "${v.fields.size} fields")
            DiffBox(v.fields.map { DiffEntry(it.key, it.from, it.to, it.mono, it.danger) })
            SectionHeading("Hash of the operation", trailing = if (state.showHash) "tap to fold" else "tap for all 64")
            HexGroups(
                v.hash, Modifier.heightIn(min = 48.dp), groups = if (state.showHash) null else 3,
                onClick = on.onToggleHash, label = "Hash of the operation",
            )
        }

        // Only this phone's own proof is known here; the page states the rule in a sentence and so does this screen (M2).
        val phone = when {
            v.phoneProof != ProofState.YourTurn -> v.phoneProof
            state.phase == ApprovalPhase.Approved -> ProofState.Signed
            declined -> ProofState.Refused
            else -> v.phoneProof
        }
        SectionHeading("Approvals")
        SshubCard {
            OwnProof(phone.toKind())
            Note("A big change takes effect only with three approvals: this phone, a second device with a person at the keyboard and the paper phrase or the emailed code.", Modifier.padding(top = 8.dp))
        }

        ChecksCard(v.checks)

        if (v.mustType != null && state.phase == ApprovalPhase.Review) RedNeverMet(state, on)

        state.error?.let { ErrorBox(it) }

        if (state.phase == ApprovalPhase.Approved) {
            val out = state.outcome?.outgoing
            SshubCard(Modifier.padding(top = 12.dp)) {
                Text(
                    "Signed with this phone", style = MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.Bold),
                    color = MaterialTheme.sshub.ok,
                )
                Note("Show this to the terminal camera or paste the text there.", Modifier.padding(top = 4.dp))
                if (out != null) {
                    QrLoop(out.qr, QR_DESCRIPTION)
                    CopyAsText(state.copied, state.clearsIn, on.onCopy)
                }
            }
        }
        if (declined) {
            SshubCard(Modifier.padding(top = 12.dp)) {
                Text("Declined.", style = MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.Bold), color = MaterialTheme.colorScheme.onSurface)
                Note("Nothing was signed. " + if (state.lapsed) "The operation has lapsed." else "The operation lapses on its own in ${durationText(left)}.")
            }
        }
    }
}

private fun ProofState.toKind() = when (this) {
    ProofState.YourTurn -> ProofKind.YourTurn
    ProofState.Signed -> ProofKind.Signed
    ProofState.Waiting -> ProofKind.Waiting
    ProofState.Refused -> ProofKind.Refused
}

/** D8, D9: Decline (outline) and Approve with the literal subtitle. Disabled Approve stays visible and names its reason. */
@Composable
private fun RedBar(state: RedState, on: RedActions) {
    when {
        state.closed -> SshubButton("Back to approvals", on.onBack, Modifier.fillMaxWidth())
        state.phase == ApprovalPhase.Approved -> SshubButton("Done", on.onBack, Modifier.fillMaxWidth())
        state.phase == ApprovalPhase.Declined -> SshubButton("Back to approvals", on.onBack, Modifier.fillMaxWidth())
        else -> {
            val gate = state.gate
            BarButtons {
                SshubButton("Decline", on.onDecline, kind = ButtonKind.Outline, enabled = state.phase == ApprovalPhase.Review)
                SshubButton(
                    "Approve", on.onApprove, kind = ButtonKind.Primary,
                    subtitle = if (gate.reason == TYPE_ID_FIRST) TYPE_ID_FIRST else "sign with the phone",
                    enabled = gate.enabled, disabledReason = gate.reason,
                )
            }
        }
    }
}

/** D13 for a Red notice: the proposer was never met, so its id is typed to approve. No amber inside. */
@Composable
private fun RedNeverMet(state: RedState, on: RedActions) {
    val v = state.view
    val required = v.mustType ?: return
    val mismatch = state.typed.isNotBlank() && !typedIdMatches(required, state.typed)
    WarnBox("This proposer has never met this phone.", Modifier.padding(top = 12.dp)) {
        Text(
            "Its signature cannot be checked against a known device. Approve only if you proposed this change yourself and the id printed on the terminal is $required.",
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
        )
        Text(nameAndId(v.proposerName, required), style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
    }
    TypedIdField(
        state.typed, on.onTyped, on.onScanId, Modifier.padding(top = 12.dp), placeholder = "Type $required",
        error = if (mismatch) "does not match" else null, label = "Typed proposer id",
    )
}
