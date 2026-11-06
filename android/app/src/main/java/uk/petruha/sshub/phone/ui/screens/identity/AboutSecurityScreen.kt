package uk.petruha.sshub.phone.ui.screens.identity

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.CheckLine
import uk.petruha.sshub.phone.ui.components.FingerprintGrid
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.HexGroups
import uk.petruha.sshub.phone.ui.components.KeyValue
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.SshubRow
import uk.petruha.sshub.phone.ui.components.StopBox
import uk.petruha.sshub.phone.ui.components.TextArea
import uk.petruha.sshub.phone.ui.components.hexGroups
import uk.petruha.sshub.phone.ui.theme.SshubText
import uk.petruha.sshub.phone.ui.theme.sshub

/** How the value typed from the terminal compares with this app's signing fingerprint. */
enum class FingerprintCompare { None, Incomplete, Invalid, Equal, Differ }

/** Lower case, no spaces, colons or dashes: the form both sides are compared in. */
fun normalizeFingerprint(text: String): String = text.filter { !it.isWhitespace() && it != ':' && it != '-' }.lowercase()

fun compareFingerprint(app: String, reference: String): FingerprintCompare {
    val r = normalizeFingerprint(reference)
    return when {
        r.isEmpty() -> FingerprintCompare.None
        r.any { it !in '0'..'9' && it !in 'a'..'f' } -> FingerprintCompare.Invalid
        r.length < app.length -> FingerprintCompare.Incomplete
        r == normalizeFingerprint(app) -> FingerprintCompare.Equal
        else -> FingerprintCompare.Differ
    }
}

/** One bundled licence: the heading line, the upstream source and the unchanged text from assets/licenses. */
data class LicenceText(val name: String, val copyright: String, val source: String, val text: String)

data class AboutState(
    val versionName: String = "",
    val deviceName: String = "",
    val deviceId: String = "",
    /** SHA-256 of the signing certificate, 64 hex digits. */
    val fingerprint: String = "",
    /** What the user typed or pasted from the terminal. */
    val reference: String = "",
    val comparing: Boolean = false,
    val checked: Set<Int> = emptySet(),
    val autoLockSeconds: Int = 60,
    val strongBox: Boolean = false,
    val backupConfirmed: Boolean = false,
    val licencesOpen: Boolean = false,
    val licences: List<LicenceText> = emptyList(),
) {
    val compare: FingerprintCompare get() = if (comparing) compareFingerprint(fingerprint, reference) else FingerprintCompare.None
}

class AboutActions(
    val onToggleCompare: () -> Unit = {},
    val onReference: (String) -> Unit = {},
    val onToggleGroup: (Int) -> Unit = {},
    val onBackup: () -> Unit = {},
    val onToggleLicences: () -> Unit = {},
    val onBack: () -> Unit = {},
)

