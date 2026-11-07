package uk.petruha.sshub.phone.ui.screens.recovery

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.RecoveryNoticeSummary
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.Glyph
import uk.petruha.sshub.phone.ui.components.GlyphKind
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.theme.sshub

/** `now` is unix seconds and only feeds the "left" text of each notice row. */
data class RecoveryState(
    val notices: List<RecoveryNoticeSummary> = emptyList(),
    val now: Long = 0,
)

data class RecoveryActions(
    val onBack: () -> Unit = {},
    val onScan: () -> Unit = {},
    val onOpenNotice: (token: Long) -> Unit = {},
)

/** Spec 12 `recovery`: what recovery is, scan a notice, guidance for a lost phone. */
@Composable
fun RecoveryScreen(state: RecoveryState, on: RecoveryActions) {
    ScreenFrame(
        "Recovery and a new phone", on.onBack,
        bar = { SshubButton("Scan a recovery notice", on.onScan, Modifier.fillMaxWidth(), ButtonKind.Primary) },
    ) {
        Note("If someone starts replacing your phone, it shows on every device for seven days. A terminal can show a recovery notice as a QR code: scan it here to read it, and cancel it if you did not start it.")
        Gap(14.dp)
        if (state.notices.isEmpty()) {
            SshubCard {
                Text("No recovery notice is known to this phone", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                Note("A notice appears here when a terminal shows one and you scan it.")
            }
        } else state.notices.forEach { n ->
            NoticeRow(n, state.now) { on.onOpenNotice(n.token) }
            Gap()
        }
        Gap()
        Guidance(
            "I still have the 24 words",
            "Restore them on a new phone and take the phone's own chain from a terminal before it signs anything. No recovery is needed. Anyone who holds the lost phone and can unlock it can still approve requests: revoke it from a terminal.",
        )
        Gap()
        Guidance(
            "I lost the phone and the words",
            "On a terminal start phone recovery. It needs two other attended devices or the paper phrase, a third factor, and a seven day timelock shown on every device. Afterwards enroll the new phone as usual: the ring reissues shares to it and clears short codes.",
        )
        Gap()
        Guidance(
            "A recovery is running and I did not start it",
            "A phone that can still sign proves the recovery is an attack. Scan the notice and cancel it. No other factor is needed.",
        )
    }
}

@Composable
private fun Guidance(title: String, body: String) {
    SshubCard {
        Text(title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
        Note(body)
    }
}

/** Red outline, a warning glyph and the words: a waiting notice is never mistaken for a plain list row (D3). */
@Composable
private fun NoticeRow(n: RecoveryNoticeSummary, now: Long, onClick: () -> Unit) {
    val sc = MaterialTheme.sshub
    val left = n.endsAt - now
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp)
            .background(MaterialTheme.colorScheme.surfaceContainerLow, MaterialTheme.shapes.medium)
            .border(2.dp, sc.danger, MaterialTheme.shapes.medium)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(14.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Glyph(GlyphKind.Warning, sc.dangerText, Modifier.clearAndSetSemantics { })
        Column(Modifier.weight(1f)) {
            Text("A recovery notice is waiting", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
            Text(
                "${n.startedBy} started a recovery, " + if (left > 0) "${formatRemaining(left)} left" else "timelock ended",
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Glyph(GlyphKind.Chevron, MaterialTheme.colorScheme.onSurfaceVariant, Modifier.clearAndSetSemantics { })
    }
}
