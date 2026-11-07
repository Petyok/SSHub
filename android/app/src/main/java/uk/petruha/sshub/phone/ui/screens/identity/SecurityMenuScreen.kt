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
import uk.petruha.sshub.phone.ui.screens.home.TAKE_CHAIN_LABEL
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
    /** The chain was not taken yet (restored phone): the menu offers it again, whatever happened to the first attempt. */
    val headPending: Boolean = false,
)

class SecurityMenuActions(
    val onTakeChain: () -> Unit = {},
    val onTrustList: () -> Unit = {},
    val onCodes: () -> Unit = {},
    val onReshare: () -> Unit = {},
    val onRecovery: () -> Unit = {},
    val onNewPhone: () -> Unit = {},
    val onBackup: () -> Unit = {},
    val onExport: () -> Unit = {},
    val onAbout: () -> Unit = {},
    val onLock: () -> Unit = {},
    val onTheme: (ThemeChoice) -> Unit = {},
    val onAutoLock: (Int) -> Unit = {},
)

/** Hub of the Security tab (spec 12 `menu`): full-device flows, safety, app settings. */
@Composable
fun SecurityMenuScreen(state: SecurityMenuState, on: SecurityMenuActions) {
    val closed = "closed: signing fingerprint differs"
    fun go(open: () -> Unit): (() -> Unit)? = if (state.blocked) null else open
    ScreenFrame(title = "Security", onBack = null) {
        if (state.blocked) {
            StopBox("Signing fingerprint differs", live = true) {
                Text(
                    "This app is not signed by the key the terminal expects. Nothing that signs or releases is open. $GET_AGAIN",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
                )
                Text(REINSTALL_WARNING, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
            }
        }
        if (state.headPending) {
            SectionHeading("Chain", first = !state.blocked, trailing = "not taken yet")
            Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                SshubRow(TAKE_CHAIN_LABEL, secondary = if (state.blocked) closed else "This phone does not sign until it has taken its chain", onClick = go(on.onTakeChain))
            }
        }
        SectionHeading("Full device", first = !state.blocked && !state.headPending, trailing = "${state.deviceName} ${state.deviceId}")
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            SshubRow("Trust sessions", secondary = if (state.blocked) closed else "Computers this phone lets open some hosts without asking", onClick = go(on.onTrustList))
            SshubRow("Short codes I issued", secondary = if (state.blocked) closed else "One live code per credential", onClick = go(on.onCodes))
            SshubRow("Update the shares", secondary = if (state.blocked) closed else "After a device was removed", onClick = go(on.onReshare))
        }
        SectionHeading("Safety")
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            SshubRow("I am getting a new phone", secondary = if (state.blocked) closed else "Move to a new phone with your 24 paper words", onClick = go(on.onNewPhone))
            SshubRow("I lost my phone", secondary = if (state.blocked) closed else "A recovery you did not start can be cancelled here", onClick = go(on.onRecovery))
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
