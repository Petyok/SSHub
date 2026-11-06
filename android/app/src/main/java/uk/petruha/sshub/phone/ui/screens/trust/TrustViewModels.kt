package uk.petruha.sshub.phone.ui.screens.trust

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import java.util.WeakHashMap
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import uk.petruha.sshub.phone.core.AppEnv
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.HandoffKind
import uk.petruha.sshub.phone.core.Platform
import uk.petruha.sshub.phone.core.TrustApi
import uk.petruha.sshub.phone.core.TrustDecision
import uk.petruha.sshub.phone.core.TrustHandoff

/** D10: an approval session closes 12 minutes after the screen was opened. Nothing is signed after that. */
internal const val APPROVAL_WINDOW_SECONDS = 12 * 60L

/** Carries the signed reply from the screen that signed to the hand-off screen. Never persisted; one per `AppEnv`. */
class TrustHandoffHolder { var current: TrustHandoff? = null }

private val holders = WeakHashMap<AppEnv, TrustHandoffHolder>()
internal fun trustHandoffs(env: AppEnv): TrustHandoffHolder = holders.getOrPut(env) { TrustHandoffHolder() }

private fun Throwable.shown(): String = (this as? CoreException)?.message?.takeIf { it.isNotBlank() }
    ?: ("Something went wrong." + if ((this as? CoreException)?.code == CoreException.Code.Refused) " Nothing was signed." else "")

class TrustRequestViewModel(
    private val core: TrustApi,
    private val platform: Platform,
    private val token: Long,
    private val handoff: TrustHandoffHolder,
) : ViewModel() {
    private val _state = MutableStateFlow<TrustRequestState?>(null)
    val state: StateFlow<TrustRequestState?> = _state
    private val openedAt = platform.clock.nowSeconds()

    init { viewModelScope.launch { core.trustRequest(token)?.let { _state.value = TrustRequestState(it) } } }

    fun typed(text: String) = _state.update { it?.copy(typed = text, error = null) }

    fun decline() {
        if (_state.value == null) return
        viewModelScope.launch {
            _state.update { it?.copy(busy = true, error = null) }
            try {
                core.decideTrust(token, TrustDecision.Decline, platform.clock.nowSeconds())
                _state.update { it?.copy(busy = false, phase = TrustPhase.Declined) }
            } catch (e: CoreException) {
                _state.update { it?.copy(busy = false, error = e.shown()) }
            }
        }
    }

    /** `onSigned` runs after the signed reply is in the hand-off holder. */
    fun approve(onSigned: () -> Unit) {
        val s = _state.value ?: return
        if (!s.approveEnabled) return
        val now = platform.clock.nowSeconds()
        if (now - openedAt > APPROVAL_WINDOW_SECONDS) { _state.update { it?.copy(phase = TrustPhase.Closed) }; return }
        viewModelScope.launch {
            _state.update { it?.copy(busy = true, error = null) }
            try {
                val out = core.decideTrust(token, TrustDecision.Approve(if (s.view.device.met) null else s.typed.trim()), now)
                    ?: throw CoreException(CoreException.Code.Internal, "no reply")
                handoff.current = TrustHandoff(HandoffKind.Certificate, s.view.signedSummary, out)
                _state.update { it?.copy(busy = false, phase = TrustPhase.Signed) }
                onSigned()
            } catch (e: CoreException) {
                _state.update { it?.copy(busy = false, error = e.shown()) }
            }
        }
    }
}

class TrustExtendViewModel(
    private val core: TrustApi,
    private val platform: Platform,
    private val token: Long,
) : ViewModel() {
    private val base = MutableStateFlow<TrustExtendState?>(null)
    val state: StateFlow<TrustExtendState?> =
        combine(base, platform.clipboard.clearsInSeconds) { s, c -> s?.copy(clearsIn = c) }
            .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)
    private val openedAt = platform.clock.nowSeconds()

    init { viewModelScope.launch { core.trustExtend(token)?.let { base.value = TrustExtendState(it) } } }


    fun decline() {
        viewModelScope.launch {
            base.update { it?.copy(busy = true, error = null) }
            try {
                core.decideTrust(token, TrustDecision.Decline, platform.clock.nowSeconds())
                base.update { it?.copy(busy = false, phase = TrustPhase.Declined) }
            } catch (e: CoreException) {
                base.update { it?.copy(busy = false, error = e.shown()) }
            }
        }
    }

    fun approve() {
        val s = base.value ?: return
        if (!s.approveEnabled) return
        val now = platform.clock.nowSeconds()
        if (now - openedAt > APPROVAL_WINDOW_SECONDS) { base.update { it?.copy(phase = TrustPhase.Closed) }; return }
        viewModelScope.launch {
            base.update { it?.copy(busy = true, error = null) }
            try {
                val out = core.decideTrust(token, TrustDecision.Approve(null), now) ?: throw CoreException(CoreException.Code.Internal, "no reply")
                base.update { it?.copy(busy = false, phase = TrustPhase.Signed, handoff = TrustHandoff(HandoffKind.Extension, s.view.signedSummary, out)) }
            } catch (e: CoreException) {
                base.update { it?.copy(busy = false, error = e.shown()) }
            }
        }
    }

    fun copy() {
        val h = base.value?.handoff ?: return
        platform.clipboard.copySensitive("SSHub signed answer", h.outgoing.text)
    }
}

