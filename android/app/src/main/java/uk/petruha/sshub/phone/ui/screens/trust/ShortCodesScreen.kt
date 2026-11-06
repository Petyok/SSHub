package uk.petruha.sshub.phone.ui.screens.trust

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.ShortCodeRow
import uk.petruha.sshub.phone.core.ShortCodeState
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.EmptyState
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.HexGroups
import uk.petruha.sshub.phone.ui.components.KeyValue
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.TickScale
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.components.groupedUid
import uk.petruha.sshub.phone.ui.theme.SshubText

data class ShortCodesState(
    val now: Long,
    val codes: List<ShortCodeRow>,
    /** `host` whose Declare it dead sheet is open. */
    val confirming: String? = null,
    val busy: Boolean = false,
    val error: String? = null,
)

class ShortCodesActions(
    val onAskDead: (String) -> Unit,
    val onKeep: () -> Unit,
    val onDeclareDead: () -> Unit,
    val onBack: () -> Unit,
)

internal fun days(row: ShortCodeRow): Long = (row.expiresAt - row.issuedAt) / 86_400

/** Fraction of the life of a code that has elapsed, clamped; expired and dead codes are not live so they show the elapsed part as is. */
internal fun elapsed(row: ShortCodeRow, now: Long): Float {
    val total = (row.expiresAt - row.issuedAt).coerceAtLeast(1)
    return ((now - row.issuedAt).toFloat() / total).coerceIn(0f, 1f)
}

/** Spec 12 `codes`: short codes issued, expiry on a tick scale, Declare it dead with a warning sheet, empty state. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ShortCodesScreen(state: ShortCodesState, on: ShortCodesActions) {
    ScreenFrame("Short codes I issued", on.onBack) {
        state.error?.let { WarnBox(it, live = true); Gap() }
        if (state.codes.isEmpty()) {
            EmptyState("None.", "No short codes issued. You can issue one when you approve a release.")
        } else {
            Note("One live code per credential. The terminal shows the same host id. The code itself is never stored here.")
            SectionHeading("Per credential", trailing = if (state.codes.size == 1) "1 code" else "${state.codes.size} codes")
            state.codes.forEach { c ->
                SshubCard {
                    HexGroups(c.host, groups = null, label = "Host id")
                    FlowRow(Modifier.padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp), itemVerticalAlignment = Alignment.CenterVertically) {
                        when (c.state) {
                            ShortCodeState.Live -> TrustStateChip("live", ChipTone.Ok)
                            ShortCodeState.Expired -> TrustStateChip("expired", ChipTone.Neutral)
                            ShortCodeState.DeclaredDead -> TrustStateChip("declared dead", ChipTone.Danger)
                        }
                    }
                    KeyValue("issued", isoDate(c.issuedAt))
                    KeyValue("expires (${days(c)} days)", isoDate(c.expiresAt))
                    TickScale(
                        28, 4, elapsed(c, state.now),
                        if (c.state == ShortCodeState.Live && c.expiresAt > state.now) "Issued ${isoDate(c.issuedAt)}, expires ${isoDate(c.expiresAt)}, ${spanWords(c.expiresAt - state.now)} left"
                        else "Issued ${isoDate(c.issuedAt)}, expired ${isoDate(c.expiresAt)}",
                        Modifier.padding(top = 4.dp), labels = Triple("issued", "${days(c)} days", "expires"),
                    )
                    if (c.state == ShortCodeState.Live) {
                        Gap(10.dp)
                        SshubButton("Declare it dead", { on.onAskDead(c.host) }, Modifier.fillMaxWidth(), enabled = !state.busy)
                    }
                }
                Gap()
            }
        }
    }
    val asked = state.codes.firstOrNull { it.host == state.confirming }
    if (asked != null) TrustConfirmSheet("Declare it dead?", "Declare it dead", "Keep it live", on.onDeclareDead, on.onKeep, enabled = !state.busy) {
        Text("${groupedUid(asked.host)}, issued ${isoDate(asked.issuedAt)}", style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
        WarnBox("Only when you know the sealed blob never reached the terminal, or the credential was reissued.") {
            Text("If it did arrive, the old blob stays valid until it expires.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
        }
    }
}
