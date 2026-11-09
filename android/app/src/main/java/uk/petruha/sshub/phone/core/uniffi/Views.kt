package uk.petruha.sshub.phone.core.uniffi

import uk.petruha.sshub.phone.core.ChainAdopted
import uk.petruha.sshub.phone.core.ChainCheck
import uk.petruha.sshub.phone.core.ChannelKind
import uk.petruha.sshub.phone.core.CertExtensionInfo
import uk.petruha.sshub.phone.core.CertState
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.EnrollRefusal
import uk.petruha.sshub.phone.core.EnrollStage
import uk.petruha.sshub.phone.core.IssuedCert
import uk.petruha.sshub.phone.core.IssuedShortCode
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.OpenedChain
import uk.petruha.sshub.phone.core.OpenedEnroll
import uk.petruha.sshub.phone.core.OpenedEnrolled
import uk.petruha.sshub.phone.core.OpenedRed
import uk.petruha.sshub.phone.core.OpenedRefused
import uk.petruha.sshub.phone.core.OpenedRelease
import uk.petruha.sshub.phone.core.OpenedTrustExtend
import uk.petruha.sshub.phone.core.OpenedPhoneHandover
import uk.petruha.sshub.phone.core.OpenedTrustRenew
import uk.petruha.sshub.phone.core.PhoneHandoverView
import uk.petruha.sshub.phone.core.OpenedTrustRequest
import uk.petruha.sshub.phone.core.Outgoing
import uk.petruha.sshub.phone.core.ProofState
import uk.petruha.sshub.phone.core.RecoveryNoticeOpened
import uk.petruha.sshub.phone.core.RecoveryNoticeView
import uk.petruha.sshub.phone.core.RedField
import uk.petruha.sshub.phone.core.RefusalKind
import uk.petruha.sshub.phone.core.RedView
import uk.petruha.sshub.phone.core.RefusalReason
import uk.petruha.sshub.phone.core.ReleaseChannel
import uk.petruha.sshub.phone.core.ReleaseCheck
import uk.petruha.sshub.phone.core.ReleaseView
import uk.petruha.sshub.phone.core.ReshareContributor
import uk.petruha.sshub.phone.core.ReshareHolder
import uk.petruha.sshub.phone.core.ReshareView
import uk.petruha.sshub.phone.core.ShortCodeRow
import uk.petruha.sshub.phone.core.ShortCodeState
import uk.petruha.sshub.phone.core.TrustDevice
import uk.petruha.sshub.phone.core.TrustEpochView
import uk.petruha.sshub.phone.core.TrustExtendView
import uk.petruha.sshub.phone.core.TrustRenewView
import uk.petruha.sshub.phone.core.TrustFieldView
import uk.petruha.sshub.phone.core.TrustRefusal
import uk.petruha.sshub.phone.core.TrustRequestView
import uk.petruha.sshub.phone.ui.components.QrMatrix
import java.time.Instant
import uniffi.sshub_phone_core.CertExtendView as RsCertExtendView
import uniffi.sshub_phone_core.CertRenewView as RsCertRenewView
import uniffi.sshub_phone_core.PhoneHandoverView as RsPhoneHandoverView
import uniffi.sshub_phone_core.ChainAdopted as RsChainAdopted
import uniffi.sshub_phone_core.CertRequestView as RsCertRequestView
import uniffi.sshub_phone_core.CertStatus as RsCertStatus
import uniffi.sshub_phone_core.Check as RsCheck
import uniffi.sshub_phone_core.EnrollStep as RsEnrollStep
import uniffi.sshub_phone_core.EnrollView as RsEnrollView
import uniffi.sshub_phone_core.EnrolledView as RsEnrolledView
import uniffi.sshub_phone_core.IssuedCertView as RsIssuedCertView
import uniffi.sshub_phone_core.IssuedCode as RsIssuedCode
import uniffi.sshub_phone_core.ManualEntry as RsManualEntry
import uniffi.sshub_phone_core.Opened as RsOpened
import uniffi.sshub_phone_core.Outgoing as RsOutgoing
import uniffi.sshub_phone_core.QrFrame as RsQrFrame
import uniffi.sshub_phone_core.RedNoticeView as RsRedNoticeView
import uniffi.sshub_phone_core.ReenrollNoticeView as RsReenrollNoticeView
import uniffi.sshub_phone_core.RefusalKind as RsRefusalKind
import uniffi.sshub_phone_core.RefusalView as RsRefusalView
import uniffi.sshub_phone_core.ReshareView as RsReshareView
import uniffi.sshub_phone_core.Row as RsRow
import uniffi.sshub_phone_core.ShareRequestView as RsShareRequestView
import uniffi.sshub_phone_core.ShortCodeEntry as RsShortCodeEntry
import uniffi.sshub_phone_core.ShortCodeOffer as RsShortCodeOffer

