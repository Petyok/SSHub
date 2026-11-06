package uk.petruha.sshub.phone

import android.os.Build
import android.view.View
import android.view.WindowManager
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.fragment.app.FragmentActivity
import uk.petruha.sshub.phone.app.AppRoot
import uk.petruha.sshub.phone.app.CoreFactoryImpl
import uk.petruha.sshub.phone.core.uniffi.UniffiPhoneCore
import uk.petruha.sshub.phone.platform.AndroidPlatform
import uk.petruha.sshub.phone.platform.FileSettingsStore
import uk.petruha.sshub.phone.platform.LockPolicy

/** The single Activity. FragmentActivity because BiometricPrompt needs one. */
class MainActivity : FragmentActivity() {
    internal val platform: AndroidPlatform get() { CoreFactoryImpl.appContext = applicationContext; return AndroidPlatform.shared(applicationContext) { CoreFactoryImpl.bleLinks } }
    private val core get() = CoreFactoryImpl.create()
    private val settings get() = FileSettingsStore.shared(applicationContext)

    /** Android lets an app read the clipboard only with window focus: a pending clear is retried here. */
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        platform.clipboard.onWindowFocusChanged(hasFocus)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        CoreFactoryImpl.appContext = applicationContext
        CoreFactoryImpl.configure(intent)
        window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) window.setHideOverlayWindows(true)
        enableEdgeToEdge()
        platform.lockPolicy = LockPolicy({ settings.autoLockSeconds.value }, { core.lock() })
        (core as? UniffiPhoneCore)?.let { it.blocked = { settings.signerMismatch.value }; it.vault = platform.keys }
        platform.attach(this) // before setContent: permission launchers must be registered before STARTED
        setContent {
            Box {
                AppRoot(core, platform, settings)
                if (BuildConfig.DEBUG) DebugBanner()
            }
        }
        // Drop taps that arrive while another window covers ours (tapjacking); set after setContent so the content view exists.
        findViewById<View>(android.R.id.content).filterTouchesWhenObscured = true
    }
}

/** Thin red strip so a debug install is never mistaken for the release app. */
@Composable
private fun DebugBanner() {
    Box(Modifier.fillMaxWidth().background(Color.Red).statusBarsPadding().padding(vertical = 1.dp), contentAlignment = Alignment.Center) {
        Text("debug build", color = Color.White, fontSize = 10.sp)
    }
}