/** About and security (spec 12 `about`, D17): version, the signing fingerprint with a compare against the terminal, lock facts, what the app is not. */
@Composable
fun AboutSecurityScreen(state: AboutState, on: AboutActions) {
    SecureWindow()
    val differ = state.compare == FingerprintCompare.Differ
    ScreenFrame(title = "About and security", onBack = on.onBack) {
        SectionHeading("App", first = true)
        SshubCard {
            KeyValue("version", state.versionName)
            KeyValue("this phone", "${state.deviceName} ${state.deviceId}")
        }
        SectionHeading(
            "Signing fingerprint",
            trailing = when (state.compare) {
                FingerprintCompare.Equal -> "matches terminal"
                FingerprintCompare.Differ -> "DIFFERS from terminal"
                else -> null
            },
        )
        SshubCard {
            Text("this app, sha256", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Gap(8.dp)
            FingerprintGrid(state.fingerprint, state.checked, on.onToggleGroup)
            if (state.comparing) {
                Gap(12.dp)
                Text(
                    "${state.checked.size} of ${hexGroups(state.fingerprint).size} groups marked. Mark a group when it equals the same group on the terminal.",
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Gap(12.dp)
                Text("what the terminal prints", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Gap(6.dp)
                TextArea(state.reference, on.onReference, label = "Fingerprint printed by the terminal", placeholder = "64 hex digits, spaces allowed")
                Gap(8.dp)
                when (state.compare) {
                    FingerprintCompare.Incomplete -> Note("${normalizeFingerprint(state.reference).length} of ${state.fingerprint.length} digits typed.")
                    FingerprintCompare.Invalid -> CheckLine("Only the digits 0 to 9 and the letters a to f belong in a fingerprint.", ok = false)
                    FingerprintCompare.Equal -> CheckLine("All ${hexGroups(state.fingerprint).size} groups equal the value of the terminal.", ok = true)
                    else -> Unit
                }
            }
            if (differ) {
                Gap(12.dp)
                MismatchBox(state)
            }
            if (!differ) {
                Gap(12.dp)
                SshubButton(
                    if (state.comparing) "Stop comparing" else "Compare with terminal", on.onToggleCompare, Modifier.fillMaxWidth(),
                    kind = if (state.comparing) ButtonKind.Outline else ButtonKind.Primary,
                )
            }
            Gap(12.dp)
            Note("The fingerprint detects a wrong or outdated build. It does not detect a malicious app signed by the same key: compare it with what your terminal prints.")
        }
        SectionHeading("Lock", trailing = "system prompt")
        SshubCard {
            KeyValue("unlock", "fingerprint, face or screen lock, asked by Android", mono = false)
            KeyValue("auto-lock", autoLockLabel(state.autoLockSeconds))
            KeyValue(
                "key storage",
                if (state.strongBox) "StrongBox, a separate security chip" else "Trusted execution environment, no StrongBox chip on this phone",
                mono = false,
            )
            KeyValue("wrong tries", "Android counts them and blocks the unlock for a while. The app has no PIN of its own.", mono = false)
        }
        SectionHeading("Recovery")
        SshubRow(
            "Recovery words",
            secondary = if (state.backupConfirmed) "Confirmed on paper." else "24 words, shown after you hold.",
            onClick = on.onBackup,
        )
        Gap(8.dp)
        SshubRow(
            "Licences",
            Modifier.semantics { stateDescription = if (state.licencesOpen) "Open" else "Closed" },
            secondary = "Inter and JetBrains Mono, both SIL OFL 1.1",
            onClick = on.onToggleLicences,
        )
        if (state.licencesOpen) {
            Gap(8.dp)
            Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                state.licences.forEach { l ->
                    SshubCard {
                        Text(l.name, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
                        Text(l.copyright, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Text(l.source, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Gap(8.dp)
                        Text(l.text, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurface)
                    }
                }
            }
        }
        SectionHeading("What this app is not")
        SshubCard {
            Note("Not a password manager. It holds no credential keys and no copy of your hosts: the terminal sends it only its own wrapped share inside each request. It is a share holder that approves, and never a place to read a password.")
            Gap(8.dp)
            Note("A rooted or compromised Android can read the secret while the app is unlocked. The Keystore does not protect against that; the freshness limits and revocation do.")
        }
    }
}

/** D14, D17: both values in full, no way forward except Back. */
@Composable
private fun MismatchBox(state: AboutState) {
    val app = hexGroups(state.fingerprint)
    val ref = hexGroups(normalizeFingerprint(state.reference))
    val differing = app.indices.filter { app[it] != ref.getOrNull(it) }.map { it + 1 }
    StopBox("Signing fingerprint mismatch", live = true) {
        Text(
            "This app is not signed by the key the terminal expects. Do not use it for anything that matters. Delete it and install the app again from the source you trust.",
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
        )
        Text("this app", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        HexGroups(state.fingerprint, groups = null, label = "App fingerprint")
        Text("the terminal", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        HexGroups(normalizeFingerprint(state.reference), groups = null, label = "Terminal fingerprint")
        Text(
            "Groups that differ: " + differing.joinToString(", "),
            style = MaterialTheme.typography.bodyMedium.copy(color = MaterialTheme.sshub.dangerText),
        )
    }
}
