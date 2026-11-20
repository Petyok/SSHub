package io.github.petyok.sshub.platform

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.SystemClock
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import io.github.petyok.sshub.core.Clock
import io.github.petyok.sshub.core.Platform

/**
 * Production ports, ONE per process (like the core): the retained ViewModels keep the instance they were built with, so the ports
 * must outlive every Activity. Only what is bound to an Activity is re-pointed by `attach`: the BiometricGate, the camera and BLE
 * permission launchers and the lifecycle observer. `bleLinks` is the core's `ble_new`; without it the Bluetooth port reports Unsupported.
 */
/** What the app plugs into the platform's locking: the configured auto-lock time and how to drop the core's unlocked state. */
class LockPolicy(val autoLockSeconds: () -> Int, val lockCore: () -> Unit)

class AndroidPlatform internal constructor(
    context: Context, bleLinks: BleLinkFactory?,
    private val elapsedMs: () -> Long = { SystemClock.elapsedRealtime() },
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Main),
) : Platform {
    private val app = context.applicationContext
    @Volatile private var gate: BiometricGate? = null
    private var attached: Pair<FragmentActivity, LifecycleEventObserver>? = null

    /** Set by the Activity on every `onCreate`; the platform outlives it. */
    @Volatile var lockPolicy: LockPolicy? = null
    private var stoppedAt: Long? = null
    private var timer: Job? = null

    /** The one lock: the core drops its unlocked state and the vault needs a prompt again. The UI follows the vault state. */
    fun lockNow() {
        timer?.cancel(); timer = null
        lockPolicy?.lockCore?.invoke()
        keys.lock()
    }

    /** Screen off locks at once. */
    fun onScreenOff() = lockNow()

    /** The app left the foreground: lock after the configured time, measured on the monotonic clock. */
    internal fun onAppStopped() {
        val seconds = lockPolicy?.autoLockSeconds?.invoke() ?: return
        stoppedAt = elapsedMs()
        timer?.cancel()
        timer = scope.launch { delay(seconds * 1000L); lockNow() }
    }

    /** Back in the foreground: lock if the time ran out while the timer could not (process frozen). */
    internal fun onAppStarted() {
        timer?.cancel(); timer = null
        val at = stoppedAt ?: return
        stoppedAt = null
        val seconds = lockPolicy?.autoLockSeconds?.invoke() ?: return
        if (elapsedMs() - at >= seconds * 1000L) lockNow()
    }

    // One receiver for the life of the process, on the application context: it never captures an Activity and cannot leak.
    private val screenOff = object : BroadcastReceiver() {
        override fun onReceive(c: Context?, i: Intent?) { onScreenOff() }
    }
    init { app.registerReceiver(screenOff, IntentFilter(Intent.ACTION_SCREEN_OFF), Context.RECEIVER_NOT_EXPORTED) }

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
    override val audio = AndroidAudio(app)
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
                Lifecycle.Event.ON_STOP -> { clipboard.onStop(); onAppStopped() }
                Lifecycle.Event.ON_START -> onAppStarted()
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
