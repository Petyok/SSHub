package uk.petruha.sshub.phone.platform

import android.content.Context
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import uk.petruha.sshub.phone.core.Clock
import uk.petruha.sshub.phone.core.Platform

/**
 * Production ports, ONE per process (like the core): the retained ViewModels keep the instance they were built with, so the ports
 * must outlive every Activity. Only what is bound to an Activity is re-pointed by `attach`: the BiometricGate, the camera and BLE
 * permission launchers and the lifecycle observer. `bleLinks` is the core's `ble_new`; without it the Bluetooth port reports Unsupported.
 */
class AndroidPlatform private constructor(context: Context, bleLinks: BleLinkFactory?) : Platform {
    private val app = context.applicationContext
    @Volatile private var gate: BiometricGate? = null
    private var attached: Pair<FragmentActivity, LifecycleEventObserver>? = null

    /** Tests only: answers the prompt instead of the BiometricGate. */
    @Volatile internal var promptOverride: VaultPrompt? = null

    /** The Activity the prompt, the launchers and the observer are bound to right now. */
    internal val attachedActivity: FragmentActivity? get() = attached?.first

    override val clock = Clock { System.currentTimeMillis() / 1000 }
    override val clipboard = AndroidClipboard(app)
    override val keys = AndroidKeyVault(app, prompt = { title, subtitle, cipher ->
        val o = promptOverride
        if (o != null) o(title, subtitle, cipher) else gate?.prompt(title, subtitle, cipher)
    })
    override val camera = AndroidCamera(app)
    override val ble = AndroidBle(app, bleLinks)
    override val audio = AndroidAudio()
    override val system = AndroidSystem(app)

    /** Call from `onCreate`, before STARTED (the permission launchers must be registered then). Safe to call again for a new Activity. */
    fun attach(activity: FragmentActivity) {
        detach()
        gate = BiometricGate(activity)
        camera.attach(activity)
        ble.attach(activity)
        val observer = LifecycleEventObserver { _, e ->
            when (e) {
                Lifecycle.Event.ON_RESUME -> clipboard.onResume()
                Lifecycle.Event.ON_PAUSE -> audio.stop()
                Lifecycle.Event.ON_STOP -> clipboard.onStop()
                Lifecycle.Event.ON_DESTROY -> if (attached?.first === activity) detach()
                else -> {}
            }
        }
        activity.lifecycle.addObserver(observer)
        attached = activity to observer
    }

    private fun detach() {
        attached?.let { (a, o) -> a.lifecycle.removeObserver(o) }
        attached = null
        gate = null
        camera.detach()
        ble.detach()
    }

    companion object {
        @Volatile private var instance: AndroidPlatform? = null

        /** The process-wide platform. `bleLinks` is read once, when the platform is first built. */
        fun shared(context: Context, bleLinks: () -> BleLinkFactory? = { null }): AndroidPlatform =
            instance ?: synchronized(this) { instance ?: AndroidPlatform(context, bleLinks()).also { instance = it } }
    }
}
