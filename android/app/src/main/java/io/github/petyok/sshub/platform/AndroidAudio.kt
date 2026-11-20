package io.github.petyok.sshub.platform

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.os.SystemClock
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import io.github.petyok.sshub.core.AudioPort

/** What the speaker is doing; [Failed] carries a sentence the screen can show (playback never throws into the caller). */
sealed interface AudioStatus {
    data object Idle : AudioStatus
    data object Playing : AudioStatus
    data class Failed(val reason: String) : AudioStatus
}

/** Pure helpers of the playback: the silent tail and the longest a burst may take. */
internal object FskPlayback {
    const val SILENCE_MS = 100
    const val MARGIN_MS = 1_500L

    /** The burst plus [SILENCE_MS] of silence, so the HAL cutting the last tens of milliseconds costs nothing. */
    fun withSilence(pcm: ShortArray, rate: Int): ShortArray = pcm.copyOf(pcm.size + (rate.toLong() * SILENCE_MS / 1000).toInt())

    /** The expected duration plus a margin: a track that is still not done by then has stalled. */
    fun boundMs(samples: Int, rate: Int): Long = samples * 1000L / rate + MARGIN_MS
}

/**
 * Mono 16-bit playback of the FSK PCM on the media stream (`USAGE_MEDIA`, so the media volume applies and silent or vibrate mode do
 * not mute it), under transient exclusive audio focus. The volume is the user's: this class never changes it; [mediaVolumeZero] says
 * when the media volume is zero so a screen can hint. With no [context] there is no focus and no volume check (tests of the pure parts).
 */
class AndroidAudio(private val context: Context? = null) : AudioPort {
    private val _playing = MutableStateFlow(false)
    override val playing: StateFlow<Boolean> = _playing.asStateFlow()
    private val _status = MutableStateFlow<AudioStatus>(AudioStatus.Idle)
    /** Idle, Playing, or Failed(reason) after a build or write error, a stalled track or lost focus. Reset when the next play starts. */
    val status: StateFlow<AudioStatus> = _status.asStateFlow()
    private val _volumeZero = MutableStateFlow(false)
    /** True while the media volume is zero or muted; refreshed by [refreshVolume] and at the start of every play. */
    val mediaVolumeZero: StateFlow<Boolean> = _volumeZero.asStateFlow()
    @Volatile private var stopSignal: CompletableDeferred<Unit>? = null
    @Volatile private var lostFocus = false

    private val audioManager: AudioManager? get() = context?.getSystemService(AudioManager::class.java)

    /** Re-reads the media volume (call when the screen that offers the sound is shown). */
    fun refreshVolume() {
        val am = audioManager ?: return
        _volumeZero.value = am.getStreamVolume(AudioManager.STREAM_MUSIC) == 0 || am.isStreamMute(AudioManager.STREAM_MUSIC)
    }

    private fun fail(reason: String) { _status.value = AudioStatus.Failed(reason) }

    override suspend fun play(pcm: ShortArray, rate: Int) {
        if (pcm.isEmpty()) return
        stop()
        refreshVolume()
        if (rate <= 0) { fail("The sound could not be played: invalid sample rate."); return }
        val signal = CompletableDeferred<Unit>()
        stopSignal = signal
        lostFocus = false
        _status.value = AudioStatus.Idle
        val data = FskPlayback.withSilence(pcm, rate)
        var track: AudioTrack? = null
        var focus: AudioFocusRequest? = null
        try {
            val attrs = AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .build()
            val t = AudioTrack.Builder()
                .setAudioAttributes(attrs)
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setSampleRate(rate)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                        .build(),
                )
                .setTransferMode(AudioTrack.MODE_STATIC)
                .setBufferSizeInBytes(data.size * 2)
                .build()
            track = t
            if (t.state != AudioTrack.STATE_INITIALIZED) { fail("The sound could not be played: the audio output refused the sound."); return }
            focus = requestFocus(attrs)
            val written = t.write(data, 0, data.size)
            if (written != data.size) { fail("The sound could not be played: the audio output took $written of ${data.size} samples."); return }
            _status.value = AudioStatus.Playing
            _playing.value = true
            t.play()
            val bound = FskPlayback.boundMs(data.size, rate)
            val start = SystemClock.elapsedRealtime()
            while (!signal.isCompleted && head(t) < data.size && SystemClock.elapsedRealtime() - start < bound) delay(40)
            when {
                lostFocus -> fail("Another app took the sound. Play it again.")
                !signal.isCompleted && head(t) < data.size -> fail("The sound did not play out. Check the media volume.")
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            fail("The sound could not be played: ${e.message ?: e.javaClass.simpleName}.")
        } finally {
            track?.let { runCatching { it.stop() }; runCatching { it.release() } }
            focus?.let { f -> runCatching { audioManager?.abandonAudioFocusRequest(f) } }
            if (stopSignal === signal) { stopSignal = null; _playing.value = false }
            if (_status.value == AudioStatus.Playing) _status.value = AudioStatus.Idle
        }
    }

    // playbackHeadPosition counts frames as an unsigned 32 bit value
    private fun head(t: AudioTrack): Long = t.playbackHeadPosition.toLong() and 0xFFFFFFFFL

    private fun requestFocus(attrs: AudioAttributes): AudioFocusRequest? {
        val am = audioManager ?: return null
        val req = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE)
            .setAudioAttributes(attrs)
            .setOnAudioFocusChangeListener { change ->
                if (change == AudioManager.AUDIOFOCUS_LOSS || change == AudioManager.AUDIOFOCUS_LOSS_TRANSIENT) {
                    lostFocus = true
                    stop()
                }
            }
            .build()
        // The result is not acted on: without focus the burst still plays, and a call that holds it mutes the sound by itself.
        am.requestAudioFocus(req)
        return req
    }

    override fun stop() { stopSignal?.complete(Unit) }
}
