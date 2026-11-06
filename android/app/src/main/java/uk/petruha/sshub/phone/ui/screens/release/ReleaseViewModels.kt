package uk.petruha.sshub.phone.ui.screens.release

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import uk.petruha.sshub.phone.core.ChannelKind
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.nameAndId
import uk.petruha.sshub.phone.core.OpenedRed
import uk.petruha.sshub.phone.core.OpenedRelease
import uk.petruha.sshub.phone.core.Platform
import uk.petruha.sshub.phone.core.ReleaseApi
import uk.petruha.sshub.phone.core.ReleaseChannels
import uk.petruha.sshub.phone.core.ReleaseDecision
import uk.petruha.sshub.phone.core.ReleaseOutcome
import uk.petruha.sshub.phone.core.VaultResult

/*
 * Thin view models: they hold the core sub-interface and the platform, tick `now` once per second and run the two decisions.
 * The tick only changes what is shown; no path leads from it to `decide` (D11).
 */

private const val CLIPBOARD_LABEL = "SSHub reply"

/** The system lock prompt every signing goes through (spec 11). Returns the text to show on failure, null to go on. */
private suspend fun Platform.confirmWithSystemLock(title: String, subtitle: String): String? {
    val r = keys.unlock(title, subtitle)
    val failure = vaultFailure(r)
    if (r is VaultResult.Secret) r.bytes.fill(0)
    return failure
}

class RedViewModel(private val core: ReleaseApi, private val platform: Platform, private val token: Long) : ViewModel() {
    private val _state = MutableStateFlow<RedState?>(null)
    /** Null when the core no longer holds the notice; the screen then goes back. */
    val state: StateFlow<RedState?> = _state.asStateFlow()

    init {
        val v = (core.pendingView(token) as? OpenedRed)?.view
        if (v != null) _state.value = RedState(v, platform.clock.nowSeconds())
        viewModelScope.launch { while (true) { delay(1000); _state.update { it?.copy(now = platform.clock.nowSeconds()) } } }
        viewModelScope.launch {
            platform.clipboard.clearsInSeconds.collect { c -> _state.update { s -> s?.let { it.copy(clearsIn = c, copied = it.copied && c != null) } } }
        }
    }

    fun toggleHash() = _state.update { it?.copy(showHash = !it.showHash) }
    fun typed(text: String) = _state.update { it?.copy(typed = text) }

    fun approve() {
        val s = _state.value ?: return
        if (!s.gate.enabled) return
        _state.update { it?.copy(phase = ApprovalPhase.Signing, error = null) }
        viewModelScope.launch {
            val failure = platform.confirmWithSystemLock("Approve Red operation", hashPrefix(s.view.hash))
            if (failure != null) { _state.update { it?.copy(phase = ApprovalPhase.Review, error = failure) }; return@launch }
            try {
                val out = core.decide(token, ReleaseDecision.Approve(typedId = s.view.mustType?.let { s.typed.trim() }), platform.clock.nowSeconds())
                _state.update { it?.copy(phase = ApprovalPhase.Approved, outcome = out) }
            } catch (e: CoreException) {
                _state.update { it?.copy(phase = ApprovalPhase.Review, error = (e.message ?: "The core refused.") + " Nothing was signed.") }
            }
        }
    }

    fun decline() {
        val s = _state.value ?: return
        if (s.phase != ApprovalPhase.Review) return
        viewModelScope.launch {
            try {
                core.decide(token, ReleaseDecision.Decline, platform.clock.nowSeconds())
                _state.update { it?.copy(phase = ApprovalPhase.Declined) }
            } catch (e: CoreException) {
                _state.update { it?.copy(error = e.message ?: "The core refused.") }
            }
        }
    }

    fun copy() {
        val text = _state.value?.outcome?.outgoing?.text ?: return
        platform.clipboard.copySensitive(CLIPBOARD_LABEL, text)
        _state.update { it?.copy(copied = true) }
    }
}

class ReleaseViewModel(private val core: ReleaseApi, private val platform: Platform, private val token: Long) : ViewModel() {
    private val _state = MutableStateFlow<ReleaseState?>(null)
    val state: StateFlow<ReleaseState?> = _state.asStateFlow()

    init {
        val v = (core.pendingView(token) as? OpenedRelease)?.view
        if (v != null) _state.value = ReleaseState(v, platform.clock.nowSeconds())
        viewModelScope.launch { while (true) { delay(1000); _state.update { it?.copy(now = platform.clock.nowSeconds()) } } }
        viewModelScope.launch {
            platform.clipboard.clearsInSeconds.collect { c -> _state.update { s -> s?.let { it.copy(clearsIn = c, copied = it.copied && c != null) } } }
        }
    }

    fun typed(text: String) = _state.update { it?.copy(typed = text) }
    fun channel(kind: ChannelKind, on: Boolean) = _state.update { it?.copy(channelsOn = if (on) it.channelsOn + kind else it.channelsOn - kind) }

    /** The one sound choice: off (null), fast or slow. Never two profiles at once. */
    fun sound(kind: ChannelKind?) = _state.update { s ->
        s?.copy(channelsOn = s.channelsOn - ChannelKind.SoundFast - ChannelKind.SoundSlow + listOfNotNull(kind))
    }
    fun shortCode(on: Boolean) = _state.update { it?.copy(shortCode = on) }
    fun byHand(on: Boolean) = _state.update { it?.copy(byHand = on) }

    fun approve() {
        val s = _state.value ?: return
        if (!s.gate.enabled) return
        _state.update { it?.copy(phase = ApprovalPhase.Signing, error = null) }
        viewModelScope.launch {
            val failure = platform.confirmWithSystemLock("Approve share release", nameAndId(s.view.requesterName, s.view.requesterId))
            if (failure != null) { _state.update { it?.copy(phase = ApprovalPhase.Review, error = failure) }; return@launch }
            val on = s.channelsOn
            val decision = ReleaseDecision.Approve(
                channels = ReleaseChannels(
                    qr = ChannelKind.Qr in on,
                    soundFast = ChannelKind.SoundFast in on, soundSlow = ChannelKind.SoundSlow in on,
                ),
                shortCode = s.shortCode, byHand = s.byHand, typedId = s.view.mustType?.let { s.typed.trim() },
            )
            val out: ReleaseOutcome
            try {
                out = core.decide(token, decision, platform.clock.nowSeconds())
            } catch (e: CoreException) {
                _state.update { it?.copy(phase = ApprovalPhase.Review, error = (e.message ?: "The core refused.") + " Nothing was signed.") }
                return@launch
            }
            _state.update { it?.copy(phase = ApprovalPhase.Approved, outcome = out) }
            val frame = out.outgoing ?: return@launch
            if (ChannelKind.SoundFast in on) frame.audioFast?.let { platform.audio.play(it, frame.audioRate) }
            if (ChannelKind.SoundSlow in on) frame.audioSlow?.let { platform.audio.play(it, frame.audioRate) }
        }
    }

    fun decline() {
        val s = _state.value ?: return
        if (s.phase != ApprovalPhase.Review) return
        viewModelScope.launch {
            try {
                core.decide(token, ReleaseDecision.Decline, platform.clock.nowSeconds())
                _state.update { it?.copy(phase = ApprovalPhase.Declined) }
            } catch (e: CoreException) {
                _state.update { it?.copy(error = e.message ?: "The core refused.") }
            }
        }
    }

    fun copy() {
        val text = _state.value?.outcome?.outgoing?.text ?: return
        platform.clipboard.copySensitive(CLIPBOARD_LABEL, text)
        _state.update { it?.copy(copied = true) }
    }
}
