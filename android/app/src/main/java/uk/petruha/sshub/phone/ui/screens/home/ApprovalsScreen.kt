package uk.petruha.sshub.phone.ui.screens.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.Chip
import uk.petruha.sshub.phone.ui.components.EmptyState
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SegmentedHeader
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubRow

const val SCAN_REQUEST_LABEL = "Scan a request"

/** `approvals` of spec 12. The segment counts always show the number of items, so an empty segment is visible before it is opened. */
@Composable
fun ApprovalsScreen(state: ApprovalsState, on: ApprovalsActions) {
    val tabs = ApprovalTab.entries
    val sel = state.selected.coerceIn(0, tabs.lastIndex)
    val shown = state.pending.filter { it.kind.tab() == tabs[sel] }
    ScreenFrame(
        title = "Approvals", onBack = null,
        headerTrailing = {
            Text(
                if (state.pending.isEmpty()) "none" else "${state.pending.size} waiting",
                style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        bar = { SshubButton(SCAN_REQUEST_LABEL, on.onScan, Modifier.fillMaxWidth(), kind = ButtonKind.Primary) },
    ) {
        SegmentedHeader(tabs.map { t -> t.label to state.pending.count { it.kind.tab() == t } }, sel, on.onSelect)
        Gap()
        if (shown.isEmpty()) {
            EmptyState(NOTHING_WAITING_TITLE, NOTHING_WAITING_BODY)
        } else {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                shown.forEach { item ->
                    SshubRow(
                        title = item.title, secondary = item.age,
                        trailing = { Chip(item.kind.word()) },
                        onClick = { on.onOpen(item) },
                    )
                }
            }
            Note("A request lives about two minutes. A Red notice lives three days.", Modifier.padding(top = 12.dp))
        }
    }
}
