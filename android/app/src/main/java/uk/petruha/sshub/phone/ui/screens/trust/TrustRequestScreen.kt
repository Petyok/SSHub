package uk.petruha.sshub.phone.ui.screens.trust

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.TrustRequestView
import uk.petruha.sshub.phone.ui.components.BannerKind
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.HexGroups
import uk.petruha.sshub.phone.ui.components.MonoField
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.OwnProof
import uk.petruha.sshub.phone.ui.components.ProofKind
import uk.petruha.sshub.phone.ui.components.RedBanner
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.StopBox
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.theme.SshubText

/** Review: the request as the core describes it. Declined and Closed end without a signature; Signed is the hand-off (request: own screen). */
enum class TrustPhase { Review, Declined, Signed, Closed }

data class TrustRequestState(
    val view: TrustRequestView,
    val typed: String = "",
    val phase: TrustPhase = TrustPhase.Review,
    val busy: Boolean = false,
    val error: String? = null,
) {
    /** D13: a requester never met needs its id typed. The core checks it again. */
    val gateOk: Boolean get() = view.device.met || typed.trim().equals(view.device.id, ignoreCase = true)

    /** D8: Approve only when nothing refuses, every check passes and the gate is open. */
    val approveEnabled: Boolean
        get() = phase == TrustPhase.Review && !busy && view.refusal == null && view.epoch?.checks.orEmpty().all { it.ok } && gateOk
}

class TrustRequestActions(
    val onTyped: (String) -> Unit,
    val onApprove: () -> Unit,
    val onDecline: () -> Unit,
    val onBack: () -> Unit,
)

/** Spec 12 `trust-request`: Red certificate request, variants ok / unmet / refused (0..8, never an Approve) / closed. */
@Composable
fun TrustRequestScreen(state: TrustRequestState, on: TrustRequestActions) {
    SecureWindow()
    when (state.phase) {
        TrustPhase.Declined -> ScreenFrame(
            "Trust request", on.onBack,
            banner = { RedBanner("DECLINED", kind = BannerKind.Neutral, trailing = "trust certificate") },
            bar = { SshubButton("Back to approvals", on.onBack, Modifier.fillMaxWidth()) },
        ) {
            SshubCard {
                Text("Declined.", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                Note("Nothing was signed. This phone dropped the request: to look at it again, ask the terminal for a new one.")
            }
        }
        TrustPhase.Closed -> ScreenFrame(
            "Trust request", on.onBack,
            banner = { RedBanner("WINDOW CLOSED", kind = BannerKind.Neutral, trailing = "trust certificate") },
            bar = { SshubButton("Back to approvals", on.onBack, Modifier.fillMaxWidth()) },
        ) {
            SshubCard {
                Text("Window closed.", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                Note("The approval window of 12 minutes ended. Nothing was signed. Open the request again.")
            }
        }
        // Signed is shown by TrustSignedScreen; a request state never stays here.
        TrustPhase.Signed, TrustPhase.Review -> ReviewFrame(state, on)
    }
}

@Composable
private fun ReviewFrame(state: TrustRequestState, on: TrustRequestActions) {
    val v = state.view
    val refused = v.refusal != null
    ScreenFrame(
        "Trust request", on.onBack,
        banner = { RedBanner("TRUST CERTIFICATE", trailing = "Red operation") },
        bar = {
            if (refused) SshubButton("Back to approvals", on.onBack, Modifier.fillMaxWidth())
            else {
                if (!state.gateOk) Note("Type the device id in the field under the warning to enable Approve.")
                BarButtons {
                    SshubButton("Decline", on.onDecline, enabled = !state.busy)
                    SshubButton(
                        "Approve", on.onApprove, kind = ButtonKind.Primary, subtitle = "sign with the phone",
                        enabled = state.approveEnabled,
                        disabledReason = when {
                            state.busy -> "signing"
                            !state.gateOk -> "type the id first"
                            v.epoch?.checks.orEmpty().any { !it.ok } -> "a check failed"
                            else -> null
                        },
                    )
                }
            }
        },
    ) {
        Text("Trust session requested", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
        Text(v.headline, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Gap()
        v.refusal?.let { r ->
            StopBox(r.title, live = true) { Text(r.text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface) }
            Gap()
        }
        TrustFieldsBox(v.fields)
        v.epoch?.let { Gap(8.dp); EpochBox(it) }
        Gap()
        if (!refused && !v.device.met) {
            WarnBox("Never met") {
                Text(
                    "This phone has not met that device. Compare its id with the one on the terminal before you approve, then type it below.",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
                )
                Text("Device", style = SshubText.fieldKey, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(v.device.label, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurface)
                Text("Device id", style = SshubText.fieldKey, color = MaterialTheme.colorScheme.onSurfaceVariant)
                HexGroups(v.device.id, groups = null, label = "Device id")
            }
            Gap(8.dp)
            MonoField(
                state.typed, on.onTyped, placeholder = "Type ${v.device.id}", capitalize = true,
                label = "Type the device id of the requester",
                error = if (state.typed.length >= v.device.id.length && !state.gateOk) "That is not the id of the requester" else null,
            )
            Gap()
        }
        if (!refused) {
            SshubCard { OwnProof(ProofKind.YourTurn) }
            Gap(8.dp)
            Note(
                "The certificate takes effect only with three proofs: this phone, a second terminal and a third factor. " +
                    "This signature is the phone's part: it authors the certificate. No hash is shown: the core gives none for a trust request.",
            )
        }
        state.error?.let { Gap(); WarnBox(it, live = true) }
    }
}
