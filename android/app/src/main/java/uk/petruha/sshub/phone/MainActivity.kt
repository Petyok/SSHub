package uk.petruha.sshub.phone

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import uniffi.sshub_phone_core.probe

/** Phase 1 skeleton: one screen showing a value computed by the Rust core. Not styled on purpose. */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Public test vector (assets/companion/test/fixtures/challenges.json): secret = 0x42 x 32.
        val p = probe(ByteArray(32) { 0x42 })
        setContent {
            MaterialTheme {
                Column(Modifier.padding(24.dp)) {
                    Text("SSHub phone core", style = MaterialTheme.typography.titleMedium)
                    Text(p.deviceId, Modifier.testTag("device_id"))
                    Text(p.blake3Empty, Modifier.testTag("blake3_empty"))
                    Text(if (p.signatureOk) "signature ok" else "signature FAILED", Modifier.testTag("signature"))
                }
            }
        }
    }
}
