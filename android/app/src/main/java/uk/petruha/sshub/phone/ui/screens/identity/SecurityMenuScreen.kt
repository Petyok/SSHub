package uk.petruha.sshub.phone.ui.screens.identity

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.AutoLockChoices
import uk.petruha.sshub.phone.core.ThemeChoice
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SshubRow
import uk.petruha.sshub.phone.ui.components.StopBox

data class SecurityMenuState(
    val deviceName: String = "",
    val deviceId: String = "",
    val theme: ThemeChoice = ThemeChoice.Dark,
    val autoLockSeconds: Int = AutoLockChoices.DEFAULT,
    val backupConfirmed: Boolean = false,
    /** D17: the signing fingerprint differs from the terminal's value; every flow except About and Lock now is closed. */
    val blocked: Boolean = false,
)

class SecurityMenuActions(
    val onTrustList: () -> Unit = {},
    val onCodes: () -> Unit = {},
    val onReshare: () -> Unit = {},
    val onRecovery: () -> Unit = {},
    val onBackup: () -> Unit = {},
    val onExport: () -> Unit = {},
    val onAbout: () -> Unit = {},
    val onLock: () -> Unit = {},
    val onTheme: (ThemeChoice) -> Unit = {},
    val onAutoLock: (Int) -> Unit = {},
)

/** Hub of the Security tab (spec 12 `menu`): holder flows, safety, app settings. */
@Composable
fun SecurityMenuScreen(state: SecurityMenuState, on: SecurityMenuActions) {
    val closed = "closed: signing fingerprint differs"
    fun go(open: () -> Unit): (() -> Unit)? = if (state.blocked) null else open
    ScreenFrame(title = "Security", onBack = null) {
        if (state.blocked) {
            StopBox("Signing fingerprint differs", live = true) {
                Text(
                    "This app is not signed by the key the terminal expects. Nothing that signs or releases is open until the fingerprints match. Open About and security to compare again.",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
                )
            }
        }
        SectionHeading("Holder", first = !state.blocked, trailing = "${state.deviceName} ${state.deviceId}")
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            SshubRow("Trust sessions", secondary = if (state.blocked) closed else "Certificates this phone issued", onClick = go(on.onTrustList))
            SshubRow("Short codes I issued", secondary = if (state.blocked) closed else "One live code per credential", onClick = go(on.onCodes))
            SshubRow("Reshare and wrap keys", secondary = if (state.blocked) closed else "After a holder is revoked", onClick = go(on.onReshare))
        }
        SectionHeading("Safety")
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            SshubRow("Recovery and a new phone", secondary = if (state.blocked) closed else "A recovery you did not start can be cancelled here", onClick = go(on.onRecovery))
            SshubRow(
                "Recovery words",
                secondary = if (state.blocked) closed else if (state.backupConfirmed) "24 words, confirmed on paper. Shown after a fresh unlock." else "24 words, needs your fingerprint, face or screen lock",
                onClick = go(on.onBackup),
            )
            SshubRow("Export my ops as text", secondary = if (state.blocked) closed else "For a terminal without a camera", onClick = go(on.onExport))
        }
        SectionHeading("Appearance", trailing = state.theme.name.lowercase())
        ChoiceGroup(
            listOf("System" to ThemeChoice.System, "Dark" to ThemeChoice.Dark, "Light" to ThemeChoice.Light),
            state.theme, on.onTheme,
        )
        SectionHeading("Auto-lock", trailing = "after the app leaves the screen")
        ChoiceGroup(AutoLockChoices.seconds.map { autoLockLabel(it) to it }, state.autoLockSeconds, on.onAutoLock)
        SectionHeading("App")
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            SshubRow("About and security", secondary = "Signing fingerprint, lock settings", onClick = on.onAbout)
            SshubRow("Lock now", secondary = "Wipes keys from memory", onClick = on.onLock)
        }
    }
}