/*
 * Pure mapping from the uniffi records of the Rust core to the Kotlin models (docs/host-sync-android.md section 2.3).
 * Strings the core produced are passed through untouched. A field the core does not provide is null or empty here, never invented.
 * The only text written in Kotlin is a label for a Kotlin-side concept (channel names, the three proof names of a Red operation)
 * and the one-line summaries marked below.
 */

/** Lifetime of a share challenge (`CHALLENGE_EXPIRY_SECS` of sshub-sync-proto): the core gives `expires_at` only. */
internal const val CHALLENGE_SECS = 120L

/** The core renders a QR animation only up to this many bytes (`MAX_QR_DOC_BYTES`). */
internal const val QR_MAX_DOC_BYTES = 12_000

/** Words of a four word code or a 24 word phrase: the core joins them with `-` or a space. */
internal fun words(text: String?): List<String> = text.orEmpty().split('-', ' ', '\n', '\t').filter { it.isNotBlank() }

/** `who()` of the core is `name [id]` ("(no name known) [id]" for a stranger). */
internal fun whoName(who: String): String = who.substringBeforeLast('[').trim()

internal fun whoId(who: String): String? = who.substringAfterLast('[', "").substringBefore(']').takeIf { it.isNotEmpty() }

internal fun List<RsRow>.value(label: String): String? = firstOrNull { it.label == label }?.value

/** UTC stamp in the form of the core's own `iso()` (`2026-11-25T10:00:00Z`). */
internal fun utc(seconds: Long): String = runCatching { Instant.ofEpochSecond(seconds).toString() }.getOrDefault(seconds.toString())

// ------------------------------------------------------------------------------------------ outgoing

internal fun qrMatrix(f: RsQrFrame): QrMatrix {
    val n = f.size.toInt()
    val m = f.modules
    return QrMatrix(n, BooleanArray(n * n) { m[it].toInt() == 1 })
}

internal fun mapOutgoing(o: RsOutgoing, audioFast: ShortArray? = null, audioSlow: ShortArray? = null) = Outgoing(
    frame = o.frame,
    text = o.text,
    qr = o.qr.map(::qrMatrix),
    audioFast = audioFast,
    audioSlow = audioSlow,
)

internal fun mapShortCode(c: RsIssuedCode) = IssuedShortCode(c.code, c.validUntil, c.weak)

// ------------------------------------------------------------------------------------------ opened

/** A check of the core. `warn` + `typeGate`: the one rule that stops blocking Approve once the requester id was typed (D13). */
internal fun RsCheck.asCheck(typeGate: Boolean = false) =
    ReleaseCheck(text, ok = ok && !(warn && typeGate), needsTypedId = warn && typeGate)

/** The core's own class of an enrollment refusal (`RefusalView.kind`); the app picks its words by it, never by the sentence. */
internal fun enrollRefusalKind(k: RsRefusalKind): RefusalKind = when (k) {
    RsRefusalKind.MISMATCH -> RefusalKind.Mismatch
    RsRefusalKind.DECLINED -> RefusalKind.Declined
    RsRefusalKind.BAD_SIGNATURE -> RefusalKind.BadSignature
    RsRefusalKind.UNUSABLE_KEYS -> RefusalKind.UnusableKeys
    RsRefusalKind.WRONG_ORDER -> RefusalKind.WrongOrder
    RsRefusalKind.STRAY_FRAME -> RefusalKind.StrayFrame
    RsRefusalKind.OTHER -> RefusalKind.Other
}

