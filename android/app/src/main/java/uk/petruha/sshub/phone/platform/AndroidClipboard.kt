package uk.petruha.sshub.phone.platform

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import android.os.SystemClock
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import uk.petruha.sshub.phone.core.ClipboardPort

/** Pure countdown for the clipboard clear: no Android, time is passed in (milliseconds from any monotonic clock). */
class ClipClearScheduler(private val ttlMs: Long = 60_000L) {
    private var deadline: Long? = null
    fun arm(nowMs: Long) { deadline = nowMs + ttlMs }
    fun disarm() { deadline = null }
    val armed: Boolean get() = deadline != null
    fun due(nowMs: Long): Boolean = deadline?.let { nowMs >= it } ?: false
    /** Whole seconds left, rounded up; null when nothing is armed. */
    fun remainingSeconds(nowMs: Long): Int? = deadline?.let { (((it - nowMs).coerceAtLeast(0L) + 999L) / 1000L).toInt() }
}

/**
 * D19. A copy carries EXTRA_IS_SENSITIVE (API 33+) and is cleared after 60 s. Android hides the clipboard from an app without window
 * focus (the read returns null) but lets it write, so an expiry while the app is away clears blind, stays pending, and is verified and
 * retried when the window has focus again (`onWindowFocusChanged`, which the Activity forwards).
 */
class AndroidClipboard(
    context: Context,
    private val nowMs: () -> Long = { SystemClock.elapsedRealtime() },
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Main),
) : ClipboardPort {
    private val cm: ClipboardManager = context.getSystemService(ClipboardManager::class.java)
    private val sched = ClipClearScheduler()
    private val left = MutableStateFlow<Int?>(null)
    override val clearsInSeconds: StateFlow<Int?> = left.asStateFlow()
    private var label = ""
    @Volatile private var pending = false
    private var ticker: Job? = null

    override fun copySensitive(label: String, text: String) {
        val clip = ClipData.newPlainText(label, text)
        if (Build.VERSION.SDK_INT >= 33) {
            clip.description.extras = PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true) }
        }
        cm.setPrimaryClip(clip)
        this.label = label
        pending = false
        focused = true // a copy is made from the screen the person is looking at
        sched.arm(nowMs())
        left.value = sched.remainingSeconds(nowMs())
        ticker?.cancel()
        ticker = scope.launch { while (isActive) { delay(1000); tick() } }
    }

    private val appCtx = context.applicationContext

    override fun readText(): String? = cm.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(appCtx)?.toString()

    /** One countdown step; also called on resume and stop. */
    internal fun tick() {
        val now = nowMs()
        if (sched.due(now)) {
            sched.disarm()
            ticker?.cancel()
            left.value = null
            pending = true
            clearIfOurs(blindOk = !focused)
        } else {
            left.value = sched.remainingSeconds(now)
        }
    }

    /** The window gets focus after resume, so a clear still pending is retried for a couple of seconds, reading only when Android allows it. */
    fun onResume() {
        tick()
        if (!pending) return
        retry?.cancel()
        retry = scope.launch {
            repeat(8) {
                if (!pending) return@launch
                clearIfOurs(blindOk = false)
                delay(250)
            }
        }
    }

    fun onStop() { focused = false; tick() }

    /**
     * Window focus is what Android requires to read the clipboard: the Activity forwards its `onWindowFocusChanged` here, and a clear
     * still pending is retried the moment the window has focus.
     */
    fun onWindowFocusChanged(hasFocus: Boolean) {
        focused = hasFocus
        if (hasFocus) clearIfOurs(blindOk = false)
    }

    private fun access(): ClipAccess = clipAccess(cm.primaryClipDescription, label)

    private fun clearIfOurs(blindOk: Boolean) {
        if (!pending) return
        when (access()) {
            // Somebody else's clip replaced ours: not ours to touch.
            ClipAccess.Foreign -> pending = false
            ClipAccess.Ours -> {
                tryClear()
                // A clear that was ignored leaves the description in place: it only counts once it is gone.
                pending = access() == ClipAccess.Ours
            }
            // No description: the clipboard is empty, or Android hides it from an app without focus. With focus it is empty (nothing of
            // ours is left). Without focus a write is still allowed, so the expiry clears blind; it stays pending until a read proves it.
            ClipAccess.None -> if (focused) pending = false else if (blindOk) tryClear()
        }
    }

    private fun tryClear() {
        try { cm.clearPrimaryClip() } catch (_: SecurityException) { }
    }

    @Volatile private var focused = true
    private var retry: Job? = null
}

internal enum class ClipAccess { None, Ours, Foreign }

/** No description (empty or unreadable), our clip (label and, from API 33, the sensitive flag match), or somebody else's. */
internal fun clipAccess(d: ClipDescription?, label: String, sdk: Int = Build.VERSION.SDK_INT): ClipAccess = when {
    d == null -> ClipAccess.None
    d.label?.toString() == label && (sdk < 33 || d.extras?.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE) == true) -> ClipAccess.Ours
    else -> ClipAccess.Foreign
}
