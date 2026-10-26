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
 * D19. A copy carries EXTRA_IS_SENSITIVE (API 33+) and is cleared after 60 s. Android blocks clipboard access from the background,
 * so an expiry that happens while the app is away stays pending and is retried on the next resume.
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
    private var pending = false
    private var ticker: Job? = null

    override fun copySensitive(label: String, text: String) {
        val clip = ClipData.newPlainText(label, text)
        if (Build.VERSION.SDK_INT >= 33) {
            clip.description.extras = PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true) }
        }
        cm.setPrimaryClip(clip)
        this.label = label
        pending = false
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
            clearIfOurs()
        } else {
            left.value = sched.remainingSeconds(now)
        }
    }

    fun onResume() { tick(); clearIfOurs() }
    fun onStop() { tick() }

    private fun clearIfOurs() {
        if (!pending) return
        val d = cm.primaryClipDescription
        if (d == null || !isOurs(d)) { pending = false; return }
        try { cm.clearPrimaryClip() } catch (_: SecurityException) { return }
        // A background clear is silently ignored: it only counts once the description is gone.
        pending = cm.primaryClipDescription?.let { isOurs(it) } ?: false
    }

    private fun isOurs(d: ClipDescription): Boolean =
        d.label?.toString() == label && (Build.VERSION.SDK_INT < 33 || d.extras?.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE) == true)
}
