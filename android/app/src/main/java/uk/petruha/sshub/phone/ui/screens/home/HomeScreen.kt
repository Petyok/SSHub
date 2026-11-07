package uk.petruha.sshub.phone.ui.screens.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.HomePhoneView
import uk.petruha.sshub.phone.core.PendingItem
import uk.petruha.sshub.phone.core.RingDevice
import uk.petruha.sshub.phone.core.RingView
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.ChainRow
import uk.petruha.sshub.phone.ui.components.CountBadge
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SquareState
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.SshubRow
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.theme.SshubText

const val ADOPTED_NOTE = "Terminals become known again when they contact this phone."
const val NOT_MET = "not met"
const val HEAD_PENDING_TITLE = "This phone does not sign yet."
const val HEAD_PENDING_BODY = "A restored phone has lost its chain. Take it from a terminal first, or the ring will see a fork."
const val TAKE_CHAIN_LABEL = "Take my chain from a terminal"
const val NOTHING_WAITING_TITLE = "Nothing waiting"
const val NOTHING_WAITING_BODY = "Requests arrive when a terminal shows a code and you scan it. Nothing is pushed to this phone, and it keeps no connection open."

/** `home` and `empty-home` of spec 12: the ring when enrolled, the "no ring yet" card otherwise. */
@Composable
fun RingHomeScreen(state: HomeState, on: HomeActions) {
    val ring = state.ring
    ScreenFrame(
        title = "SSHub", onBack = null,
        headerTrailing = { Text(state.phone.name + " " + state.phone.id, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) },
        bar = if (ring == null && !state.restoredAdopted) ({
            BarButtons {
                SshubButton("Restore", on.onRestore, kind = ButtonKind.Outline)
                SshubButton("Enroll with a terminal", on.onEnroll, kind = ButtonKind.Primary)
            }
        }) else null,
    ) {
        // The card follows the status, not the ring: a restored phone has no ring yet and still must take its chain (H5).
        if (state.headPending) {
            WarnBox(HEAD_PENDING_TITLE) {
                Note(HEAD_PENDING_BODY)
                SshubButton(TAKE_CHAIN_LABEL, on.onTakeChain, Modifier.fillMaxWidth(), kind = ButtonKind.Outline)
            }
            Gap()
        }
        when {
            ring != null -> RingBody(state.phone, ring, state.pending, state.headPending, on)
            state.restoredAdopted -> AdoptedBody(state.phone)
            else -> EmptyHomeBody(state.phone, state.headPending)
        }
    }
}

@Composable
private fun TitleLine(title: String, trailing: String) {
    Row(Modifier.fillMaxWidth().padding(bottom = 12.dp), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
        Text(title, Modifier.weight(1f), style = MaterialTheme.typography.headlineSmall, color = MaterialTheme.colorScheme.onSurface)
        Text(trailing, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun RingBody(phone: HomePhoneView, ring: RingView, pending: List<PendingItem>, headPending: Boolean, on: HomeActions) {
    PhoneCard(phone, ring.epoch, headPending)
    SectionHeading("Approvals", trailing = "${pending.size} pending")
    PendingGroups(pending, on)
    SectionHeading("Devices", trailing = "${ring.devices.size} in ring")
    ring.devices.forEachIndexed { i, d -> DeviceRow(d, first = i == 0, last = i == ring.devices.lastIndex) }
}

@Composable
private fun PhoneCard(p: HomePhoneView, epoch: Long, headPending: Boolean) {
    SshubCard {
        Text("this phone", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(p.id, Modifier.padding(top = 2.dp, bottom = 8.dp), style = MaterialTheme.typography.displaySmall, color = MaterialTheme.colorScheme.onSurface)
        KvRow("name", p.name)
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        KvRow("ring epoch", epoch.toString())
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        KvRow("head seq", headSeqText(p, headPending))
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        KvRow("live short codes", p.liveShortCodes.toString())
    }
}

private fun headSeqText(p: HomePhoneView, headPending: Boolean) =
    p.headSeq?.toString() ?: if (headPending) "unknown, not taken yet" else "no ops yet"

private fun groupTitle(tab: ApprovalTab) = when (tab) {
    ApprovalTab.Red -> "Red operations"
    ApprovalTab.Releases -> "Share releases"
    ApprovalTab.Trust -> "Trust requests"
}

@Composable
private fun PendingGroups(pending: List<PendingItem>, on: HomeActions) {
    if (pending.isEmpty()) { Note(NOTHING_WAITING_TITLE + "."); return }
    val groups = pending.groupBy { it.kind.tab() }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        ApprovalTab.entries.forEach { tab ->
            val items = groups[tab] ?: return@forEach
            SshubRow(
                title = groupTitle(tab), secondary = items.joinToString("; ") { it.title },
                trailing = { CountBadge(items.size) },
                onClick = { if (items.size == 1) on.onOpenPending(items[0]) else on.onOpenApprovals() },
            )
        }
    }
}

@Composable
private fun DeviceRow(d: RingDevice, first: Boolean, last: Boolean) {
    val name = d.name ?: NOT_MET
    val description = when {
        d.self -> "$name, this phone"
        d.name == null -> "$name, ${d.id}"
        else -> name
    }
    ChainRow(
        state = if (d.self) SquareState.Me else SquareState.Default,
        title = name, secondary = d.id, first = first, last = last,
        chip = if (d.self) "this phone" else null,
        description = description,
    )
}

/** Restored and chain adopted but no ring known: the terminals are not lost, they reappear when they contact this phone (N9). */
@Composable
private fun AdoptedBody(p: HomePhoneView) {
    SshubCard {
        Text("this phone", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(p.id, Modifier.padding(top = 2.dp, bottom = 8.dp), style = MaterialTheme.typography.displaySmall, color = MaterialTheme.colorScheme.onSurface)
        KvRow("name", p.name)
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        KvRow("head seq", p.headSeq.toString())
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        KvRow("live short codes", p.liveShortCodes.toString())
    }
    Note(ADOPTED_NOTE, Modifier.padding(top = 12.dp))
}

@Composable
private fun EmptyHomeBody(p: HomePhoneView, headPending: Boolean) {
    TitleLine("no ring yet", "not enrolled")
    SshubCard {
        Text("this phone", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(p.id, Modifier.padding(top = 2.dp, bottom = 8.dp), style = MaterialTheme.typography.displaySmall, color = MaterialTheme.colorScheme.onSurface)
        KvRow("name", p.name)
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        KvRow("head seq", headSeqText(p, headPending))
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        KvRow("live short codes", p.liveShortCodes.toString())
    }
    SectionHeading("Devices", trailing = "1 here, 0 terminals")
    ChainRow(
        SquareState.Me, p.name, p.id + " \u00B7 not enrolled", first = true, last = false, chip = "this phone",
        description = "${p.name}, this phone, not enrolled",
    )
    ChainRow(
        SquareState.Stale, "empty", "No terminal yet. A terminal adds this phone to its ring.", first = false, last = true,
        description = "empty slot, no terminal yet",
    )
    SectionHeading("How it starts")
    SshubCard {
        Note("On a terminal start phone enrollment, then tap Enroll with a terminal here. Both screens show four words; if they are identical you tap The words match.")
    }
}

@Composable
private fun KvRow(key: String, value: String) {
    Row(
        Modifier.fillMaxWidth().padding(vertical = 10.dp).semantics(mergeDescendants = true) {},
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(key, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
    }
}
