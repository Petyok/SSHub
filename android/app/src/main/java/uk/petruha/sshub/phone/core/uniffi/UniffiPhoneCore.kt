package uk.petruha.sshub.phone.core.uniffi

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import uk.petruha.sshub.phone.core.CodeIssuedView
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.EnrollDecision
import uk.petruha.sshub.phone.core.EnrollTextKind
import uk.petruha.sshub.phone.core.ExportedOps
import uk.petruha.sshub.phone.core.HomePhoneView
import uk.petruha.sshub.phone.core.IssuedCert
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.OpenedRed
import uk.petruha.sshub.phone.core.OpenedRelease
import uk.petruha.sshub.phone.core.OpenedTrustExtend
import uk.petruha.sshub.phone.core.OpenedTrustRequest
import uk.petruha.sshub.phone.core.Outgoing
import uk.petruha.sshub.phone.core.PendingItem
import uk.petruha.sshub.phone.core.PendingKind
import uk.petruha.sshub.phone.core.PhoneCoreApi
import uk.petruha.sshub.phone.core.ExportOpRow
import uk.petruha.sshub.phone.core.PhoneStatus
import uk.petruha.sshub.phone.core.RecoveryNoticeOpened
import uk.petruha.sshub.phone.core.RecoveryNoticeSummary
import uk.petruha.sshub.phone.core.RecoveryNoticeView
import uk.petruha.sshub.phone.core.ReleaseDecision
import uk.petruha.sshub.phone.core.ReleaseOutcome
import uk.petruha.sshub.phone.core.ReshareOpened
import uk.petruha.sshub.phone.core.ReshareOutcome
import uk.petruha.sshub.phone.core.ReshareStage
import uk.petruha.sshub.phone.core.ReshareView
import uk.petruha.sshub.phone.core.RingDevice
import uk.petruha.sshub.phone.core.RingView
import uk.petruha.sshub.phone.core.Routes
import uk.petruha.sshub.phone.core.ScanEvent
import uk.petruha.sshub.phone.core.ShareWords
import uk.petruha.sshub.phone.core.KeyVault
import uk.petruha.sshub.phone.core.VaultResult
import uk.petruha.sshub.phone.core.nameAndId
import uk.petruha.sshub.phone.ui.screens.release.hashPrefix
import uk.petruha.sshub.phone.ui.screens.release.vaultFailure
import uk.petruha.sshub.phone.core.ShortCodeRow
import uk.petruha.sshub.phone.core.TrustDecision
import uk.petruha.sshub.phone.core.TrustExtendView
import uk.petruha.sshub.phone.core.TrustRequestView
import uk.petruha.sshub.phone.core.WordState
import uk.petruha.sshub.phone.core.WrapKeyState
import uk.petruha.sshub.phone.core.ChainTaken
import uk.petruha.sshub.phone.platform.SealedStateStore
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
 * `reshareCandidate`, `recoveryNotice` and the Home list are answered from it. The core drops every pending request when it opens
 * the next frame, and so does the cache.
 */
class UniffiPhoneCore(private val store: SealedStateStore) : PhoneCoreApi {
    constructor(context: Context) : this(SealedStateStore(File(context.applicationContext.noBackupFilesDir, "core")))

    private val core = RsPhoneCore(store.read())

    /** D17: true while the app signing fingerprint was reported as different. Every operation that opens a frame or signs refuses then. */
    @Volatile var blocked: () -> Boolean = { false }

    /**
     * The per-signature system lock. The shell sets it to the platform vault; every mutating call below asks it once, BEFORE the core
     * is touched, so no view model can skip it. Null only in tests of the core itself. Decline and cancel paths never ask.
     */
    @Volatile var vault: KeyVault? = null
    private suspend fun confirm(title: String, subtitle: String) {
        val v = vault ?: return
        when (val r = v.unlock(title, subtitle)) {
            is VaultResult.Secret -> r.bytes.fill(0)
            else -> throw CoreException(CoreException.Code.Refused, vaultFailure(r) ?: "The system lock gave no answer. Nothing was signed.")
        }
    }

    /** The unreadable-state stop: set by `unlock`, cleared by `confirmStateLost`. While set nothing is written and nothing opens. */
    @Volatile private var lostPending = false

    override fun stateLost(): Boolean = lostPending

    override fun confirmStateLost() {
        lostPending = false
        persist()
    }