internal fun mapRefused(v: RsRefusalView, enrollFrame: Boolean): Opened =
    if (enrollFrame) EnrollRefusal(v.reason, v.compared.getOrNull(0)?.value, v.compared.getOrNull(1)?.value, enrollRefusalKind(v.kind))
    else OpenedRefused(RefusalReason("refused", "refused", v.reason, "", emptyList(), false), v.checks.map { it.asCheck() })

/** QR and one sound choice. There is no clipboard channel: the core renders none, "Copy as text" is always offered. */
private val channels = listOf(
    ReleaseChannel(ChannelKind.Qr, "QR code"),
    ReleaseChannel(ChannelKind.SoundFast, "Sound (fast)"),
    ReleaseChannel(ChannelKind.SoundSlow, "Sound (slow)"),
)

internal fun mapRelease(v: RsShareRequestView): OpenedRelease {
    val must = v.typeToApprove
    return OpenedRelease(
        ReleaseView(
            token = v.token.toLong(),
            requesterName = v.requesterName?.takeIf { it.isNotBlank() },
            requesterId = v.requester,
            requesterKey = v.requesterKey,
            requesterKnown = v.requesterKnown,
            shareKind = v.shareKind,
            shareEpoch = v.shareEpoch.toLong(),
            shareIndex = v.shareIndex.toInt(),
            // The challenge's own lifetime: the core gives `expires_at` only.
            signedAt = v.expiresAt - CHALLENGE_SECS,
            expiresAt = v.expiresAt,
            plain = v.plain,
            headPending = v.headPending,
            words = words(v.bindingWords),
            checks = v.checks.map { it.asCheck(typeGate = must != null) },
            channels = channels,
            mustType = must,
            shortCodeOffer = when (v.shortCode) {
                is RsShortCodeOffer.Available -> "Also issue a short code (repeat access for 7 days)"
                is RsShortCodeOffer.LiveUntil -> null
            },
            hostUid = v.hostUid,
            bindingHash = v.bindingHash,
            opRows = v.op.rows.map { RedField(it.label, null, it.value, danger = it.danger) },
        ),
    )
}

internal fun mapRed(v: RsRedNoticeView): OpenedRed {
    val checks = buildList {
        add(ReleaseCheck(v.signatureNote, ok = true))
        v.blocked?.let { add(ReleaseCheck(it, ok = false)) }
    }
    return OpenedRed(
        RedView(
            token = v.token.toLong(),
            kind = v.op.kind,
            title = v.op.headline,
            proposerName = v.proposerName?.takeIf { it.isNotBlank() },
            proposerId = v.proposer,
            lamport = v.lamport.toLong(),
            seq = v.seq.toLong(),
            proposedAt = v.proposedAt,
            lapsesAt = v.lapsesAt,
            hash = v.opHash,
            fields = v.op.rows.map { RedField(it.label, null, it.value, danger = it.danger) },
            note = v.banner,
            phoneProof = if (v.blocked != null) ProofState.Refused else ProofState.YourTurn,
            checks = checks,
            mustType = v.typeToApprove,
            plain = v.op.plain,
        ),
    )
}

internal fun mapNotice(v: RsReenrollNoticeView, phoneName: String) = RecoveryNoticeOpened(
    v.token.toLong(),
    RecoveryNoticeView(
        startedBy = v.op.rows.value("Op signed by") ?: v.op.requester.orEmpty(),
        lamport = v.lamport.toLong(),
        phoneName = phoneName,
        operation = v.op.kind,
        startedAt = v.proposedAt,
        startedUtc = utc(v.proposedAt),
        endsAt = v.endsAt,
        endsUtc = utc(v.endsAt),
        hash = v.opHash,
        signatureNote = v.signatureNote,
        cancellable = true,
        opRows = v.op.rows.map { RedField(it.label, null, it.value, danger = it.danger) },
    ),
)

