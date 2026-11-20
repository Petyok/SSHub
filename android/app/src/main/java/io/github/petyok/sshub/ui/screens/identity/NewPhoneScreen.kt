package io.github.petyok.sshub.ui.screens.identity

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.dp
import io.github.petyok.sshub.ui.components.Gap
import io.github.petyok.sshub.ui.components.Note
import io.github.petyok.sshub.ui.components.ScreenFrame
import io.github.petyok.sshub.ui.components.SshubCard

/** The honest answer to "I am getting a new phone": there is no pairing with the old phone, the paper words do the move. */
@Composable
fun NewPhoneScreen(onBack: () -> Unit) {
    ScreenFrame(title = "I am getting a new phone", onBack = onBack) {
        Text(
            "Today a new phone is set up with your 24 paper words, then it reads its history from a terminal. There is no direct pairing with the old phone yet, " +
                "and your devices accept one phone at a time: use the 24 words on the new phone, then remove the old one.",
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
        )
        Gap(14.dp)
        SshubCard {
            Text("On the new phone", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
            Note("1. Open SSHub and choose Restore from 24 words.")
            Note("2. Type your 24 paper words.")
            Note("3. Take the phone's own history from a terminal before it signs anything.")
            Note("4. From a terminal, remove the old phone.")
        }
        Gap()
        Note("If the words are lost too, use I lost my phone in the Security menu.")
    }
}
