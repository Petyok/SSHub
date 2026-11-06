package uk.petruha.sshub.phone.ui.screens.trust

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import uk.petruha.sshub.phone.core.TrustHandoff
import uk.petruha.sshub.phone.ui.components.BannerKind
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ProofChain
import uk.petruha.sshub.phone.ui.components.ProofItem
import uk.petruha.sshub.phone.ui.components.ProofKind
import uk.petruha.sshub.phone.ui.components.RedBanner
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.theme.SshubText

/** `handoff` is null only when the screen is reopened after the process died: the signed reply is never persisted. */
data class TrustHandoffState(val handoff: TrustHandoff?, val clearsIn: Int? = null)
typealias TrustSignedState = TrustHandoffState
typealias TrustRevokedState = TrustHandoffState

class TrustHandoffActions(val onCopy: () -> Unit, val onDone: () -> Unit, val onBack: () -> Unit)
typealias TrustSignedActions = TrustHandoffActions
typealias TrustRevokedActions = TrustHandoffActions

/** Spec 12 `trust-signed`: signed certificate as QR and text, three proofs (this phone done, the other two waiting). */
@Composable
fun TrustSignedScreen(state: TrustSignedState, on: TrustSignedActions) {
    SecureWindow()
    val h = state.handoff
    ScreenFrame(
        "Trust session signed", on.onBack,
        banner = { RedBanner("SIGNED", kind = BannerKind.Green, trailing = "trust certificate") },
        bar = { if (h != null) HandoffBar(state.clearsIn, on.onDone, on.onCopy) else SshubButton("Done", on.onDone, Modifier.fillMaxWidth()) },
    ) {
        Text("Trust session signed", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
        if (h == null) { Gap(); Note("The signed answer is no longer on this phone. Ask the terminal for the request again."); return@ScreenFrame }
        Text(h.summary, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
        SectionHeading("Proofs", trailing = "1 of 3")
        SshubCard {
            ProofChain(
                listOf(
                    ProofItem("phone", "this phone", ProofKind.Signed),
                    ProofItem("second", "attended terminal", ProofKind.Waiting),
                    ProofItem("factor", "paper or email code", ProofKind.Waiting),
                ),
            )
        }
        Gap()
        HandoffCard(
            h.outgoing, "certificate",
            "The ring activates it after the second terminal and the third factor approve it. Carry this to the terminal: QR code, or the text.",
        )
    }
}

/** Spec 12 `trust-revoked`: signed revocation as QR and text. */
@Composable
fun TrustRevokedScreen(state: TrustRevokedState, on: TrustRevokedActions) {
    SecureWindow()
    val h = state.handoff
    ScreenFrame(
        "Revocation signed", on.onBack,
        banner = { RedBanner("REVOCATION SIGNED", kind = BannerKind.Neutral, trailing = "trust certificate") },
        bar = { if (h != null) HandoffBar(state.clearsIn, on.onDone, on.onCopy) else SshubButton("Done", on.onDone, Modifier.fillMaxWidth()) },
    ) {
        Text("Revocation signed", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
        if (h == null) { Gap(); Note("The signed revocation is no longer on this phone. Revoke the trust session again from the list."); return@ScreenFrame }
        Text(h.summary, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Gap()
        HandoffCard(h.outgoing, "revocation", "Carry this to any terminal: peers stop honouring the certificate on their next sync.")
    }
}
