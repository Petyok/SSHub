package uk.petruha.sshub.phone.platform

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import uk.petruha.sshub.phone.core.AudioPort

/** Mono 16-bit playback of the FSK PCM. The volume is the user's: this class never touches AudioManager. */
class AndroidAudio : AudioPort {
    private val _playing = MutableStateFlow(false)
    override val playing: StateFlow<Boolean> = _playing.asStateFlow()
    @Volatile private var stopSignal: CompletableDeferred<Unit>? = null

    override suspend fun play(pcm: ShortArray, rate: Int) {
        if (pcm.isEmpty()) return
        stop()
        val signal = CompletableDeferred<Unit>()
        stopSignal = signal
        val track = AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_ASSISTANCE_SONIFICATION)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                    .build(),
            )
            .setAudioFormat(
                AudioFormat.Builder()
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setSampleRate(rate)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                    .build(),
            )
            .setTransferMode(AudioTrack.MODE_STATIC)
            .setBufferSizeInBytes(pcm.size * 2)
            .build()
        try {
            track.write(pcm, 0, pcm.size)
            _playing.value = true
            track.play()
            while (!signal.isCompleted && track.playbackHeadPosition < pcm.size) delay(40)
        } finally {
            runCatching { track.stop() }
            track.release()
            if (stopSignal === signal) { stopSignal = null; _playing.value = false }
        }
    }

    override fun stop() { stopSignal?.complete(Unit) }
}
