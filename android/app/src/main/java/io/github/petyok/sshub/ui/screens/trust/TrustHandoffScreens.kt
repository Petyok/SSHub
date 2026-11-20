package io.github.petyok.sshub.ui.screens.trust

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import io.github.petyok.sshub.core.TrustHandoff
import io.github.petyok.sshub.ui.components.BannerKind
import io.github.petyok.sshub.ui.components.Gap
import io.github.petyok.sshub.ui.components.Note
import io.github.petyok.sshub.ui.components.ProofChain
import io.github.petyok.sshub.ui.components.ProofItem
import io.github.petyok.sshub.ui.components.ProofKind
import io.github.petyok.sshub.ui.components.RedBanner
import io.github.petyok.sshub.ui.components.ScreenFrame
import io.github.petyok.sshub.ui.components.SecureWindow
import io.github.petyok.sshub.ui.components.SectionHeading
import io.github.petyok.sshub.ui.components.SshubButton
import io.github.petyok.sshub.ui.components.SshubCard
import io.github.petyok.sshub.ui.theme.SshubText

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
        banner = { RedBanner("SIGNED", kind = BannerKind.Green, trailing = "trust session") },
        bar = { if (h != null) HandoffBar(state.clearsIn, on.onDone, on.onCopy) else SshubButton("Done", on.onDone, Modifier.fillMaxWidth()) },
    ) {
        Text("Trust session signed", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
        if (h == null) { Gap(); Note("The signed answer is no longer on this phone. Ask the terminal for the request again."); return@ScreenFrame }
        Text(h.summary, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
        SectionHeading("Approvals", trailing = "1 of 3")
        SshubCard {
            ProofChain(
                listOf(
                    ProofItem("phone", "this phone", ProofKind.Signed),
                    ProofItem("second", "second device", ProofKind.Waiting),
                    ProofItem("factor", "paper phrase or emailed code", ProofKind.Waiting),
                ),
            )
        }
        Gap()
        HandoffCard(
            h.outgoing, "trust session",
            "Your devices activate it after the second device and the paper phrase or the emailed code approve it. Carry this to the terminal: QR code, or the text.",
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
        banner = { RedBanner("REVOCATION SIGNED", kind = BannerKind.Neutral, trailing = "trust session") },
        bar = { if (h != null) HandoffBar(state.clearsIn, on.onDone, on.onCopy) else SshubButton("Done", on.onDone, Modifier.fillMaxWidth()) },
    ) {
        Text("Revocation signed", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
        if (h == null) { Gap(); Note("The signed revocation is no longer on this phone. Revoke the trust session again from the list."); return@ScreenFrame }
        Text(h.summary, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Gap()
        HandoffCard(h.outgoing, "revocation", "Carry this to any terminal: peers stop honouring the trust session on their next sync.")
    }
}