private const val EPOCH_ROW = "Phone epochs it holds for"
private const val REFUSED_TITLE = "Request refused"

private fun problems(list: List<String>) = list.takeIf { it.isNotEmpty() }?.let { TrustRefusal(-1, REFUSED_TITLE, it.joinToString("\n")) }

internal fun mapTrustRequest(v: RsCertRequestView): OpenedTrustRequest {
    val who = v.rows.value("Trusted device").orEmpty()
    val epoch = v.rows.firstOrNull { it.label == EPOCH_ROW }
    return OpenedTrustRequest(
        TrustRequestView(
            token = v.token.toLong(),
            device = TrustDevice(
                name = if (v.delegateKnown) whoName(who) else null,
                id = v.typeToApprove ?: whoId(who).orEmpty(),
                met = v.delegateKnown,
            ),
            headline = v.headline,
            fields = v.rows.filter { it !== epoch }.map { TrustFieldView(it.label, it.value, danger = it.danger) },
            epoch = epoch?.let { TrustEpochView(it.label, it.value) },
            signedSummary = v.headline,
            refusal = problems(v.problems),
            plain = v.plain,
        ),
    )
}

/** `N` and `E` of the extension rows as one allowance pair, or null when a row is not a plain count. */
private fun allowance(budget: String?, extra: String?): Pair<String, String>? {
    val n = budget?.trim()?.toLongOrNull() ?: return null
    val e = extra?.trim()?.toLongOrNull() ?: return null
    return "$n new hosts per day" to "a total of ${n + e} new hosts in the rolling day, as the terminal counts them"
}

internal fun mapTrustExtend(v: RsCertExtendView): OpenedTrustExtend {
    val extra = v.rows.lastOrNull()
    // Old and new allowance when both rows are plain counts; else the extra stays a plain row (diffFrom empty), never "5 to 3".
    val pair = allowance(v.rows.value("Daily budget now"), extra?.value)
    return OpenedTrustExtend(
        TrustExtendView(
            token = v.token.toLong(),
            headline = v.headline,
            plain = v.plain,
            fields = v.rows.map { TrustFieldView(it.label, it.value, danger = it.danger) },
            diffKey = if (pair != null) "Allowance for the next 24 hours" else extra?.label.orEmpty(),
            diffFrom = pair?.first.orEmpty(),
            diffTo = pair?.second ?: extra?.value.orEmpty(),
            diffDanger = extra?.danger ?: true,
            note = v.note,
            signedSummary = v.headline,
            refusal = problems(v.problems),
            again = null,
        ),
    )
}

internal fun mapTrustRenew(v: RsCertRenewView): OpenedTrustRenew {
    val r = v.renew
    return OpenedTrustRenew(
        TrustRenewView(
            token = v.token.toLong(), headline = v.headline, plain = v.plain,
            delegate = r.delegate, scope = r.scope, termSecs = r.termSecs, budget = r.budget.toInt(), renewalsLeft = r.renewalsLeft.toInt(),
            newEnd = r.newEnd, opened = r.opened, openedTotal = r.openedTotal.toInt(), evidenceAgeSecs = r.evidenceAgeSecs,
            refusal = r.refusal?.let { TrustRefusal(-1, REFUSED_TITLE, it) },
            note = v.note, signedSummary = v.headline,
        ),
    )
}

internal fun mapHandover(v: RsPhoneHandoverView) = OpenedPhoneHandover(
    PhoneHandoverView(
        token = v.token.toLong(), name = v.handover.name, words = v.handover.words, certsCarried = v.handover.certsCarried,
        opHash = v.handover.opHash, note = v.note,
    ),
)

