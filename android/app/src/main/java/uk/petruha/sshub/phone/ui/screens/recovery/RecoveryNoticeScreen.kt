package uk.petruha.sshub.phone.ui.screens.recovery

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.Outgoing
import uk.petruha.sshub.phone.core.RecoveryNoticeView
import uk.petruha.sshub.phone.ui.components.BannerKind
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.DiffBox
import uk.petruha.sshub.phone.ui.components.DiffEntry
import uk.petruha.sshub.phone.ui.components.EmptyState
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.HexGroups
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.RedBanner
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.TickScale
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.theme.SshubText

enum class NoticePhase { Review, Confirm, Signed }

/** `view` null with `loaded` true means the core knows no notice under this token. `now` is unix seconds, for the timelock text. */
data class NoticeState(
    val loaded: Boolean = false,
    val view: RecoveryNoticeView? = null,
    val phase: NoticePhase = NoticePhase.Review,
    val signed: Outgoing? = null,
    val copied: Boolean = false,
    val clearsIn: Int? = null,
    val now: Long = 0,
    val busy: Boolean = false,
    val error: String? = null,
)

data class NoticeActions(
    val onBack: () -> Unit = {},
    /** "Cancel this recovery: I still have this phone": opens the confirmation, signs nothing. */
    val onAskCancel: () -> Unit = {},
    /** "Sign the cancellation": the only call that signs. */
    val onSign: () -> Unit = {},
    val onNotYet: () -> Unit = {},
    /** "Leave it (I started this)" and "Back to recovery". */
    val onLeave: () -> Unit = {},
    val onCopy: () -> Unit = {},
    val onDone: () -> Unit = {},
)

private const val TITLE = "Phone recovery in progress"

/** Spec 12 `recovery-notice`: a Red screen (D1, D7, D12, D18). Nothing here is timed: the timelock only ends in expiry (D11). */
@Composable
fun RecoveryNoticeScreen(state: NoticeState, on: NoticeActions) {
    SecureWindow()
    val v = state.view
    if (v == null) {
        ScreenFrame(TITLE, on.onBack, bar = { SshubButton("Back to recovery", on.onLeave, Modifier.fillMaxWidth(), ButtonKind.Outline) }) {
            if (state.loaded) EmptyState("This notice is gone", "No recovery notice with this id is known to this phone. Scan it again from a terminal.")
        }
        return
    }
    if (state.phase == NoticePhase.Signed && state.signed != null) {
        ScreenFrame(
            TITLE, on.onBack,
            banner = { RedBanner("CANCELLATION SIGNED", kind = BannerKind.Green, trailing = "recovery") },
            bar = { CopyBar(state.copied, on.onCopy, on.onDone) },
        ) {
            Text("Cancellation signed", Modifier.semantics { heading() }, style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
            Text("${v.startedBy} recovery, signed with this phone", style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Gap()
            HandoffCard(
                state.signed, "QR code with the signed recovery cancellation. Use the text below if the terminal has no camera",
                "Hold the QR code to any terminal, or paste the text into it. It carries the cancellation and the head of this phone chain.",
                state.copied, state.clearsIn,
            )
        }
        return
    }
    val confirm = state.phase == NoticePhase.Confirm
    ScreenFrame(
        TITLE, on.onBack,
        banner = { RedBanner("RECOVERY YOU DID NOT START?") },
        bar = {
            if (confirm) {
                SshubButton("Sign the cancellation", on.onSign, Modifier.fillMaxWidth(), ButtonKind.Danger, enabled = !state.busy)
                SshubButton("Not yet", on.onNotYet, Modifier.fillMaxWidth(), ButtonKind.Outline)
            } else {
                SshubButton(
                    "Cancel this recovery: I still have this phone", on.onAskCancel, Modifier.fillMaxWidth(), ButtonKind.Danger,
                    enabled = v.cancellable, disabledReason = v.closedReason,
                )
                SshubButton("Leave it (I started this)", on.onLeave, Modifier.fillMaxWidth(), ButtonKind.Outline)
            }
        },
    ) {
        if (confirm) {
            WarnBox("Cancel this recovery?", live = true) {
                Text(
                    "This phone signs a cancellation. Do it only if you did not start this recovery. If you did, leave it.",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
                )
            }
            Gap()
        }
        if (state.error != null) {
            WarnBox(state.error, live = true)
            Gap()
        }
        Text(TITLE, Modifier.semantics { heading() }, style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
        Text("A phone recovery was proposed. If you did not start it, cancel it.", style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Gap()
        DiffBox(
            listOf(
                DiffEntry("Started by", null, v.startedBy, mono = true),
                DiffEntry("Lamport", null, v.lamport.toString(), mono = true),
                DiffEntry("Operation", null, v.operation, mono = true),
                DiffEntry("Started", null, "${formatAgo(state.now - v.startedAt)}, ${v.startedUtc}", mono = true),
            ),
        )
        if (v.opRows.isNotEmpty()) {
            SectionHeading("Fields of the operation", trailing = "${v.opRows.size} fields")
            DiffBox(v.opRows.map { DiffEntry(it.key, it.from, it.to, it.mono, it.danger) })
        }
        Gap()
        NoticeHash(v.hash)
        Gap()
        Timelock(v, state.now)
        Gap()
        Note(v.signatureNote)
        Gap()
        if (!v.cancellable && v.closedReason != null) {
            WarnBox(v.closedReason)
            Gap()
        }
        WarnBox("Cancel only a recovery you did not start.") {
            Text(
                "A phone that can still sign proves the recovery is an attack, so no other factor is needed.",
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
            )
        }
    }
}

/** D7: the first 12 digits in 3 groups, all 16 groups on tap. */
@Composable
private fun NoticeHash(hash: String) {
    var full by remember { mutableStateOf(false) }
    Text("Notice hash", style = SshubText.fieldKey, color = MaterialTheme.colorScheme.onSurfaceVariant)
    HexGroups(hash, Modifier.heightIn(min = 48.dp).padding(top = 4.dp), groups = if (full) null else 3, onClick = { full = !full }, label = "Notice hash")
    Note(if (full) "All 16 groups. Tap to show the prefix again." else "Tap to show all 64 digits.")
}

@Composable
private fun Timelock(v: RecoveryNoticeView, now: Long) {
    val total = (v.endsAt - v.startedAt).coerceAtLeast(1)
    val left = v.endsAt - now
    val fraction = ((now - v.startedAt).toFloat() / total).coerceIn(0f, 1f)
    SshubCard {
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(formatRemaining(left), style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurface)
            Text("timelock ends ${v.endsUtc}", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Gap(8.dp)
        TickScale(
            28, 4, fraction,
            description = "Timelock of ${total / 3_600} hours, " + if (left > 0) "${formatRemaining(left)} left, ends ${v.endsUtc}" else "ended ${v.endsUtc}",
            labels = Triple("started", "${total / 86_400} day timelock", "ends"),
        )
    }
}
