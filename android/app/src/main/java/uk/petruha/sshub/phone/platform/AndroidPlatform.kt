package uk.petruha.sshub.phone.platform

import android.content.Context
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import uk.petruha.sshub.phone.core.Clock
import uk.petruha.sshub.phone.core.Platform

/**
 * Production ports. The Activity (a FragmentActivity, for BiometricPrompt) calls `attach(this)` in `onCreate`
 * (permission launchers must be registered before STARTED). `bleLinks` is the core's `ble_new`; without it the
 * Bluetooth port reports Unsupported.
 */
class AndroidPlatform(context: Context, bleLinks: BleLinkFactory? = null) : Platform {
    private val app = context.applicationContext
    @Volatile private var gate: BiometricGate? = null

    override val clock = Clock { System.currentTimeMillis() / 1000 }
    override val clipboard = AndroidClipboard(app)
    override val keys = AndroidKeyVault(app, prompt = { title, subtitle, cipher -> gate?.prompt(title, subtitle, cipher) })
    override val camera = AndroidCamera(app)
    override val ble = AndroidBle(app, bleLinks)
    override val audio = AndroidAudio()
    override val system = AndroidSystem(app)

    fun attach(activity: FragmentActivity) {
        val g = BiometricGate(activity)
        gate = g
        camera.attach(activity)
        ble.attach(activity)
        activity.lifecycle.addObserver(LifecycleEventObserver { _, e ->
            when (e) {
                Lifecycle.Event.ON_RESUME -> clipboard.onResume()
                Lifecycle.Event.ON_PAUSE -> audio.stop()
                Lifecycle.Event.ON_STOP -> clipboard.onStop()
                Lifecycle.Event.ON_DESTROY -> if (gate === g) gate = null
                else -> {}
            }
        })
    }
}