internal fun mapReshare(v: RsReshareView): ReshareView {
    val verified = v.contributors.count { it.verified }
    val note = "$verified of ${v.contributors.size} contributors have a verified signature." +
        if (v.waiting > 0u) " ${v.waiting} credentials still wait for a second contributor." else ""
    return ReshareView(
        epoch = v.epoch.toLong(),
        holderNumber = v.myIndex.toInt(),
        credentialsReady = v.credentialsReady.toInt(),
        credentialsTotal = (v.credentialsReady + v.waiting).toInt(),
        contributors = v.contributors.map { ReshareContributor(it.name, it.deviceId, it.verified, it.known, it.hasSession) },
        note = note,
        adoptable = true,
        holdersAfter = v.holdersAfter.map { ReshareHolder(it.deviceId, it.name, it.isThisPhone) },
        storageKeyNew = v.storageKeyNew,
    )
}

internal fun mapChain(v: RsChainAdopted) = OpenedChain(
    ChainAdopted(v.token.toLong(), v.agreed, v.headSeq.toLong(), v.newestAt, v.ops.toInt(), v.previousSeq?.toLong(), v.summary, v.note, v.sender,
        v.singleSource, v.terminalsCount?.toInt(), v.terminalsNewestAt),
    emptyList<ChainCheck>(),
)

internal fun mapEnroll(v: RsEnrollView) = OpenedEnroll(
    token = v.token.toLong(),
    stage = when (v.step) {
        RsEnrollStep.COMMIT -> EnrollStage.Commit
        RsEnrollStep.REVEAL -> EnrollStage.Reveal
    },
    sasWords = words(v.sasWords),
    terminalId = v.terminalId.orEmpty(),
    terminalName = v.terminalName.orEmpty(),
    outgoing = v.outgoing?.let { mapOutgoing(it) },
)

internal fun mapEnrolled(v: RsEnrolledView, phoneId: String) = OpenedEnrolled(v.terminalId, v.terminalName, v.phoneName, phoneId)

/** The core's token of a view that can be decided or looked up again; null for values nothing refers to. */
internal fun tokenOf(o: RsOpened): Long? = when (o) {
    is RsOpened.ShareRequest -> o.view.token.toLong()
    is RsOpened.RedNotice -> o.view.token.toLong()
    is RsOpened.ReenrollNotice -> o.view.token.toLong()
    is RsOpened.CertRequest -> o.view.token.toLong()
    is RsOpened.CertExtend -> o.view.token.toLong()
    is RsOpened.CertRenew -> o.view.token.toLong()
    is RsOpened.PhoneHandover -> o.view.token.toLong()
    is RsOpened.Reshare -> o.view.token.toLong()
    is RsOpened.Chain -> o.view.token.toLong()
    is RsOpened.Refused, is RsOpened.Enroll, is RsOpened.Enrolled -> null
}

// ------------------------------------------------------------------------------------------ lists

internal fun mapShortCodeRow(e: RsShortCodeEntry, now: Long) = ShortCodeRow(
    host = e.hostUid,
    state = when {
        e.live -> ShortCodeState.Live
        e.expiresAt <= now -> ShortCodeState.Expired
        else -> ShortCodeState.DeclaredDead
    },
    issuedAt = e.issuedAt,
    expiresAt = e.expiresAt,
)

/** Every field comes from the core's structured record; `lamport` is not in it. */
internal fun mapCert(v: RsIssuedCertView) = IssuedCert(
    certId = v.certId,
    deviceName = v.deviceName?.takeIf { it.isNotBlank() },
    deviceId = v.deviceId,
    state = when (v.status) {
        RsCertStatus.ACTIVE -> CertState.Active
        RsCertStatus.EXPIRED -> CertState.Expired
        RsCertStatus.REVOKED -> CertState.Revoked
    },
    hosts = v.hosts,
    group = v.group,
    issuedAt = v.issuedAt,
    endsAt = v.effectiveExpiry,
    renewals = v.renewals.toInt(),
    epochLo = v.epochLo.toLong(),
    epochHi = v.epochHi.toLong(),
    termText = v.termText,
    budgetPerDay = v.maxNewHostsPerDay.toInt(),
    extensions = v.extensions.map { CertExtensionInfo(it.at, it.extraHosts.toInt()) },
    lamport = null,
)

internal fun manualWords(m: RsManualEntry) = words(m.words)

internal fun badInput(text: String) = CoreException(CoreException.Code.Malformed, text)
