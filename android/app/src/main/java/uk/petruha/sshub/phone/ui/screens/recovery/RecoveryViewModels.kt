package uk.petruha.sshub.phone.ui.screens.recovery

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.Platform
import uk.petruha.sshub.phone.core.RecoveryApi
import uk.petruha.sshub.phone.core.ReshareStage
import uk.petruha.sshub.phone.core.ReshareView
import uk.petruha.sshub.phone.core.WrapKeyState

private fun CoreException.text() = message?.takeIf { it.isNotBlank() } ?: code.name

/** `token` is set when the screen was opened for a reshare that `openFrame` returned. */
class ReshareViewModel(private val core: RecoveryApi, private val platform: Platform, private val token: Long?) : ViewModel() {
    private val _state = MutableStateFlow(ReshareState())
    val state: StateFlow<ReshareState> = _state.asStateFlow()

    init {
        guarded { load() }
        viewModelScope.launch { platform.clipboard.clearsInSeconds.collect { c -> _state.update { it.copy(clearsIn = c) } } }
    }

    private fun apply(w: WrapKeyState, cand: ReshareView?) = _state.update {
        it.copy(
            loaded = true, stage = w.stage, generation = w.generation, epoch = w.epoch, holderNumber = w.holderNumber,
            ready = w.credentialsReady, total = w.credentialsTotal, reshare = w.reshare, candidate = cand,
        )
    }

    private suspend fun load() {
        val now = platform.clock.nowSeconds()
        val w = core.wrapKeyState(now)
        apply(w, if (w.stage == ReshareStage.Published) null else token?.let { core.reshareCandidate(it, now) })
    }

    private fun guarded(block: suspend () -> Unit) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            try { block() } catch (e: CoreException) { _state.update { it.copy(error = e.text()) } }
            finally { _state.update { it.copy(busy = false) } }
        }
    }

    fun rotate() = guarded { core.rotateWrapKeys(platform.clock.nowSeconds()); load() }

    /** One call adopts and signs; the signed ops come back with it and are shown right away. */
    fun adoptPublish() {
        val t = token ?: return
        guarded {
            val out = core.decideReshare(t, platform.clock.nowSeconds())
            apply(out.state, null)
            _state.update { it.copy(outgoing = out.outgoing, copied = false) }
        }
    }

    fun copy() {
        val out = _state.value.outgoing ?: return
        platform.clipboard.copySensitive("Signed text", out.text)
        _state.update { it.copy(copied = true) }
    }
}

class RecoveryViewModel(private val core: RecoveryApi, private val platform: Platform) : ViewModel() {
    private val _state = MutableStateFlow(RecoveryState())
    val state: StateFlow<RecoveryState> = _state.asStateFlow()

    init { refresh() }

    fun refresh() {
        viewModelScope.launch {
            val now = platform.clock.nowSeconds()
            try { _state.value = RecoveryState(core.recoveryNotices(now), now) } catch (ex: CoreException) { _state.value = RecoveryState(emptyList(), now) }
        }
    }

    /** Updates the "left" texts only. */
    fun tick() = _state.update { it.copy(now = platform.clock.nowSeconds()) }
}

class RecoveryNoticeViewModel(private val core: RecoveryApi, private val platform: Platform, private val token: Long) : ViewModel() {
    private val _state = MutableStateFlow(NoticeState())
    val state: StateFlow<NoticeState> = _state.asStateFlow()

    init {
        viewModelScope.launch {
            val now = platform.clock.nowSeconds()
            try { _state.update { it.copy(loaded = true, view = core.recoveryNotice(token, now), now = now) } }
            catch (e: CoreException) { _state.update { it.copy(loaded = true, error = e.text(), now = now) } }
        }
        viewModelScope.launch { platform.clipboard.clearsInSeconds.collect { c -> _state.update { it.copy(clearsIn = c) } } }
    }

    fun askCancel() { if (_state.value.view?.cancellable == true) _state.update { it.copy(phase = NoticePhase.Confirm, error = null) } }
    fun notYet() = _state.update { it.copy(phase = NoticePhase.Review) }

    fun sign() {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            try {
                val out = core.decideRecoveryCancel(token, platform.clock.nowSeconds())
                _state.update { it.copy(phase = NoticePhase.Signed, signed = out) }
            } catch (e: CoreException) {
                _state.update { it.copy(phase = NoticePhase.Review, error = e.text()) }
            } finally { _state.update { it.copy(busy = false) } }
        }
    }

    fun copy() {
        val out = _state.value.signed ?: return
        platform.clipboard.copySensitive("Signed text", out.text)
        _state.update { it.copy(copied = true) }
    }

    /** Updates the timelock text only. */
    fun tick() = _state.update { it.copy(now = platform.clock.nowSeconds()) }
}

class ExportOpsViewModel(private val core: RecoveryApi, private val platform: Platform) : ViewModel() {
    private val _state = MutableStateFlow(ExportState())
    val state: StateFlow<ExportState> = _state.asStateFlow()

    init {
        viewModelScope.launch {
            try {
                val e = core.exportOps(platform.clock.nowSeconds())
                _state.update { it.copy(loaded = true, rows = e.rows, total = e.total, outgoing = e.outgoing) }
            } catch (ex: CoreException) { _state.update { it.copy(loaded = true) } }
        }
        viewModelScope.launch { platform.clipboard.clearsInSeconds.collect { c -> _state.update { it.copy(clearsIn = c) } } }
    }

    fun copy() {
        val out = _state.value.outgoing ?: return
        platform.clipboard.copySensitive("Signed text", out.text)
        _state.update { it.copy(copied = true) }
    }
}
