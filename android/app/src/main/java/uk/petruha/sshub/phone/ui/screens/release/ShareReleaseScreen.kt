package uk.petruha.sshub.phone.ui.screens.release

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.ChannelKind
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.BigWords
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.CheckboxRow
import uk.petruha.sshub.phone.ui.components.HexGroups
import uk.petruha.sshub.phone.ui.components.HoldCover
import uk.petruha.sshub.phone.ui.components.KeyValue
import uk.petruha.sshub.phone.ui.components.MonoField
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.TickScale
import uk.petruha.sshub.phone.ui.components.ToggleRow
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.components.WordGrid
import uk.petruha.sshub.phone.ui.components.WordKeycap
import uk.petruha.sshub.phone.ui.theme.SshubText
import uk.petruha.sshub.phone.ui.theme.sshub

class ReleaseActions(
    val onBack: () -> Unit = {},
    val onApprove: () -> Unit = {},
    val onDecline: () -> Unit = {},
    val onTyped: (String) -> Unit = {},
    val onChannel: (ChannelKind, Boolean) -> Unit = { _, _ -> },
    val onShortCode: (Boolean) -> Unit = {},
    val onByHand: (Boolean) -> Unit = {},
    val onCopy: () -> Unit = {},
    val onAskAgain: () -> Unit = {},
)

