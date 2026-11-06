package uk.petruha.sshub.phone.ui.screens.trust

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.CodeIssuedView
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.KeyValue
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.StopBox
import uk.petruha.sshub.phone.ui.components.TickScale
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.components.WordKeycap
import uk.petruha.sshub.phone.ui.components.fontScale
import uk.petruha.sshub.phone.ui.components.hexGroups
import uk.petruha.sshub.phone.ui.components.spokenHex
import uk.petruha.sshub.phone.ui.theme.SshubText

/** `view` is null when nothing is pending (the code was already taken): the screen says so and offers the way back. */
data class CodeIssuedState(val view: CodeIssuedView?, val now: Long, val loaded: Boolean = true)

class CodeIssuedActions(val onDone: () -> Unit, val onBack: () -> Unit)

/**
 * Spec 12 `code-issued`: the 16 character short code, once, in 4 groups of 4 (mono keycaps), FLAG_SECURE, no copy button (D19).
 * Variant `live`: a code for this credential is already live, so a second one is refused.
 */
@Composable
fun CodeIssuedScreen(state: CodeIssuedState, on: CodeIssuedActions) {
    SecureWindow()
    when (val v = state.view) {
        is CodeIssuedView.Issued -> ScreenFrame(
            "Short code issued", on.onBack,
            bar = { SshubButton("I wrote it down", on.onDone, Modifier.fillMaxWidth(), kind = ButtonKind.Primary) },
        ) {
            val validDays = (v.expiresAt - v.issuedAt) / 86_400
            Text("Short code issued", style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onSurface)
            Text("Argon2id sealed, valid for $validDays days", style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Gap()
            CodeGroups(v.code)
            Gap()
            WarnBox("The code exists only on this screen.") {
                Text(
                    "Write it down or type it on the terminal. It is not shown again.",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
                )
            }
            Gap()
            SshubCard {
                KeyValue("credential", v.credential)
                KeyValue("expires", "${isoDate(v.expiresAt)}, in $validDays days")
                TickScale(
                    28, 4, 0f, "Expires ${isoDate(v.expiresAt)}, in $validDays days",
                    Modifier.padding(top = 4.dp), labels = Triple("now", "$validDays days", "expires"),
                )
            }
        }
        is CodeIssuedView.Live -> ScreenFrame(
            "Short code issued", on.onBack,
            bar = { SshubButton("Back to short codes", on.onDone, Modifier.fillMaxWidth()) },
        ) {
            StopBox("Refused. A short code is already live.", live = true) {
                Text(
                    "The code for ${v.credential} is live until ${isoDate(v.liveUntil)}. A second one is refused. " +
                        "Declare the live one dead first if its blob never arrived or the credential was reissued.",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
                )
            }
            Gap()
            SshubCard {
                KeyValue("credential", v.credential)
                KeyValue("live until", isoDate(v.liveUntil))
            }
        }
        null -> ScreenFrame("Short code issued", on.onBack, bar = { SshubButton("Back to short codes", on.onDone, Modifier.fillMaxWidth()) }) {
            Note("There is no new short code to show. A code is shown once, right after it is issued.")
        }
    }
}

/** Four groups of four. One row at normal size, 2 x 2 from font scale 1.3. Read aloud as one string, characters separated. */
@Composable
private fun CodeGroups(code: String) {
    val groups = hexGroups(code).map { it.uppercase() }
    Column(
        Modifier.fillMaxWidth().semantics { contentDescription = "Short code, " + spokenHex(groups) },
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        DisableSelection {
            val perRow = if (fontScale() >= 1.3f) 2 else 4
            groups.chunked(perRow).forEach { row ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    row.forEach { WordKeycap(it, Modifier.weight(1f).clearAndSetSemantics { }) }
                }
            }
        }
    }
}
