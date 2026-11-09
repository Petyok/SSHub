package uk.petruha.sshub.phone.ui.screens.trust

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.CertState
import uk.petruha.sshub.phone.core.IssuedCert
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.EmptyState
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.Glyph
import uk.petruha.sshub.phone.ui.components.GlyphKind
import uk.petruha.sshub.phone.ui.components.HexGroups
import uk.petruha.sshub.phone.ui.components.KeyValue
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.theme.SshubText

data class TrustListState(
    val certs: List<IssuedCert>,
    val expanded: Set<String> = emptySet(),
    /** `certId` whose revoke sheet is open. */
    val confirming: String? = null,
    val busy: Boolean = false,
    val error: String? = null,
)

class TrustListActions(
    val onToggle: (String) -> Unit,
    val onAskRevoke: (String) -> Unit,
    val onKeep: () -> Unit,
    val onRevoke: () -> Unit,
    val onScan: () -> Unit,
    val onBack: () -> Unit,
)

/** Spec 12 `trust-list`: active, expired and revoked certificates with their extensions; revoke through a small confirmation sheet; empty state. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun TrustListScreen(state: TrustListState, on: TrustListActions) {
    SecureWindow()
    ScreenFrame(
        "Trust sessions", on.onBack,
        bar = { SshubButton("Scan a request", on.onScan, Modifier.fillMaxWidth(), kind = ButtonKind.Primary) },
    ) {
        state.error?.let { WarnBox(it, live = true); Gap() }
        if (state.certs.isEmpty()) {
            EmptyState("No trust sessions issued", "A terminal asks for one by showing a request. Scan it: this phone signs the trust session.")
        } else {
            SectionHeading("Issued by this phone", first = true, trailing = if (state.certs.size == 1) "1 trust session" else "${state.certs.size} trust sessions")
            state.certs.forEach { c ->
                val open = c.certId in state.expanded
                SshubCard {
                    Row(
                        Modifier.fillMaxWidth().heightIn(min = 56.dp)
                            .clickable(role = Role.Button) { on.onToggle(c.certId) }
                            .semantics { stateDescription = if (open) "Expanded" else "Collapsed" },
                        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        androidx.compose.foundation.layout.Column(Modifier.weight(1f)) {
                            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp), itemVerticalAlignment = Alignment.CenterVertically) {
                                Text(c.deviceName ?: c.deviceLabel, style = SshubText.deviceName, color = MaterialTheme.colorScheme.onSurface)
                                when (c.state) {
                                    CertState.Active -> TrustStateChip("active", ChipTone.Ok)
                                    CertState.Expired -> TrustStateChip("expired", ChipTone.Neutral)
                                    CertState.Revoked -> TrustStateChip("revoked", ChipTone.Danger)
                                }
                            }
                            Text(
                                "${c.scope}, ${c.termText}, ends ${isoDate(c.endsAt)}",
                                style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Text("epoch ${c.epochLo} to ${c.epochHi}", style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Text("budget ${c.budgetPerDay} per day", style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        Glyph(GlyphKind.Chevron, MaterialTheme.colorScheme.onSurfaceVariant, Modifier.rotate(if (open) 90f else 0f))
                    }
                    if (open) {
                        KeyValue("device id", c.deviceId)
                        if (c.group != null) KeyValue("group", c.group)
                        else {
                            Text("hosts", Modifier.padding(top = 6.dp), style = SshubText.fieldKey, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            c.hosts.forEach { HexGroups(it, Modifier.padding(top = 4.dp), groups = null, label = "Host id") }
                        }
                        KeyValue("issued", isoDate(c.issuedAt))
                        KeyValue("term", "${c.termText}, ends ${isoDate(c.endsAt)}")
                        if (c.renewals > 0) KeyValue("renewals", "${c.renewals} of 2")
                        KeyValue("epoch range", "epoch ${c.epochLo} to ${c.epochHi}")
                        KeyValue("trust session", c.certId)
                        c.lamport?.let { KeyValue("chain position", it.toString()) }
                        KeyValue(
                            "extensions",
                            if (c.extensions.isEmpty()) "none" else c.extensions.joinToString("\n") { "+${it.extraHosts} new hosts, ${isoMinute(it.at)}" },
                        )
                        if (c.state == CertState.Active) {
                            Gap(8.dp)
                            SshubButton("Revoke this trust session", { on.onAskRevoke(c.certId) }, Modifier.fillMaxWidth(), kind = ButtonKind.Danger, enabled = !state.busy)
                        }
                    }
                }
                Gap(12.dp)
            }
        }
    }
    val asked = state.certs.firstOrNull { it.certId == state.confirming }
    if (asked != null) TrustConfirmSheet(
        "Revoke this trust session?", "Revoke and sign", "Keep it", on.onRevoke, on.onKeep, enabled = !state.busy,
    ) {
        Text(asked.deviceName ?: asked.deviceLabel, style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
        Text("${asked.scope}, ${asked.termText}, epoch ${asked.epochLo} to ${asked.epochHi}", style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Note("Peers stop honouring it on their next sync. This phone signs the revocation; you then carry it to any terminal.")
    }
}
