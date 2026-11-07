package uk.petruha.sshub.phone.platform

import android.content.Context
import android.media.AudioManager
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeFalse
import org.junit.Test

class AudioTest {
    private val ctx: Context get() = InstrumentationRegistry.getInstrumentation().targetContext

    @Test fun the_burst_gets_100_ms_of_silence_and_a_bounded_duration() {
        val pcm = ShortArray(4_800) { 1000 }
        val out = FskPlayback.withSilence(pcm, 48_000)
        assertEquals(4_800 + 4_800, out.size)
        assertTrue(out.take(4_800).all { it == 1000.toShort() })
        assertTrue(out.drop(4_800).all { it == 0.toShort() })
        assertEquals(200L + FskPlayback.MARGIN_MS, FskPlayback.boundMs(9_600, 48_000))
    }

    // M7: a build error is a state, not a crash.
    @Test fun an_unsupported_rate_is_a_failed_state() = runBlocking {
        val a = AndroidAudio(ctx)
        a.play(ShortArray(100) { 100 }, 1)
        assertTrue(a.status.value is AudioStatus.Failed)
        assertFalse(a.playing.value)
        a.play(ShortArray(100) { 100 }, 0)
        assertTrue(a.status.value is AudioStatus.Failed)
    }

    // C-L10: play always returns within the expected duration plus the margin, even if the track never advances.
    @Test fun play_returns_within_its_bound_and_leaves_no_playing_state() = runBlocking {
        val a = AndroidAudio(ctx)
        val pcm = ShortArray(9_600) { (if (it % 2 == 0) 3000 else -3000).toShort() }
        val t0 = System.nanoTime()
        withTimeout(FskPlayback.boundMs(pcm.size + 4_800, 48_000) + 3_000) { a.play(pcm, 48_000) }
        assertTrue((System.nanoTime() - t0) / 1_000_000 < 10_000)
        assertFalse(a.playing.value)
        val s = a.status.value
        assertTrue("idle, or failed with a sentence", s == AudioStatus.Idle || s is AudioStatus.Failed)
    }

    @Test fun stop_ends_a_playing_burst() = runBlocking {
        val a = AndroidAudio(ctx)
        val pcm = ShortArray(48_000 * 20) { (if (it % 2 == 0) 3000 else -3000).toShort() }
        val job = launch(Dispatchers.Default) { a.play(pcm, 48_000) }
        val end = System.nanoTime() + 5_000_000_000L
        while (!a.playing.value && job.isActive && System.nanoTime() < end) Thread.sleep(10)
        a.stop()
        withTimeout(5_000) { job.join() }
        assertFalse(a.playing.value)
    }

    // M7: the screen can tell that the media volume is zero.
    @Test fun a_zero_media_volume_is_reported() {
        val am = ctx.getSystemService(AudioManager::class.java)
        assumeFalse("fixed volume device", am.isVolumeFixed)
        val before = am.getStreamVolume(AudioManager.STREAM_MUSIC)
        try {
            am.setStreamVolume(AudioManager.STREAM_MUSIC, 0, 0)
            val a = AndroidAudio(ctx)
            a.refreshVolume()
            assertTrue(a.mediaVolumeZero.value)
            am.setStreamVolume(AudioManager.STREAM_MUSIC, maxOf(1, before), 0)
            a.refreshVolume()
            assertFalse(a.mediaVolumeZero.value)
        } finally {
            am.setStreamVolume(AudioManager.STREAM_MUSIC, before, 0)
        }
    }
}
