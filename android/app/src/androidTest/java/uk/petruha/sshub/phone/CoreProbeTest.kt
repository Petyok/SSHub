package uk.petruha.sshub.phone

import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** The values below are the shared vectors: the phone fixture of the terminal and the BLAKE3 known answer. */
@RunWith(AndroidJUnit4::class)
class CoreProbeTest {
    @get:Rule val rule = createAndroidComposeRule<MainActivity>()

    @Test fun deviceIdComesFromTheRustCore() {
        rule.onNodeWithTag("device_id").assertTextEquals("ee8a1d5cf97016a7")
    }

    @Test fun blake3KnownAnswer() {
        rule.onNodeWithTag("blake3_empty")
            .assertTextEquals("af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262")
    }

    @Test fun signatureVerifies() {
        rule.onNodeWithTag("signature").assertTextEquals("signature ok")
    }
}
