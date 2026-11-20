package io.github.petyok.sshub.app

import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import io.github.petyok.sshub.core.InMemorySettingsStore
import io.github.petyok.sshub.core.VaultState
import io.github.petyok.sshub.fake.FakePhoneCore
import io.github.petyok.sshub.fake.FakePlatform
import io.github.petyok.sshub.ui.components.SecureWindow
import io.github.petyok.sshub.ui.theme.SshubTheme

/** Review round 2: the activity-wide FLAG_SECURE survives a secure screen, and the block closes first run. */
class ShellRound2Test {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun a_secure_screen_leaving_does_not_clear_the_window_wide_flag() {
        rule.runOnUiThread { rule.activity.window.addFlags(WindowManager.LayoutParams.FLAG_SECURE) } // what MainActivity does once
        var show by mutableStateOf(true)
        rule.setContent { if (show) SecureWindow() }
        rule.waitForIdle()
        show = false
        rule.waitForIdle()
        assertTrue("FLAG_SECURE must still be set", rule.activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
    }

    @Test fun a_secure_screen_clears_the_flag_only_if_it_set_it() {
        var show by mutableStateOf(true)
        rule.setContent { if (show) SecureWindow() }
        rule.waitForIdle()
        assertTrue(rule.activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
        show = false
        rule.waitForIdle()
        assertTrue("it set the flag itself, so it clears it", rule.activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE == 0)
    }

    @Test fun first_run_is_closed_while_the_signer_block_is_set() {
        val settings = InMemorySettingsStore().also { it.setSignerMismatch(true) }
        val platform = FakePlatform().also { it.keys.st.value = VaultState.NoKey }
        rule.setContent { SshubTheme(dark = true, reducedMotion = false) { AppRoot(FakePhoneCore(), platform, settings) } }
        rule.onNodeWithText("About and security", substring = true).assertExists()
    }
}