/** Share release (spec 12 `release`, rules D8 to D13, D15, D16, D18, D19). Stateless: every number comes from `state`. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ShareReleaseScreen(state: ReleaseState, on: ReleaseActions) {
    SecureWindow()
    val v = state.view
    val review = state.phase == ApprovalPhase.Review || state.phase == ApprovalPhase.Signing
    val released = state.phase == ApprovalPhase.Approved
    ScreenFrame(title = "Share release", onBack = on.onBack, bar = { ReleaseBar(state, on) }) {
        if (state.closed) {
            WindowClosedBox()
            return@ScreenFrame
        }
        if (review) WindowLine(windowLeft(state.openedAt, state.now))

        SshubCard {
            Text("requester", style = SshubText.fieldKey, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(if (v.requesterKnown) v.requesterName else "not met", style = MaterialTheme.typography.headlineSmall, color = MaterialTheme.colorScheme.onSurface)
            Text(v.requesterId, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurface)
            v.requesterKey?.let { HexGroups(it, Modifier.padding(top = 8.dp), groups = null, label = "Requester key") }
            KeyValue("share", "${v.shareKind}, epoch ${v.shareEpoch}, share ${v.shareIndex}", Modifier.padding(top = 8.dp))
        }

        // Lifetime of the challenge, one needle step per second (spec 8): `now` is whole seconds.
        val total = (v.expiresAt - v.signedAt).coerceAtLeast(1L)
        val left = state.left
        val signedAgo = (state.now - v.signedAt).coerceAtLeast(0L)
        SshubCard(Modifier.padding(top = 12.dp)) {
            FlowRow(
                Modifier.fillMaxWidth().clearAndSetSemantics {
                    contentDescription = announcement(left, released); liveRegion = LiveRegionMode.Polite
                },
                horizontalArrangement = Arrangement.SpaceBetween, verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(
                    if (left > 0) "$left s left" else "expired", style = MaterialTheme.typography.headlineSmall,
                    color = if (left > 0) MaterialTheme.colorScheme.onSurface else MaterialTheme.sshub.dangerText,
                )
                Text("signed ${durationText(signedAgo)} ago", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            TickScale(
                intervals = total.toInt(), major = 10, fraction = (total - left).toFloat() / total.toFloat(),
                description = "Request valid for $total seconds, $left seconds left",
                modifier = Modifier.padding(top = 8.dp), labels = Triple("signed", "", "expires after $total s"),
            )
        }

        SectionHeading("Binding words", trailing = "compare with the terminal")
        BigWords(v.words)
        Note("The terminal shows which host. Compare the binding words.", Modifier.padding(top = 8.dp))

        ChecksCard(v.checks)

        if (v.mustType != null && review) NeverMet(state, on)

        if (review) {
            val on1 = state.channelsOn.size
            SectionHeading("Send the answer by", trailing = "$on1 on")
            SshubCard {
                v.channels.forEach { c -> ToggleRow(c.label, c.kind in state.channelsOn, { on.onChannel(c.kind, it) }) }
            }
            if (v.shortCodeOffer != null) {
                SshubCard(Modifier.padding(top = 12.dp)) {
                    ToggleRow("Also issue a short code", state.shortCode, on.onShortCode)
                    Note(v.shortCodeOffer)
                }
            }
            CheckboxRow("Type it by hand", state.byHand, on.onByHand, Modifier.padding(top = 8.dp))
            if (state.byHand) {
                WarnBox("Look around first.") {
                    Text("Anyone who sees these has this share.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
                    Text(
                        "The share index and the words appear here after you approve. They stay covered until you hold.",
                        style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
                    )
                }
            }
        }

        state.error?.let { ErrorBox(it) }

        if (released) ReleasedCard(state, on)
        if (state.phase == ApprovalPhase.Declined) {
            SshubCard(Modifier.padding(top = 12.dp)) {
                Text("Declined.", style = MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.Bold), color = MaterialTheme.colorScheme.onSurface)
                Note("Nothing was released.")
            }
        }
    }
}

/** D13: warn box with name, id in mono, the typed id gates the primary button. No amber inside. */
@Composable
private fun NeverMet(state: ReleaseState, on: ReleaseActions) {
    val v = state.view
    val required = v.mustType ?: return
    val mismatch = state.typed.isNotBlank() && !typedIdMatches(required, state.typed)
    WarnBox("This requester has never met this phone.", Modifier.padding(top = 12.dp)) {
        Text(
            "Its signature cannot be checked. Approve only if you started this request yourself and the id printed on the terminal is $required.",
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
        )
        Text("${v.requesterName} $required", style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
    }
    MonoField(
        state.typed, on.onTyped, Modifier.padding(top = 12.dp), placeholder = "Type $required",
        error = if (mismatch) "does not match" else null, capitalize = true, label = "Typed requester id",
    )
}

@Composable
private fun ReleasedCard(state: ReleaseState, on: ReleaseActions) {
    val v = state.view
    val out = state.outcome
    val sent = v.channels.filter { it.kind in state.channelsOn }.joinToString(", ") { it.label }.ifEmpty { "no channel" }
    SshubCard(Modifier.padding(top = 12.dp)) {
        Text("Released.", style = MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.Bold), color = MaterialTheme.sshub.ok)
        Note("Sent by $sent. Your share left this phone.")
        val frame = out?.outgoing
        if (frame != null) {
            if (ChannelKind.Qr in state.channelsOn) QrLoop(frame.qr, QR_DESCRIPTION)
            CopyAsText(state.copied, state.clearsIn, on.onCopy)
        }
        val code = out?.shortCode
        if (code != null) {
            Text("short code, shown once", Modifier.padding(top = 12.dp), style = SshubText.fieldKey, color = MaterialTheme.colorScheme.onSurfaceVariant)
            FlowRow(
                Modifier.fillMaxWidth().padding(top = 4.dp).clearAndSetSemantics { contentDescription = "Short code, shown once: " + code.code.split('-').joinToString(", ") },
                horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                code.code.split('-').forEach { WordKeycap(it, Modifier.padding(horizontal = 4.dp)) }
            }
            if (code.weak) Note("This code is weaker than the usual profile on this phone.", Modifier.padding(top = 4.dp))
        }
        val hand = out?.byHand
        if (hand != null) {
            WarnBox("Look around first.", Modifier.padding(top = 12.dp)) {
                Text("Anyone who sees these has this share.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
            }
            Text("share index ${hand.index} of ${hand.of}", Modifier.padding(top = 12.dp, bottom = 8.dp), style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
            HoldCover { WordGrid(hand.words) }
        }
    }
}

@Composable
private fun ReleaseBar(state: ReleaseState, on: ReleaseActions) {
    when {
        state.closed -> SshubButton("Back to approvals", on.onBack, Modifier.fillMaxWidth())
        state.phase == ApprovalPhase.Approved || state.phase == ApprovalPhase.Declined -> SshubButton("Back to approvals", on.onBack, Modifier.fillMaxWidth())
        state.left <= 0L -> SshubButton("Ask the terminal again", on.onAskAgain, Modifier.fillMaxWidth())
        else -> {
            val gate = state.gate
            BarButtons {
                SshubButton("Decline", on.onDecline, kind = ButtonKind.Outline, enabled = state.phase == ApprovalPhase.Review)
                SshubButton(
                    "Approve and release", on.onApprove, kind = ButtonKind.Primary,
                    subtitle = if (gate.reason == TYPE_ID_FIRST) TYPE_ID_FIRST else "sign with the phone",
                    enabled = gate.enabled, disabledReason = gate.reason,
                )
            }
        }
    }
}
