package uk.petruha.sshub.phone

import android.content.Intent
import android.content.IntentFilter
import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.fragment.app.FragmentActivity
import uk.petruha.sshub.phone.app.AppRoot
import uk.petruha.sshub.phone.app.CoreFactoryImpl
import uk.petruha.sshub.phone.platform.AndroidPlatform

/** The single Activity. FragmentActivity because BiometricPrompt needs one. */
class MainActivity : FragmentActivity() {
    private val platform by lazy { CoreFactoryImpl.appContext = applicationContext; AndroidPlatform(applicationContext, CoreFactoryImpl.bleLinks) }
    private val core by lazy { CoreFactoryImpl.appContext = applicationContext; CoreFactoryImpl.create() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        platform.attach(this) // before setContent: permission launchers must be registered before STARTED
        // Screen off locks the app at once (spec 11).
        registerReceiver(object : android.content.BroadcastReceiver() {
            override fun onReceive(c: android.content.Context?, i: Intent?) { core.lock(); platform.keys.lock() }
        }, IntentFilter(Intent.ACTION_SCREEN_OFF), RECEIVER_NOT_EXPORTED)
        setContent { AppRoot(core, platform) }
    }
}
