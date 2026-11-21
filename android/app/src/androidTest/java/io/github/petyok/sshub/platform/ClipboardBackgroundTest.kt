package io.github.petyok.sshub.platform

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.PersistableBundle
import androidx.activity.ComponentActivity
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * Runs in the plain ComponentActivity of ui-test-manifest: any resumed window gives the process the clipboard focus these cases
 * need, and the test does not depend on the app's own Activity.
 */
class ClipboardBackgroundTest {
    private val inst = InstrumentationRegistry.getInstrumentation()
    private val ctx: Context get() = inst.targetContext
    private val cm: ClipboardManager get() = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    private fun waitUntil(what: String, ms: Long = 10_000, cond: () -> Boolean) {
        val end = System.nanoTime() + ms * 1_000_000
        while (!cond()) {
            if (System.nanoTime() > end) throw AssertionError("timed out waiting for $what")
            Thread.sleep(50)
        }
    }

    // M11: "no access" (no description) is not "not ours".
    @Test fun access_tells_no_access_from_not_ours() {
        assertEquals(ClipAccess.None, clipAccess(null, "Words", 30))
        assertEquals(ClipAccess.Ours, clipAccess(ClipDescription("Words", arrayOf("text/plain")), "Words", 30))
        assertEquals(ClipAccess.Foreign, clipAccess(ClipDescription("Other", arrayOf("text/plain")), "Words", 30))
    }

    @Test fun from_api_33_our_clip_must_carry_the_sensitive_flag() {
        assumeTrue(Build.VERSION.SDK_INT >= 33)
        val d = ClipDescription("Words", arrayOf("text/plain"))
        assertEquals(ClipAccess.Foreign, clipAccess(d, "Words", 33))
        d.extras = PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true) }
        assertEquals(ClipAccess.Ours, clipAccess(d, "Words", 33))
    }

    @Test fun a_clip_another_app_put_there_is_left_alone() {
        var now = 0L
        val job = Job()
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity {
                val clip = AndroidClipboard(ctx, nowMs = { now }, scope = CoroutineScope(job))
                clip.copySensitive("Words", "abc def")
                cm.setPrimaryClip(ClipData.newPlainText("Other", "kept"))
                now = 61_000
                clip.tick()
                assertEquals("Other", cm.primaryClipDescription?.label?.toString())
            }
        }
        job.cancel()
    }

    // M11: copy, go home, let the 60 s pass while away, come back: the clip is cleared.
    @Test fun an_expiry_while_away_is_cleared_when_the_window_has_focus_again() {
        var now = 0L
        val job = Job()
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            lateinit var clip: AndroidClipboard
            scenario.onActivity {
                clip = AndroidClipboard(ctx, nowMs = { now }, scope = CoroutineScope(job))
                clip.copySensitive("Words", "abc def")
            }
            UiDevice.getInstance(inst).pressHome()
            waitUntil("the activity to stop") { scenario.state == Lifecycle.State.CREATED }
            clip.onStop()
            now = 61_000
            clip.tick() // the expiry, while the app is in the background
            ctx.startActivity(Intent(ctx, ComponentActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_REORDER_TO_FRONT))
            waitUntil("the activity to resume") { scenario.state == Lifecycle.State.RESUMED }
            waitUntil("window focus") {
                var focused = false
                scenario.onActivity { focused = it.window.decorView.hasWindowFocus() }
                focused
            }
            scenario.onActivity {
                clip.onWindowFocusChanged(true)
                assertNull(cm.primaryClipDescription)
            }
        }
        job.cancel()
    }
}
