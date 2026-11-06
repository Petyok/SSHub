package uk.petruha.sshub.phone

import android.content.Intent
import android.content.IntentFilter
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

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        CoreFactoryImpl.appContext = applicationContext
        CoreFactoryImpl.configure(intent)
        enableEdgeToEdge()
        platform.lockPolicy = LockPolicy({ settings.autoLockSeconds.value }, { core.lock() })
        (core as? UniffiPhoneCore)?.blocked = { settings.signerMismatch.value }
        platform.attach(this) // before setContent: permission launchers must be registered before STARTED
        // Screen off locks the app at once (spec 11).
        registerReceiver(object : android.content.BroadcastReceiver() {
            override fun onReceive(c: android.content.Context?, i: Intent?) { platform.onScreenOff() }
        }, IntentFilter(Intent.ACTION_SCREEN_OFF), RECEIVER_NOT_EXPORTED)
        setContent { AppRoot(core, platform, settings) }
    }
}
