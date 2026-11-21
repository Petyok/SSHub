package io.github.petyok.sshub.core.uniffi

import android.content.Context
import android.os.Looper
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import io.github.petyok.sshub.BuildConfig
import java.util.concurrent.Executors
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import io.github.petyok.sshub.core.CodeIssuedView
import io.github.petyok.sshub.core.CoreException
import io.github.petyok.sshub.core.EnrollDecision
import io.github.petyok.sshub.core.EnrollTextKind
import io.github.petyok.sshub.core.ExportedOps
import io.github.petyok.sshub.core.HomePhoneView
import io.github.petyok.sshub.core.IssuedCert
import io.github.petyok.sshub.core.Opened
import io.github.petyok.sshub.core.OpenedRed
import io.github.petyok.sshub.core.OpenedRelease
import io.github.petyok.sshub.core.OpenedTrustExtend
import io.github.petyok.sshub.core.OpenedPhoneHandover
import io.github.petyok.sshub.core.OpenedTrustRenew
import io.github.petyok.sshub.core.OpenedTrustRequest
import io.github.petyok.sshub.core.Outgoing
import io.github.petyok.sshub.core.PendingItem
import io.github.petyok.sshub.core.PendingKind
import io.github.petyok.sshub.core.PhoneCoreApi
import io.github.petyok.sshub.core.ExportOpRow
import io.github.petyok.sshub.core.PhoneStatus
import io.github.petyok.sshub.core.RecoveryNoticeOpened
import io.github.petyok.sshub.core.RecoveryNoticeSummary
import io.github.petyok.sshub.core.RecoveryNoticeView
import io.github.petyok.sshub.core.ReleaseDecision
import io.github.petyok.sshub.core.ReleaseOutcome
import io.github.petyok.sshub.core.ReshareOpened
import io.github.petyok.sshub.core.ReshareOutcome
import io.github.petyok.sshub.core.ReshareStage
import io.github.petyok.sshub.core.ReshareView
import io.github.petyok.sshub.core.RingDevice
import io.github.petyok.sshub.core.RingView
import io.github.petyok.sshub.core.Routes
import io.github.petyok.sshub.core.ScanEvent
import io.github.petyok.sshub.core.ShareWords
import io.github.petyok.sshub.core.KeyVault
import io.github.petyok.sshub.core.VaultResult
import io.github.petyok.sshub.core.nameAndId
import io.github.petyok.sshub.ui.screens.release.hashPrefix
import io.github.petyok.sshub.ui.screens.release.vaultFailure
import io.github.petyok.sshub.core.ShortCodeRow
import io.github.petyok.sshub.core.TrustDecision
import io.github.petyok.sshub.core.TrustExtendView
import io.github.petyok.sshub.core.PhoneHandoverView
import io.github.petyok.sshub.core.TrustRenewView
import io.github.petyok.sshub.core.TrustRequestView
import io.github.petyok.sshub.core.WordState
import io.github.petyok.sshub.core.WrapKeyState
import io.github.petyok.sshub.core.ChainTaken
import io.github.petyok.sshub.platform.SealedStateStore
import java.io.File
import java.io.IOException
import uniffi.sshub_phone_core.AudioProfile as RsAudioProfile
import uniffi.sshub_phone_core.BleSession as RsBleSession
import uniffi.sshub_phone_core.CoreException as RsCoreException
import uniffi.sshub_phone_core.Decision as RsDecision
import uniffi.sshub_phone_core.InternalException as RsInternalException
import uniffi.sshub_phone_core.Opened as RsOpened
import uniffi.sshub_phone_core.Outgoing as RsOutgoing
import uniffi.sshub_phone_core.PhoneCore as RsPhoneCore
import uniffi.sshub_phone_core.ReleaseOptions as RsReleaseOptions
import uniffi.sshub_phone_core.ReshareView as RsReshareView
import uniffi.sshub_phone_core.ScanEvent as RsScanEvent
import uniffi.sshub_phone_core.ShortCodeOffer as RsShortCodeOffer
import uniffi.sshub_phone_core.UnlockInfo as RsUnlockInfo
import uniffi.sshub_phone_core.isWord
import uniffi.sshub_phone_core.createSecret as rsCreateSecret

