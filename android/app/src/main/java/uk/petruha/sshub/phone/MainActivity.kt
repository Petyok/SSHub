package uk.petruha.sshub.phone

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
        setContent { AppRoot(core, platform, settings) }
    }
}
