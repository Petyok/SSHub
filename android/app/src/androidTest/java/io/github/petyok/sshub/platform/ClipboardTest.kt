package io.github.petyok.sshub.platform

import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import io.github.petyok.sshub.MainActivity

class ClipboardTest {
    @Test fun scheduler_counts_down_and_expires() {
        val s = ClipClearScheduler(60_000)
        assertNull(s.remainingSeconds(0))
        assertFalse(s.due(0))
        s.arm(1_000)
        assertEquals(60, s.remainingSeconds(1_000))
        assertEquals(60, s.remainingSeconds(1_500))
        assertEquals(1, s.remainingSeconds(60_999))
        assertFalse(s.due(60_999))
        assertTrue(s.due(61_000))
        assertEquals(0, s.remainingSeconds(70_000))
        s.disarm()
        assertNull(s.remainingSeconds(70_000))
        assertFalse(s.due(70_000))
    }

    @Test fun copy_is_marked_sensitive_and_clears_after_60s() {
        val ctx = InstrumentationRegistry.getInstrumentation().targetContext
        val cm = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        var now = 0L
        val job = Job()
        // The activity gives the test process the focus Android requires for clipboard access.
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity {
                val clip = AndroidClipboard(ctx, nowMs = { now }, scope = CoroutineScope(job))
                clip.copySensitive("Words", "abc def")
                assertEquals(60, clip.clearsInSeconds.value)
                if (Build.VERSION.SDK_INT >= 33) {
                    val d = cm.primaryClipDescription!!
                    assertTrue(d.extras!!.getBoolean(ClipDescription.EXTRA_IS_SENSITIVE))
                }
                now = 30_000
                clip.tick()
                assertEquals(30, clip.clearsInSeconds.value)
                assertEquals("Words", cm.primaryClipDescription?.label?.toString())
                now = 60_000
                clip.tick()
                assertNull(clip.clearsInSeconds.value)
                job.cancel()
                assertNull(cm.primaryClipDescription)
            }
        }
    }
}