/**
 * The production `PhoneCoreApi`: a thin adapter over the uniffi `PhoneCore` of crates/sshub-phone-core (docs/host-sync-android.md 2.3).
 *
 * One `PhoneCore` per process. The sealed state is read at construction and written (atomically, by [SealedStateStore]) after every
 * call that changes it. `openFrame` fills a small token cache with the Kotlin view of what it opened; `pendingView`, `trustRequest`,
 * `reshareCandidate`, `recoveryNotice` and the Home list are answered from it. The cache follows the core's own pending queue
 * (`pendingTokens`): an entry stays exactly as long as the core still lists its token.
 */
class UniffiPhoneCore(private val store: SealedStateStore) : PhoneCoreApi {
    constructor(context: Context) : this(SealedStateStore(File(context.applicationContext.noBackupFilesDir, "core")))

    /** Replaced (never sealed) after a failed save, see `persist`. */
    @Volatile private var core = RsPhoneCore(store.read())

    /**
     * Every call into the Rust core runs on this ONE thread, in order. The core holds its Mutex for a whole short-code release (about
     * 0.7 s on a server CPU, seconds on a phone), so a call from Main would freeze the UI: debug builds throw when `guard` sees Main.
     * Reads that need the core are `suspend`; `lock` and `resetScan` are queued here and return at once.
     */
    internal val coreDispatcher: CoroutineDispatcher = Executors.newSingleThreadExecutor { r -> Thread(r, "sshub-core").apply { isDaemon = true } }.asCoroutineDispatcher()
    private val coreScope = CoroutineScope(SupervisorJob() + coreDispatcher)

    /** D17: true while the app signing fingerprint was reported as different. Every operation that opens a frame or signs refuses then. */
    @Volatile var blocked: (() -> Boolean)? = null

    /**
     * The per-signature system lock. The shell sets it to the platform vault; every mutating call below asks it once, BEFORE the core
     * is touched, so no view model can skip it. Null refuses: the adapter fails closed until the shell has installed it.
     */
    @Volatile var vault: KeyVault? = null
    private suspend fun confirm(title: String, subtitle: String) {
        val v = vault ?: throw CoreException(CoreException.Code.Refused, "The system lock is not attached. Nothing was signed.")
        when (val r = v.unlock(title, subtitle)) {
            is VaultResult.Secret -> r.bytes.fill(0)
            else -> throw CoreException(CoreException.Code.Refused, vaultFailure(r) ?: "The system lock gave no answer. Nothing was signed.")
        }
    }

    /** The unreadable-state stop: set by `unlock`, cleared by `confirmStateLost`. While set nothing is written and nothing opens. */
    @Volatile private var lostPending = false

    override fun stateLost(): Boolean = lostPending

    override suspend fun confirmStateLost() = withContext(coreDispatcher) {
        lostPending = false
        persist()
    }

    private fun subtitleOf(o: Opened?): String = when (o) {
        is OpenedRed -> hashPrefix(o.view.hash)
        is OpenedRelease -> nameAndId(o.view.requesterName, o.view.requesterId)
        is OpenedTrustRequest -> o.view.device.id
        is OpenedTrustExtend -> o.view.headline.take(40)
        is OpenedTrustRenew -> o.view.delegate.take(40)
        is OpenedPhoneHandover -> o.view.name.take(40)
        is ReshareOpened -> "epoch ${o.view.epoch}"
        is RecoveryNoticeOpened -> o.view.phoneName
        else -> ""
    }