    private fun subtitleOf(o: Opened?): String = when (o) {
        is OpenedRed -> hashPrefix(o.view.hash)
        is OpenedRelease -> nameAndId(o.view.requesterName, o.view.requesterId)
        is OpenedTrustRequest -> o.view.device.id
        is OpenedTrustExtend -> o.view.headline.take(40)
        is ReshareOpened -> "epoch ${o.view.epoch}"
        is RecoveryNoticeOpened -> o.view.phoneName
        else -> ""
    }

    /** The last Approve outcomes, by token, until the phone locks. */
    private val outcomes = HashMap<Long, ReleaseOutcome>()
    override fun lastOutcome(token: Long): ReleaseOutcome? = synchronized(lock) { outcomes[token] }

    private fun requireOpen() {
        if (lostPending) throw CoreException(CoreException.Code.Refused, "The saved state could not be read. Read the stop screen first.")
        if (blocked()) throw CoreException(CoreException.Code.Refused, "Closed: this app's signing fingerprint differs from the one the terminal shows. Do not use it.")
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

    private fun persist() = synchronized(persistLock) {
        val blob = guard { core.sealedState() }
        try {
            store.write(blob)
        } catch (e: IOException) {
            throw CoreException(CoreException.Code.Internal, "The phone state could not be written: ${e.message}")
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

    private fun drop(token: Long) = synchronized(lock) { cache.remove(token); Unit }

    /**
     * The core removes a pending request when a decision runs, except after `BadInput` (a wrong typed id): then the request stays
     * pending, the token stays valid and the cached view stays. Success, a refusal and every other error consume it, so does the cache.
     */
    private fun rsDecide(token: Long, decision: RsDecision, now: Long): RsOutgoing =
        try {
            guard { core.decide(token.toULong(), decision, now) }.also { drop(token) }
        } catch (e: CoreException) {
            if (e.code != CoreException.Code.Malformed) drop(token)
            throw e
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

    override fun scanGray(width: Int, height: Int, luma: ByteArray): ScanEvent {
        val events = core.scanGray(width.toUInt(), height.toUInt(), luma)
        events.filterIsInstance<RsScanEvent.Document>().firstOrNull()?.let { return ScanEvent.Frame(it.bytes) }
        // A completed document that is not one of ours: the page ignores it, the screen says so.
        if (events.any { it is RsScanEvent.Foreign }) return ScanEvent.Foreign
        val p = events.filterIsInstance<RsScanEvent.Progress>().lastOrNull() ?: return ScanEvent.None
        return ScanEvent.Progress(p.have.toInt(), p.total.toInt())
    }

    override fun resetScan() = core.scanReset()

    override fun frameFromText(text: String): ByteArray? = core.frameFromText(text)

    override suspend fun openFrame(frame: ByteArray, now: Long): Opened = withContext(Dispatchers.Default) {
        requireOpen()
        // "SWR1" || version u16 || kind: the core drops its pending requests only for a frame that has the magic.
        val magic = frame.size >= 7 && frame[0] == 'S'.code.toByte() && frame[1] == 'W'.code.toByte() &&
            frame[2] == 'R'.code.toByte() && frame[3] == '1'.code.toByte()
        val enrollFrame = magic && frame[6].toInt() in 6..8
        val rs = guard { core.openFrame(frame, now) }
        val me = guard { core.status(now) }
        val opened: Opened = when (rs) {
            is RsOpened.Refused -> mapRefused(rs.view, enrollFrame)
            is RsOpened.ShareRequest -> mapRelease(rs.view)
            is RsOpened.RedNotice -> mapRed(rs.view)
            is RsOpened.ReenrollNotice -> mapNotice(rs.view, selfName(me).ifBlank { "this phone" })
            is RsOpened.CertRequest -> mapTrustRequest(rs.view)
            is RsOpened.CertExtend -> mapTrustExtend(rs.view)
            is RsOpened.Reshare -> ReshareOpened(rs.view.token.toLong(), mapReshare(rs.view))
            is RsOpened.Chain -> mapChain(rs.view)
            is RsOpened.Enroll -> mapEnroll(rs.view)
            is RsOpened.Enrolled -> mapEnrolled(rs.view, me.deviceId)
        }
        synchronized(lock) {
            if (magic) cache.clear()
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

    override fun status(now: Long): PhoneStatus = toStatus(guard { core.status(now) })

    override fun createSecret(): ByteArray = rsCreateSecret()

    override fun restoreFromWords(words: List<String>): ByteArray {
        val secret = guard { core.restoreFromWords(words.joinToString(" ")) }
        reset()
        return secret
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

    override fun backupWords(): List<CharArray> =
        guard { core.backupWords() }.split(' ').filter { it.isNotEmpty() }.map { it.toCharArray() }

    override fun unlock(secret: ByteArray, now: Long): PhoneStatus {
        val info = guard { core.unlock(secret) }
        reset()
        if (info.stateLost) {
            // Keep the unreadable bytes, write nothing else until the user confirms (the stop screen).
            store.keepAsLost()
            lostPending = true
        } else {
            lostPending = false
            persist()
        }
        return toStatus(guard { core.status(now) })
    }

    override fun lock() {
        core.lock()
        reset()
    }

    override suspend fun neverSigned(typedId: String, now: Long): PhoneStatus {
        requireOpen()
        confirm("Confirm: this key never signed", typedId)
        guard { core.neverSigned(typedId) }
        persist()
        return toStatus(guard { core.status(now) })
    }

    override suspend fun adoptChain(token: Long, now: Long): ChainTaken = withContext(Dispatchers.Default) {
        requireOpen()
        val head = (entry(token)?.raw as? RsOpened.Chain)?.view?.headSeq?.toLong()
        confirm("Take this chain", "seq ${head ?: 0}")
        rsDecide(token, RsDecision.Approve(null), now)
        persist()
        ChainTaken(head, toStatus(guard { core.status(now) }))
    }

    // ---------------------------------------------------------------------------------------- home

    override fun ring(now: Long): RingView? {
        val i = guard { core.status(now) }
        if (!i.enrolled) return null
        val devices = i.ring.map { RingDevice(it.name.takeIf { n -> n.isNotBlank() }, it.deviceId, self = it.deviceId == i.deviceId) }
            .let { list ->
                if (list.any { it.self }) list
                else listOf(RingDevice(i.deviceName?.takeIf { n -> n.isNotBlank() }, i.deviceId, self = true)) + list
            }
        return RingView(epoch = i.ringEpoch.toLong(), devices = devices, headPending = i.headPending)
    }

    override fun thisPhone(now: Long): HomePhoneView {
        val i = guard { core.status(now) }
        return HomePhoneView(i.deviceId, selfName(i), i.headSeq?.toLong(), i.liveShortCodes.toInt())
    }

    override fun pending(now: Long): List<PendingItem> = synchronized(lock) {
        cache.entries.sortedBy { it.key }.mapNotNull { (token, e) ->
            when (val o = e.opened) {
                is OpenedRed -> PendingItem(PendingKind.Red, o.view.title, null, Routes.Red + "/" + token)
                is OpenedRelease -> PendingItem(PendingKind.Release, nameAndId(o.view.requesterName, o.view.requesterId), null, Routes.Release + "/" + token)
                is OpenedTrustRequest -> PendingItem(PendingKind.Trust, o.view.headline, null, Routes.TrustRequest + "/" + token)
                is OpenedTrustExtend -> PendingItem(PendingKind.Extend, o.view.headline, null, Routes.TrustExtend + "/" + token)
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
        val out = withContext(NonCancellable + Dispatchers.Default) {
            when (val raw = e.raw) {
                is RsOpened.ShareRequest -> decideRelease(token, raw, decision, now)
                else -> decideRed(token, decision, now)
            }
        }
        if (decision is ReleaseDecision.Approve) synchronized(lock) { outcomes[token] = out }
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

    override suspend fun decideTrust(token: Long, decision: TrustDecision, now: Long): Outgoing? = withContext(Dispatchers.Default) {
        requireOpen()
        val e = entry(token) ?: throw badInput("This request is no longer pending.")
        if (e.raw !is RsOpened.CertRequest && e.raw !is RsOpened.CertExtend) throw badInput("This is not a trust request.")
        if (decision is TrustDecision.Approve) confirm(if (e.raw is RsOpened.CertRequest) "Approve trust request" else "Approve certificate extension", subtitleOf(e.opened))
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

    override suspend fun issuedCerts(now: Long): List<IssuedCert> = withContext(Dispatchers.Default) {
        // The core lists them in chain order, oldest first.
        guard { core.issuedCerts(now) }.reversed().map(::mapCert)
    }

    override suspend fun revokeCert(certId: String, now: Long): Outgoing = withContext(Dispatchers.Default) {
        requireOpen()
        confirm("Revoke certificate", certId.take(16))
        val o = guard { core.revokeCert(certId, now) }
        persist()
        mapOutgoing(o)
    }

    override suspend fun shortCodes(now: Long): List<ShortCodeRow> = withContext(Dispatchers.Default) {
        guard { core.shortCodes(now) }.map { mapShortCodeRow(it, now) }
    }

    override suspend fun declareShortCodeDead(host: String) = withContext(Dispatchers.Default) {
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

    override suspend fun wrapKeyState(now: Long): WrapKeyState = withContext(Dispatchers.Default) { wrapState(guard { core.status(now) }) }

    override suspend fun rotateWrapKeys(now: Long): Outgoing = withContext(Dispatchers.Default) {
        requireOpen()
        confirm("Rotate wrap keys", "")
        val o = guard { core.rotateWrapKeys(now) }
        synchronized(lock) { rotated = true }
        persist()
        mapOutgoing(o)
    }

    override suspend fun reshareCandidate(token: Long, now: Long): ReshareView? = (entry(token)?.opened as? ReshareOpened)?.view

    /**
     * The core adopts and signs in one step: `decide(Approve)` on a reshare token appends the new shares to the chain and returns the
     * ops to hand to a terminal. They are mapped and returned here, never held, so no restart can lose them between two calls.
     */
    override suspend fun decideReshare(token: Long, now: Long): ReshareOutcome = withContext(Dispatchers.Default) {
        requireOpen()
        val view = (entry(token)?.opened as? ReshareOpened)?.view ?: throw badInput("This reshare is no longer pending.")
        confirm("Adopt reshare", "epoch ${view.epoch}")
        val o = rsDecide(token, RsDecision.Approve(null), now)
        persist()
        synchronized(lock) {
            publishedView = view
            published = true
        }
        ReshareOutcome(wrapState(guard { core.status(now) }), mapOutgoing(o))
    }

    override suspend fun exportOps(now: Long): ExportedOps = withContext(Dispatchers.Default) {
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

    override suspend fun decideRecoveryCancel(token: Long, now: Long): Outgoing = withContext(Dispatchers.Default) {
        requireOpen()
        val notice = entry(token)
        if (notice?.raw !is RsOpened.ReenrollNotice) throw badInput("This recovery notice is no longer pending.")
        confirm("Cancel this recovery", subtitleOf(notice.opened))
        val o = rsDecide(token, RsDecision.Approve(null), now)
        persist()
        mapOutgoing(o)
    }

    // ---------------------------------------------------------------------------------------- enrollment

    /** The name rules live in the core and are applied by `enrollDecide(Name)`, which throws `Malformed` with the core's sentence. */
    override fun nameProblem(name: String): String? = null

    /** Header of a frame is `SWR1 || version u16 || kind`: only the kind byte is read, the core judges the frame in `openFrame`. */
    override fun enrollTextKind(text: String): EnrollTextKind {
        if (text.isBlank()) return EnrollTextKind.Empty
        val frame = core.frameFromText(text) ?: return EnrollTextKind.NotEnrollment
        if (frame.size < 7) return EnrollTextKind.NotEnrollment
        return when (frame[6].toInt()) {
            6 -> EnrollTextKind.Step1Code
            7 -> EnrollTextKind.Step3Keys
            8 -> EnrollTextKind.Step5Confirm
            else -> EnrollTextKind.NotEnrollment
        }
    }

    override suspend fun enrollDecide(token: Long, decision: EnrollDecision, now: Long): Outgoing = withContext(Dispatchers.Default) {
        requireOpen()
        if (decision is EnrollDecision.Sas && decision.matches) confirm("Enroll this terminal", "the four words match")
        val d = when (decision) {
            is EnrollDecision.Name -> RsDecision.EnrollName(decision.name)
            is EnrollDecision.Sas -> RsDecision.EnrollSas(decision.matches)
        }
        // An enrollment decision belongs to the running flow, not to a pending token.
        val o = guard { core.decide(token.toULong(), d, now) }
        persist()
        mapOutgoing(o)
    }
}