/** Hand-off screens (`trust-signed`, `trust-revoked`): shows what the holder carries. */
class TrustHandoffViewModel(private val platform: Platform, holder: TrustHandoffHolder) : ViewModel() {
    private val handoff = holder.current
    val state: StateFlow<TrustHandoffState> = platform.clipboard.clearsInSeconds
        .map { TrustHandoffState(handoff, it) }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), TrustHandoffState(handoff))

    fun copy() { handoff?.let { platform.clipboard.copySensitive("SSHub signed answer", it.outgoing.text) } }
}

class TrustListViewModel(
    private val core: TrustApi,
    private val platform: Platform,
    private val handoff: TrustHandoffHolder,
) : ViewModel() {
    private val _state = MutableStateFlow(TrustListState(emptyList()))
    val state: StateFlow<TrustListState> = _state

    init { reload() }

    private fun reload() {
        viewModelScope.launch { _state.update { it.copy(certs = core.issuedCerts(platform.clock.nowSeconds())) } }
    }

    fun toggle(certId: String) = _state.update { it.copy(expanded = if (certId in it.expanded) it.expanded - certId else it.expanded + certId) }
    fun askRevoke(certId: String) = _state.update { it.copy(confirming = certId, error = null) }
    fun keep() = _state.update { it.copy(confirming = null) }

    fun revoke(onRevoked: () -> Unit) {
        val cert = _state.value.certs.firstOrNull { it.certId == _state.value.confirming } ?: return
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            try {
                val out = core.revokeCert(cert.certId, platform.clock.nowSeconds())
                handoff.current = TrustHandoff(
                    HandoffKind.Revocation,
                    "${cert.deviceLabel}, ${cert.scope}, was to end ${isoDate(cert.endsAt)}",
                    out,
                )
                _state.update { it.copy(busy = false, confirming = null) }
                reload()
                onRevoked()
            } catch (e: CoreException) {
                _state.update { it.copy(busy = false, confirming = null, error = e.shown()) }
            }
        }
    }
}

class ShortCodesViewModel(private val core: TrustApi, private val platform: Platform) : ViewModel() {
    private val _state = MutableStateFlow(ShortCodesState(platform.clock.nowSeconds(), emptyList()))
    val state: StateFlow<ShortCodesState> = _state

    init { reload() }

    private fun reload() {
        viewModelScope.launch {
            val now = platform.clock.nowSeconds()
            _state.update { it.copy(now = now, codes = core.shortCodes(now)) }
        }
    }

    fun askDead(host: String) = _state.update { it.copy(confirming = host, error = null) }
    fun keep() = _state.update { it.copy(confirming = null) }

    fun declareDead() {
        val host = _state.value.confirming ?: return
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            try {
                core.declareShortCodeDead(host)
                _state.update { it.copy(busy = false, confirming = null) }
                reload()
            } catch (e: CoreException) {
                _state.update { it.copy(busy = false, confirming = null, error = e.shown()) }
            }
        }
    }
}

/** Takes the short code once in `init`; the view model (not saved state) is the only place it lives. */
class CodeIssuedViewModel(core: TrustApi, platform: Platform) : ViewModel() {
    private val _state = MutableStateFlow(CodeIssuedState(null, platform.clock.nowSeconds(), loaded = false))
    val state: StateFlow<CodeIssuedState> = _state

    init {
        viewModelScope.launch {
            val now = platform.clock.nowSeconds()
            _state.value = CodeIssuedState(core.takeIssuedShortCode(now), now)
        }
    }
}