    /** The last Approve outcomes (newest 8 tokens, without the short code and the by-hand words), until the phone locks. */
    private val outcomes = object : LinkedHashMap<Long, ReleaseOutcome>() {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<Long, ReleaseOutcome>) = size > MAX_OUTCOMES
    }
    override fun lastOutcome(token: Long): ReleaseOutcome? = synchronized(lock) { outcomes[token] }
    internal fun keptOutcomeCount(): Int = synchronized(lock) { outcomes.size }

    private fun requireOpen() {
        if (lostPending) throw CoreException(CoreException.Code.Refused, "The saved state could not be read. Read the stop screen first.")
        val b = blocked ?: throw CoreException(CoreException.Code.Refused, "The app is not ready.")
        if (b()) throw CoreException(CoreException.Code.Refused, "Closed: this app's signing fingerprint differs from the one the terminal shows. Do not use it.")
    }
    private val persistLock = Any()

    /** Guards everything below. */
    private val lock = Any()

    private class Entry(val opened: Opened, val raw: RsOpened)

    private val cache = LinkedHashMap<Long, Entry>()
    private var lastReshare: RsReshareView? = null
    private var rotated = false
    /** The reshare adopted and published by the one `decide(Approve)`; null until then. */
    private var publishedView: ReshareView? = null
    private var published = false
    private var issued: CodeIssuedView? = null

    // ---------------------------------------------------------------------------------------- plumbing

    private inline fun <T> guard(block: () -> T): T = try {
        if (BuildConfig.DEBUG) check(Looper.myLooper() != Looper.getMainLooper()) { "a core call on the main thread" }
        block()
    } catch (e: RsCoreException) {
        throw coreException(e)
    } catch (e: RsInternalException) {
        throw CoreException(CoreException.Code.Internal, e.message ?: "internal error")
    }

    private fun coreException(e: RsCoreException): CoreException = when (e) {
        is RsCoreException.Locked -> CoreException(CoreException.Code.Refused, "This phone is locked.")
        is RsCoreException.BadInput -> CoreException(CoreException.Code.Malformed, e.reason)
        is RsCoreException.Refused -> CoreException(if (headPending()) CoreException.Code.HeadPending else CoreException.Code.Refused, e.reason)
        is RsCoreException.Internal -> CoreException(CoreException.Code.Internal, e.reason)
    }

    private fun headPending(): Boolean = try { core.status(0).headPending } catch (_: Exception) { false }

    /** A write that fails must not leave a signed frame in the user's hands for a state the phone did not keep: lock everything. */
    private fun persist() = synchronized(persistLock) {
        val blob = guard { core.sealedState() }
        try {
            store.write(blob)
        } catch (e: IOException) {
            reset()
            // Not `core.lock()`: that seals the live state, which already holds the op this write failed to keep, and the next unlock would
            // bring it back. Drop the core without sealing and start over from the file, which still has the last state that was saved.
            val old = core
            core = RsPhoneCore(store.read())
            runCatching { old.destroy() }
            vault?.lock()
            throw CoreException(CoreException.Code.Internal, "The phone state could not be saved; the request was discarded.")
        }
    }

    private fun reset() = synchronized(lock) {
        cache.clear()
        lastReshare = null
        rotated = false
        publishedView = null
        published = false
        issued = null
        outcomes.clear()
    }

    private fun entry(token: Long): Entry? = synchronized(lock) { cache[token] }

    /** The cache follows the core's own pending queue: whatever the core no longer lists is dropped, the rest stays. */
    private fun syncCache() {
        val live = try { guard { core.pendingTokens() }.map { it.toLong() }.toSet() } catch (_: Exception) { emptySet() }
        synchronized(lock) { cache.keys.retainAll(live) }
    }

    private fun rsDecide(token: Long, decision: RsDecision, now: Long): RsOutgoing =
        try {
            guard { core.decide(token.toULong(), decision, now) }
        } finally {
            syncCache()
        }

    private fun selfName(i: RsUnlockInfo): String = i.deviceName.orEmpty()

    private fun toStatus(i: RsUnlockInfo) = PhoneStatus(i.deviceId, selfName(i), i.enrolled, i.headPending, i.ringEpoch.toLong(), i.stateLost)

    /** Sound for each profile asked for, made from the share frame the core returned. */
    private fun mapWithAudio(o: RsOutgoing, fast: Boolean, slow: Boolean): Outgoing {
        val frame = o.audioFrame
        val f = if (fast && frame != null) guard { core.audioBurst(frame, RsAudioProfile.FAST) }.toShortArray() else null
        val s = if (slow && frame != null) guard { core.audioBurst(frame, RsAudioProfile.SLOW) }.toShortArray() else null
        return mapOutgoing(o, f, s)
    }

    internal fun newBleSession(mtu: Int): RsBleSession = core.bleNew(mtu.toUInt())

    // ---------------------------------------------------------------------------------------- scan

    /** Called from the camera analysis thread (never Main); it queues behind a running decide like everything else. */
    override fun scanGray(width: Int, height: Int, luma: ByteArray): ScanEvent {
        if (BuildConfig.DEBUG) check(Looper.myLooper() != Looper.getMainLooper()) { "scanGray called on the main thread" }
        return runBlocking(coreDispatcher) { scanGrayOnCore(width, height, luma) }
    }

    private fun scanGrayOnCore(width: Int, height: Int, luma: ByteArray): ScanEvent {
        val events = guard { core.scanGray(width.toUInt(), height.toUInt(), luma) }
        events.filterIsInstance<RsScanEvent.Document>().firstOrNull()?.let { return ScanEvent.Frame(it.bytes) }
        // A completed document that is not one of ours: the page ignores it, the screen says so.
        // A small plain QR (the device id code) arrives as Foreign with its bytes: hand it on as a frame, the screen decides by purpose.
        events.filterIsInstance<RsScanEvent.Foreign>().firstOrNull { it.bytes.size >= DEVICE_ID_PREFIX.size && it.bytes.copyOfRange(0, DEVICE_ID_PREFIX.size).contentEquals(DEVICE_ID_PREFIX) }?.let { return ScanEvent.Frame(it.bytes) }
        if (events.any { it is RsScanEvent.Foreign }) return ScanEvent.Foreign
        val p = events.filterIsInstance<RsScanEvent.Progress>().lastOrNull() ?: return ScanEvent.None
        return ScanEvent.Progress(p.have.toInt(), p.total.toInt())
    }

    override fun resetScan() { coreScope.launch { guard { core.scanReset() } } }

    override suspend fun frameFromText(text: String): ByteArray? = withContext(coreDispatcher) { guard { core.frameFromText(text) } }

    override suspend fun openFrame(frame: ByteArray, now: Long): Opened = withContext(coreDispatcher) {
        requireOpen()
        // "SWR1" || version u16 || kind: the core drops its pending requests only for a frame that has the magic.
        val magic = frame.size >= 7 && frame[0] == 'S'.code.toByte() && frame[1] == 'W'.code.toByte() &&
            frame[2] == 'R'.code.toByte() && frame[3] == '1'.code.toByte()
        val enrollFrame = magic && frame[6].toInt() in 6..8
        val rs = try { guard { core.openFrame(frame, now) } } finally { syncCache() }
        val me = guard { core.status(now) }
        val opened: Opened = when (rs) {
            is RsOpened.Refused -> mapRefused(rs.view, enrollFrame)
            is RsOpened.ShareRequest -> mapRelease(rs.view)
            is RsOpened.RedNotice -> mapRed(rs.view)
            is RsOpened.ReenrollNotice -> mapNotice(rs.view, selfName(me).ifBlank { "this phone" })
            is RsOpened.CertRequest -> mapTrustRequest(rs.view)
            is RsOpened.CertExtend -> mapTrustExtend(rs.view)
            is RsOpened.CertRenew -> mapTrustRenew(rs.view)
            is RsOpened.PhoneHandover -> mapHandover(rs.view)
            is RsOpened.Reshare -> ReshareOpened(rs.view.token.toLong(), mapReshare(rs.view))
            is RsOpened.Chain -> mapChain(rs.view)
            is RsOpened.Enroll -> mapEnroll(rs.view)
            is RsOpened.Enrolled -> mapEnrolled(rs.view, me.deviceId)
        }
        synchronized(lock) {
            // the core's queue decides what survives this frame (syncCache above)
            tokenOf(rs)?.let { cache[it] = Entry(opened, rs) }
            if (rs is RsOpened.Reshare) {
                // A new reshare starts a new sequence; the rotation of a finished one does not carry over.
                lastReshare = rs.view
                if (published) rotated = false
                publishedView = null
                published = false
            }
        }
        if (rs !is RsOpened.Refused) persist()
        opened
    }

    // ---------------------------------------------------------------------------------------- identity

    override suspend fun status(now: Long): PhoneStatus = withContext(coreDispatcher) { toStatus(guard { core.status(now) }) }

    override fun createSecret(): ByteArray = rsCreateSecret()

    override suspend fun restoreFromWords(words: List<String>): ByteArray = withContext(coreDispatcher) {
        val secret = guard { core.restoreFromWords(words.joinToString(" ")) }
        reset()
        secret
    }

    /** The BIP39 list lives in the core (`isWord`): backup words, manual entry and SAS words are judged by it. */
    override fun validateWord(word: String): WordState {
        val w = word.trim()
        return when {
            w.isEmpty() -> WordState.Empty
            isWord(w) -> WordState.Ok
            else -> WordState.Bad
        }
    }

    override suspend fun backupWords(): List<CharArray> = withContext(coreDispatcher) {
        guard { core.backupWords() }.split(' ').filter { it.isNotEmpty() }.map { it.toCharArray() }
    }

    override suspend fun unlock(secret: ByteArray, now: Long): PhoneStatus = withContext(coreDispatcher) {
        val info = guard { core.unlock(secret) }
        reset()
        if (info.stateLost) {
            // Keep the unreadable bytes, write nothing else until the user confirms (the stop screen).
            if (!lostPending) store.keepAsLost()
            lostPending = true
        } else if (!lostPending) {
            persist()
        }
        // lostPending survives lock and unlock: after a lock the core opens its own sealed (empty) state and no longer says stateLost,
        // but the file on disk is still the unreadable one. Only `confirmStateLost` clears the stop.
        toStatus(guard { core.status(now) })
    }

    /** The Kotlin state is dropped now; the core's own lock is queued behind whatever it is doing and runs next. */
    override fun lock() {
        reset()
        coreScope.launch { guard { core.lock() } }
    }
    override suspend fun neverSigned(typedId: String, now: Long): PhoneStatus {
        requireOpen()
        confirm("Confirm: this key never signed", typedId)
        return withContext(NonCancellable + coreDispatcher) {
            guard { core.neverSigned(typedId) }
            persist()
            toStatus(guard { core.status(now) })
        }
    }

    override suspend fun adoptChain(token: Long, now: Long, acknowledgedSingleSource: Boolean): ChainTaken {
        requireOpen()
        val head = (entry(token)?.raw as? RsOpened.Chain)?.view?.headSeq?.toLong()
        confirm("Take this chain", "seq ${head ?: 0}")
        return withContext(NonCancellable + coreDispatcher) {
            rsDecide(token, if (acknowledgedSingleSource) RsDecision.AcceptSingleSource else RsDecision.Approve(null), now)
            persist()
            ChainTaken(head, toStatus(guard { core.status(now) }))
        }
    }

    // ---------------------------------------------------------------------------------------- home

    override suspend fun ring(now: Long): RingView? = withContext(coreDispatcher) { ringOf(guard { core.status(now) }) }

    private fun ringOf(i: RsUnlockInfo): RingView? {
        if (!i.enrolled) return null
        val devices = i.ring.map { RingDevice(it.name.takeIf { n -> n.isNotBlank() }, it.deviceId, self = it.deviceId == i.deviceId) }
            .let { list ->
                if (list.any { it.self }) list
                else listOf(RingDevice(i.deviceName?.takeIf { n -> n.isNotBlank() }, i.deviceId, self = true)) + list
            }
        return RingView(epoch = i.ringEpoch.toLong(), devices = devices, headPending = i.headPending)
    }

    override suspend fun thisPhone(now: Long): HomePhoneView = withContext(coreDispatcher) {
        val i = guard { core.status(now) }
        HomePhoneView(i.deviceId, selfName(i), i.headSeq?.toLong(), i.liveShortCodes.toInt())
    }

    override fun pending(now: Long): List<PendingItem> = synchronized(lock) {
        cache.entries.sortedBy { it.key }.mapNotNull { (token, e) ->
            when (val o = e.opened) {
                is OpenedRed -> PendingItem(PendingKind.Red, o.view.title, null, Routes.Red + "/" + token)
                is OpenedRelease -> PendingItem(PendingKind.Release, nameAndId(o.view.requesterName, o.view.requesterId), null, Routes.Release + "/" + token)
                is OpenedTrustRequest -> PendingItem(PendingKind.Trust, o.view.headline, null, Routes.TrustRequest + "/" + token)
                is OpenedTrustExtend -> PendingItem(PendingKind.Extend, o.view.headline, null, Routes.TrustExtend + "/" + token)
                is OpenedTrustRenew -> PendingItem(PendingKind.Renew, o.view.headline, null, Routes.TrustRenew + "/" + token)
                is OpenedPhoneHandover -> PendingItem(PendingKind.Handover, if (o.view.admission) "Admit this phone: ${o.view.name}" else "Replace this phone with ${o.view.name}", null, Routes.Handover + "/" + token)
                is RecoveryNoticeOpened -> PendingItem(PendingKind.Recovery, o.view.startedBy, null, Routes.RecoveryNotice + "/" + token)
                is ReshareOpened -> PendingItem(PendingKind.Reshare, "epoch ${o.view.epoch}", null, Routes.Reshare + "/" + token)
                else -> null
            }
        }
    }

    // ---------------------------------------------------------------------------------------- release

    override fun pendingView(token: Long): Opened? = entry(token)?.opened?.takeIf { it is OpenedRed || it is OpenedRelease }

    override suspend fun decide(token: Long, decision: ReleaseDecision, now: Long): ReleaseOutcome {
        requireOpen()
        val e = entry(token) ?: throw badInput("This request is no longer pending.")
        if (e.raw !is RsOpened.ShareRequest && e.raw !is RsOpened.RedNotice) throw badInput("This is not a request that releases a share or signs a Red operation.")
        if (decision is ReleaseDecision.Approve) {
            confirm(if (e.raw is RsOpened.RedNotice) "Approve Red operation" else "Approve share release", subtitleOf(e.opened))
        }
        // Once the core is asked it finishes: leaving the screen must not cancel a half-done signature.
        val out = withContext(NonCancellable + coreDispatcher) {
            when (val raw = e.raw) {
                is RsOpened.ShareRequest -> decideRelease(token, raw, decision, now)
                else -> decideRed(token, decision, now)
            }
        }
        if (decision is ReleaseDecision.Approve) synchronized(lock) { outcomes[token] = ReleaseOutcome(outgoing = out.outgoing, shortCode = null, byHand = null, note = out.note) }
        return out
    }

    private fun decideRed(token: Long, decision: ReleaseDecision, now: Long): ReleaseOutcome = when (decision) {
        ReleaseDecision.Decline -> {
            rsDecide(token, RsDecision.Refuse, now)
            ReleaseOutcome(null)
        }
        is ReleaseDecision.Approve -> {
            val o = rsDecide(token, RsDecision.Approve(decision.typedId), now)
            persist()
            ReleaseOutcome(mapOutgoing(o), note = o.note)
        }
    }

    private fun decideRelease(token: Long, raw: RsOpened.ShareRequest, decision: ReleaseDecision, now: Long): ReleaseOutcome = when (decision) {
        ReleaseDecision.Decline -> {
            rsDecide(token, RsDecision.Refuse, now)
            ReleaseOutcome(null)
        }
        is ReleaseDecision.Approve -> {
            val c = decision.channels
            val options = RsReleaseOptions(
                typedId = decision.typedId,
                qr = c.qr,
                clipboard = c.clipboard,
                sound = c.soundFast || c.soundSlow,
                audioProfile = if (c.soundFast) RsAudioProfile.FAST else RsAudioProfile.SLOW,
                issueShortCode = decision.shortCode,
                byHand = decision.byHand,
            )
            val o = try {
                rsDecide(token, RsDecision.Release(options), now)
            } catch (e: CoreException) {
                // The core states the expiry as a refusal; the Release screen has a code for it.
                throw if (e.code == CoreException.Code.Refused && now >= raw.view.expiresAt) CoreException(CoreException.Code.Expired, e.message.orEmpty()) else e
            }
            persist()
            val code = o.shortCode
            if (decision.shortCode) {
                val offer = raw.view.shortCode
                synchronized(lock) {
                    issued = when {
                        code != null -> CodeIssuedView.Issued(raw.view.hostUid, code.code.replace("-", ""), now, code.validUntil)
                        offer is RsShortCodeOffer.LiveUntil -> CodeIssuedView.Live(raw.view.hostUid, offer.expiresAt)
                        else -> null
                    }
                }
            }
            ReleaseOutcome(
                outgoing = mapWithAudio(o, c.soundFast, c.soundSlow),
                shortCode = code?.let(::mapShortCode),
                byHand = if (decision.byHand) o.manual?.let { ShareWords(it.index.toInt(), manualWords(it)) } else null,
                note = o.note,
            )
        }
    }

    // ---------------------------------------------------------------------------------------- trust

    override suspend fun trustRequest(token: Long): TrustRequestView? = (entry(token)?.opened as? OpenedTrustRequest)?.view

    override suspend fun trustExtend(token: Long): TrustExtendView? = (entry(token)?.opened as? OpenedTrustExtend)?.view

    override suspend fun trustRenew(token: Long): TrustRenewView? = (entry(token)?.opened as? OpenedTrustRenew)?.view

    override suspend fun phoneHandover(token: Long): PhoneHandoverView? = (entry(token)?.opened as? OpenedPhoneHandover)?.view

    override suspend fun decideTrust(token: Long, decision: TrustDecision, now: Long): Outgoing? {
        requireOpen()
        val e = entry(token) ?: throw badInput("This request is no longer pending.")
        if (e.raw !is RsOpened.CertRequest && e.raw !is RsOpened.CertExtend && e.raw !is RsOpened.CertRenew && e.raw !is RsOpened.PhoneHandover) throw badInput("This is not a trust request.")
        if (decision is TrustDecision.Approve) confirm(
            when (e.raw) { is RsOpened.CertRequest -> "Approve trust request"; is RsOpened.CertRenew -> "Approve trust session renewal"; is RsOpened.PhoneHandover -> "Approve phone handover"; else -> "Approve certificate extension" },
            subtitleOf(e.opened),
        )
        return withContext(NonCancellable + coreDispatcher) {
            when (decision) {
                TrustDecision.Decline -> {
                    rsDecide(token, RsDecision.Refuse, now)
                    null
                }
                is TrustDecision.Approve -> {
                    val o = rsDecide(token, RsDecision.Approve(decision.typedId), now)
                    persist()
                    mapOutgoing(o)
                }
            }
        }
    }

    override suspend fun issuedCerts(now: Long): List<IssuedCert> = withContext(coreDispatcher) {
        // The core lists them in chain order, oldest first.
        guard { core.issuedCerts(now) }.reversed().map(::mapCert)
    }

    override suspend fun revokeCert(certId: String, now: Long): Outgoing {
        requireOpen()
        confirm("Revoke certificate", certId.take(16))
        return withContext(NonCancellable + coreDispatcher) {
            val o = guard { core.revokeCert(certId, now) }
            persist()
            mapOutgoing(o)
        }
    }

    override suspend fun shortCodes(now: Long): List<ShortCodeRow> = withContext(coreDispatcher) {
        guard { core.shortCodes(now) }.map { mapShortCodeRow(it, now) }
    }

    override suspend fun declareShortCodeDead(host: String) = withContext(coreDispatcher) {
        requireOpen()
        guard { core.declareShortCodeDead(host) }
        persist()
    }

    override suspend fun takeIssuedShortCode(now: Long): CodeIssuedView? = synchronized(lock) {
        val v = issued
        issued = null
        when {
            v is CodeIssuedView.Issued && v.expiresAt <= now -> null
            v is CodeIssuedView.Live && v.liveUntil <= now -> null
            else -> v
        }
    }

    // ---------------------------------------------------------------------------------------- recovery

    private fun stage(): ReshareStage = when {
        published -> ReshareStage.Published
        rotated && cache.values.any { it.opened is ReshareOpened } -> ReshareStage.Candidate
        rotated -> ReshareStage.Rotated
        else -> ReshareStage.Start
    }

    private fun wrapState(i: RsUnlockInfo): WrapKeyState = synchronized(lock) {
        val r = lastReshare
        val stage = stage()
        WrapKeyState(
            generation = i.wrapEpoch.toInt(),
            epoch = r?.epoch?.toLong() ?: 0,
            holderNumber = r?.myIndex?.toInt() ?: 0,
            credentialsReady = r?.credentialsReady?.toInt() ?: 0,
            credentialsTotal = r?.let { (it.credentialsReady + it.waiting).toInt() } ?: 0,
            stage = stage,
            reshare = if (stage == ReshareStage.Published) publishedView else null,
        )
    }

    override suspend fun wrapKeyState(now: Long): WrapKeyState = withContext(coreDispatcher) { wrapState(guard { core.status(now) }) }

    override suspend fun rotateWrapKeys(now: Long): Outgoing {
        requireOpen()
        confirm("Rotate wrap keys", "")
        return withContext(NonCancellable + coreDispatcher) {
            val o = guard { core.rotateWrapKeys(now) }
            synchronized(lock) { rotated = true }
            persist()
            mapOutgoing(o)
        }
    }

    override suspend fun reshareCandidate(token: Long, now: Long): ReshareView? = (entry(token)?.opened as? ReshareOpened)?.view

    /**
     * The core adopts and signs in one step: `decide(Approve)` on a reshare token appends the new shares to the chain and returns the
     * ops to hand to a terminal. They are mapped and returned here, never held, so no restart can lose them between two calls.
     */
    override suspend fun decideReshare(token: Long, now: Long): ReshareOutcome {
        requireOpen()
        val view = (entry(token)?.opened as? ReshareOpened)?.view ?: throw badInput("This reshare is no longer pending.")
        confirm("Adopt reshare", "epoch ${view.epoch}")
        return withContext(NonCancellable + coreDispatcher) {
            val o = rsDecide(token, RsDecision.Approve(null), now)
            persist()
            synchronized(lock) {
                publishedView = view
                published = true
            }
            ReshareOutcome(wrapState(guard { core.status(now) }), mapOutgoing(o))
        }
    }

    override suspend fun exportOps(now: Long): ExportedOps = withContext(coreDispatcher) {
        val head = guard { core.status(now) }.headSeq ?: return@withContext ExportedOps(emptyList(), 0, null)
        val text = guard { core.exportOpsText() }
        val frame = guard { core.frameFromText(text) } ?: throw CoreException(CoreException.Code.Internal, "The exported ops are not valid text.")
        val qr = if (frame.size <= QR_MAX_DOC_BYTES) guard { core.qrFrames(frame, 25u) }.map(::qrMatrix) else emptyList()
        // The rows are the core's own rendering of the same ops (the newest ones the frame carries). `total` is the newest seq.
        val rows = guard { core.exportOpsView() }.map {
            ExportOpRow(
                seq = it.rows.value("Op seq / epoch")?.substringBefore('/')?.trim()?.toLongOrNull() ?: 0,
                kind = it.kind,
                time = it.rows.value("Op time").orEmpty(),
                hash = it.hash,
                danger = it.dangerCount > 0u,
            )
        }
        ExportedOps(rows, head.toInt(), Outgoing(frame = frame, text = text, qr = qr))
    }

    override suspend fun recoveryNotices(now: Long): List<RecoveryNoticeSummary> = synchronized(lock) {
        cache.entries.sortedBy { it.key }.mapNotNull { (token, e) ->
            val v = (e.opened as? RecoveryNoticeOpened)?.view ?: return@mapNotNull null
            if (v.endsAt <= now) null else RecoveryNoticeSummary(token, v.startedBy, v.startedAt, v.endsAt)
        }
    }

    override suspend fun recoveryNotice(token: Long, now: Long): RecoveryNoticeView? = (entry(token)?.opened as? RecoveryNoticeOpened)?.view

    override suspend fun decideRecoveryCancel(token: Long, now: Long): Outgoing {
        requireOpen()
        val notice = entry(token)
        if (notice?.raw !is RsOpened.ReenrollNotice) throw badInput("This recovery notice is no longer pending.")
        confirm("Cancel this recovery", subtitleOf(notice.opened))
        return withContext(NonCancellable + coreDispatcher) {
            val o = rsDecide(token, RsDecision.Approve(null), now)
            persist()
            mapOutgoing(o)
        }
    }

    // ---------------------------------------------------------------------------------------- enrollment

    /** The name rules live in the core: the same call `enrollDecide(Name)` applies, asked before the scan (L10). */
    override fun nameProblem(name: String): String? = uniffi.sshub_phone_core.checkEnrollName(name)

    /** Header of a frame is `SWR1 || version u16 || kind`: only the kind byte is read, the core judges the frame in `openFrame`. */
    override suspend fun enrollTextKind(text: String): EnrollTextKind {
        if (text.isBlank()) return EnrollTextKind.Empty
        val frame = frameFromText(text) ?: return EnrollTextKind.NotEnrollment
        if (frame.size < 7) return EnrollTextKind.NotEnrollment
        return when (frame[6].toInt()) {
            6 -> EnrollTextKind.Step1Code
            7 -> EnrollTextKind.Step3Keys
            8 -> EnrollTextKind.Step5Confirm
            else -> EnrollTextKind.NotEnrollment
        }
    }

    override suspend fun enrollDecide(token: Long, decision: EnrollDecision, now: Long): Outgoing {
        requireOpen()
        if (decision is EnrollDecision.Sas && decision.matches) confirm("Enroll this terminal", "the four words match")
        val d = when (decision) {
            is EnrollDecision.Name -> RsDecision.EnrollName(decision.name)
            is EnrollDecision.Sas -> RsDecision.EnrollSas(decision.matches)
        }
        return withContext(NonCancellable + coreDispatcher) {
            // An enrollment decision belongs to the running flow, not to a pending token.
            val o = try { guard { core.decide(token.toULong(), d, now) } } finally { syncCache() }
            persist()
            mapOutgoing(o)
        }
    }

    private companion object {
        const val MAX_OUTCOMES = 8
    }
}

/** The plain QR a terminal shows for a device id: `sshub:dev:1:<16 hex>`. The only foreign document the app reads. */
private val DEVICE_ID_PREFIX = "sshub:dev:1:".toByteArray(Charsets.US_ASCII)
