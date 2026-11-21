package io.github.petyok.sshub.fake

import java.time.LocalDate
import java.time.ZoneOffset
import io.github.petyok.sshub.core.CertExtensionInfo
import io.github.petyok.sshub.core.CertState
import io.github.petyok.sshub.core.CodeIssuedView
import io.github.petyok.sshub.core.CoreException
import io.github.petyok.sshub.core.IssuedCert
import io.github.petyok.sshub.core.Outgoing
import io.github.petyok.sshub.core.ShortCodeRow
import io.github.petyok.sshub.core.ShortCodeState
import io.github.petyok.sshub.core.TrustAgain
import io.github.petyok.sshub.core.TrustApi
import io.github.petyok.sshub.core.TrustCheck
import io.github.petyok.sshub.core.TrustDecision
import io.github.petyok.sshub.core.TrustDevice
import io.github.petyok.sshub.core.TrustEpochView
import io.github.petyok.sshub.core.TrustExtendView
import io.github.petyok.sshub.core.TrustFieldView
import io.github.petyok.sshub.core.PhoneHandoverView
import io.github.petyok.sshub.core.TrustRefusal
import io.github.petyok.sshub.core.TrustRenewView
import io.github.petyok.sshub.core.TrustRequestView
import io.github.petyok.sshub.ui.components.QrMatrix

/**
 * Sample data of the prototype (data.js, A-trust.js) for previews and tests.
 * Renewal tokens: 30 fresh, 31 stale, 32 delegate only, 33 over the cap, 34 no list sent. Request tokens: 1 ok, 2 never met, 10 + n refusal n (0..8). Extension tokens: 20 ok, 21 notmine, 22 revoked, 23 again.
 */
object TrustSamples {
    /** 2026-10-26 09:41 UTC. */
    const val NOW = 1_793_007_660L
    const val PHONE_EPOCH = 7L
    const val UNMET_ID = "E7A4-6B02"
    const val CODE = "K7Q29FMXC41T8PWD"
    val HOSTS = listOf("web-01", "web-02", "db-staging", "bastion", "nuc-lab")
    /** Host uids as the core gives them: 64 hex digits. The real core has no host names. */
    fun uid(n: Int) = "%02x".format(n).repeat(32)
    val UIDS = (1..5).map { uid(it) }
    val MANY_HOSTS = (1..65).map { "web-%02d".format(it) }

    fun day(y: Int, m: Int, d: Int): Long = LocalDate.of(y, m, d).atStartOfDay(ZoneOffset.UTC).toEpochSecond()

    private fun request(
        token: Long, name: String? = "laptop-main", id: String = "7B3E-0D15", hosts: List<String> = HOSTS, days: Int = 30,
        end: String = "2026-11-25", budget: Int = 10, lo: Long = PHONE_EPOCH, hi: Long = PHONE_EPOCH + 1, thisPhone: Boolean = false,
        refusal: TrustRefusal? = null,
    ): TrustRequestView {
        val met = name != null
        val who = if (met) "$name [$id]" else "not met [$id]"
        return TrustRequestView(
            token = token, device = TrustDevice(name, id, met),
            headline = "$who asks to open ${hosts.size} hosts for $days days",
            fields = listOf(
                TrustFieldView("Trusted device", if (thisPhone) "$who (this phone)" else who),
                TrustFieldView("Hosts (${hosts.size})", if (hosts.isEmpty()) "none" else hosts.joinToString(", ")),
                TrustFieldView("Term", if (days > 0) "$days days, ends $end" else "over: ended $end"),
                TrustFieldView("New hosts per day", budget.toString()),
            ),
            epoch = TrustEpochView(
                "Epoch range: must start at epoch $PHONE_EPOCH, at most one epoch wide", "epoch $lo to $hi",
                listOf(TrustCheck("starts at this phone's own epoch $PHONE_EPOCH", lo == PHONE_EPOCH), TrustCheck("at most one epoch wide", hi >= lo && hi - lo <= 1)),
            ),
            signedSummary = "$who, ${hosts.size} hosts, $days days, ends $end",
            refusal = refusal,
        )
    }

    val REFUSAL_TEXTS = listOf(
        "The term is longer than 90 days (120 days). Ask for a shorter one.",
        "The daily budget of new hosts must be between 1 and 50, not 80.",
        "A list names 1 to 64 hosts; this one names none.",
        "A list names 1 to 64 hosts; use a group for more.",
        "The epoch range is inverted.",
        "The epoch range 7 to 9 is wider than a terminal asks for (this phone's epoch plus 1): the certificate would outlive the next reshare.",
        "The range starts at epoch 6, but this phone is at epoch 7. Deliver this phone's newest ops to the terminal (Export my ops) and ask again.",
        "The request names this phone as the trusted device. A phone does not delegate to itself.",
        "The term has already ended: the expiry is not in the future.",
    )

    val requestOk = request(1)
    val requestUnmet = request(2, name = null, id = UNMET_ID)

    fun requestRefused(n: Int): TrustRequestView {
        val r = TrustRefusal(n, "Refused. Nothing was signed.", REFUSAL_TEXTS[n])
        val t = (10 + n).toLong()
        return when (n) {
            0 -> request(t, days = 120, end = "2027-02-23", refusal = r)
            1 -> request(t, budget = 80, refusal = r)
            2 -> request(t, hosts = emptyList(), refusal = r)
            3 -> request(t, hosts = MANY_HOSTS, refusal = r)
            4 -> request(t, lo = PHONE_EPOCH + 1, hi = PHONE_EPOCH, refusal = r)
            5 -> request(t, hi = PHONE_EPOCH + 2, refusal = r)
            6 -> request(t, lo = PHONE_EPOCH - 1, hi = PHONE_EPOCH, refusal = r)
            7 -> request(t, name = "pixel-8", id = "4F2A-91C7", thisPhone = true, refusal = r)
            else -> request(t, days = 0, end = "2026-10-20", refusal = r)
        }
    }

    private fun extend(token: Long, name: String, id: String, certId: String, hosts: Int, ends: String, budget: Int, refusal: TrustRefusal? = null, again: TrustAgain? = null) =
        TrustExtendView(
            token = token,
            headline = if (refusal == null) "Allow 10 more new hosts for the rolling day on a trust session" else "for 1 to 50 more new hosts for one certificate",
            fields = listOf(
                TrustFieldView("trusted device", "$name [$id]"), TrustFieldView("certificate", certId), TrustFieldView("hosts", hosts.toString()),
                TrustFieldView("term ends", ends), TrustFieldView("budget now", "$budget per day, used $budget of $budget"), TrustFieldView("requested", "+10 new hosts"),
            ),
            diffKey = "New hosts allowed in the next 24 hours", diffFrom = "$budget per day, $budget of $budget used", diffTo = "10 more, for 24 hours only",
            note = "An extension covers its own 24 hours only. It never changes scope or term. Allowed range: 1 to 50.",
            signedSummary = "$name [$id], +10 new hosts for 24 hours",
            refusal = refusal, again = again,
        )

    val extendOk = extend(20, "workstation", "9C61-E2A8", "D41E-9A07", 3, "2026-11-03", 10)
    val extendNotMine = extend(21, "nuc-lab", "C5F0-27B9", "A81C-33F0", 2, "2026-11-15", 10,
        TrustRefusal(0, "Refused. This phone did not issue this certificate.", "Only the phone that signed a certificate can extend its budget. Nothing was signed."))
    val extendRevoked = extend(22, "laptop-old", "1D88-A340", "6B20-F1D5", 2, "2026-11-27", 20,
        TrustRefusal(1, "Refused. This certificate is revoked or expired.", "A budget extension cannot revive it. Nothing was signed."))
    val extendAgain = extend(23, "laptop-main", "7B3E-0D15", "3F9A-07C2", 5, "2026-11-11", 10,
        TrustRefusal(2, "Refused. Extended less than 24 hours ago.", "A second extension of the same certificate within 24 hours is refused. Possible again in 17 h 22 min."),
        TrustAgain(17 * 3600L + 22 * 60L, 86_400L))

    private fun renew(token: Long, evidenceAge: Long?, refusal: TrustRefusal? = null, left: Int = 2, opened: List<String> = listOf("web-01", "db-staging"), openedTotal: Int = 11) = TrustRenewView(
        token = token, headline = "Renew the trust session of workstation [9C61-E2A8] until 2026-11-17",
        plain = "workstation may keep opening 3 hosts for 14 more days.",
        delegate = "workstation [9C61-E2A8]", scope = "3 listed hosts", termSecs = 14 * 86_400L, budget = 10, renewalsLeft = left,
        newEnd = day(2026, 11, 17), opened = opened, openedTotal = openedTotal, evidenceAgeSecs = evidenceAge, refusal = refusal,
        note = if (refusal == null) "A renewal needs no second terminal: this phone alone extends the trust session within the limits shown." else "",
        signedSummary = "workstation [9C61-E2A8], until 2026-11-17",
    )

    /** Renewal tokens: 30 fresh evidence, 31 stale (a day old), 32 only the delegate vouches, 33 over the cap (refused). */
    val renewFresh = renew(30, 60)
    val renewStale = renew(31, 86_400)
    val renewDelegateOnly = renew(32, null)
    val renewNoList = renew(34, 60, opened = emptyList(), openedTotal = 0)
    val renewOverCap = renew(33, 60, TrustRefusal(-1, "Request refused", "This trust session was renewed 2 times already: a new one needs the full approval."), left = 0)

    /** Handover tokens: 40 the old phone's request, 41 the new phone's admission card. */
    val handoverOld = PhoneHandoverView(
        token = 40, name = "pixel-9", words = listOf("amber", "basket", "cactus", "dolphin", "ember", "falcon", "garden", "harbor"),
        certsCarried = listOf("workstation [9C61-E2A8] until 2026-11-03"), opHash = null,
        note = "Compare these words with the terminal's screen. They must be identical, word for word: if they differ, refuse.",
    )
    val handoverNew = PhoneHandoverView(
        token = 41, name = "pixel-9", words = emptyList(), certsCarried = emptyList(), opHash = ByteArray(32) { (it * 7 + 1).toByte() },
        note = "This phone is the new phone of the ring. Sign only if you started this replacement.",
    )

    fun certs(): List<IssuedCert> = listOf(
        IssuedCert("3F9A-07C2", "laptop-main", "7B3E-0D15", CertState.Active, UIDS, null, day(2026, 10, 12), day(2026, 11, 11), PHONE_EPOCH, PHONE_EPOCH + 1, "30 days", 10,
            listOf(CertExtensionInfo(day(2026, 10, 26) + 3 * 3600 + 3 * 60, 10)), 42),
        IssuedCert("D41E-9A07", "workstation", "9C61-E2A8", CertState.Active, UIDS.take(3), null, day(2026, 10, 20), day(2026, 11, 3), PHONE_EPOCH, PHONE_EPOCH + 1, "14 days", 10, emptyList(), 47),
        IssuedCert("8E55-1C3B", null, "5A19-C0E4", CertState.Expired, UIDS.take(1), null, day(2026, 10, 5), day(2026, 10, 12), PHONE_EPOCH - 1, PHONE_EPOCH, "7 days", 5, emptyList(), null),
        IssuedCert("6B20-F1D5", "laptop-old", "1D88-A340", CertState.Revoked, emptyList(), "prod-web", day(2026, 9, 28), day(2026, 11, 27), PHONE_EPOCH, PHONE_EPOCH, "60 days", 20, emptyList(), 31),
    )

    fun codes(): List<ShortCodeRow> = listOf(
        ShortCodeRow(uid(1), ShortCodeState.Live, day(2026, 10, 24), day(2026, 10, 31)),
        ShortCodeRow(uid(2), ShortCodeState.Live, day(2026, 10, 22), day(2026, 10, 29)),
        ShortCodeRow(uid(3), ShortCodeState.Expired, day(2026, 10, 12), day(2026, 10, 19)),
        ShortCodeRow(uid(4), ShortCodeState.DeclaredDead, day(2026, 10, 23), day(2026, 10, 30)),
    )

    fun codeIssued() = CodeIssuedView.Issued("deploy@web-01", CODE, day(2026, 10, 26), day(2026, 11, 2))
    fun codeLive() = CodeIssuedView.Live("deploy@db-staging", day(2026, 10, 31))

    /** A deterministic, NOT scannable, pseudo QR plus pseudo base64 text; one frame so no loop runs in tests. */
    fun outgoing(seed: String): Outgoing {
        var a = seed.fold(7) { h, c -> h * 31 + c.code }
        fun next(): Int { a = a * 1_664_525 + 1_013_904_223; return a ushr 24 }
        val n = 29
        val bits = BooleanArray(n * n) { next() < 110 }
        val alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        val text = (1..240).map { alphabet[next() % 64] }.joinToString("") + "=="
        return Outgoing(frame = seed.toByteArray(), text = text, qr = listOf(QrMatrix(n, bits)))
    }
}

class FakeTrust(var now: Long = TrustSamples.NOW) : TrustApi {
    val requests = mutableMapOf<Long, TrustRequestView>().apply {
        put(1, TrustSamples.requestOk); put(2, TrustSamples.requestUnmet)
        for (n in 0..8) put((10 + n).toLong(), TrustSamples.requestRefused(n))
    }
    val extensions = mapOf(20L to TrustSamples.extendOk, 21L to TrustSamples.extendNotMine, 22L to TrustSamples.extendRevoked, 23L to TrustSamples.extendAgain)
    val renewals = mapOf(30L to TrustSamples.renewFresh, 31L to TrustSamples.renewStale, 32L to TrustSamples.renewDelegateOnly, 33L to TrustSamples.renewOverCap, 34L to TrustSamples.renewNoList)
    val handovers = mapOf(40L to TrustSamples.handoverOld, 41L to TrustSamples.handoverNew)
    val certs = TrustSamples.certs().toMutableList()
    val codes = TrustSamples.codes().toMutableList()
    var pendingCode: CodeIssuedView? = TrustSamples.codeIssued()
    /** Every decision in order, for tests. */
    val decisions = mutableListOf<Pair<Long, TrustDecision>>()

    override suspend fun trustRequest(token: Long) = requests[token]
    override suspend fun trustExtend(token: Long) = extensions[token]
    override suspend fun trustRenew(token: Long) = renewals[token]
    override suspend fun phoneHandover(token: Long) = handovers[token]

    override suspend fun decideTrust(token: Long, decision: TrustDecision, now: Long): Outgoing? {
        decisions += token to decision
        if (decision is TrustDecision.Decline) return null
        val req = requests[token]
        if (req != null) {
            if (req.refusal != null) throw CoreException(CoreException.Code.Refused, req.refusal.text)
            if (!req.device.met && (decision as TrustDecision.Approve).typedId?.trim()?.equals(req.device.id, ignoreCase = true) != true) {
                throw CoreException(CoreException.Code.Refused, "The typed id does not match the requester.")
            }
            return TrustSamples.outgoing("trust-$token")
        }
        if (handovers.containsKey(token)) return TrustSamples.outgoing("handover-$token")
        renewals[token]?.let {
            if (it.red) throw CoreException(CoreException.Code.Refused, it.refusal?.text ?: "The news from the other terminals is too old.")
            return TrustSamples.outgoing("renew-$token")
        }
        val ext = extensions[token] ?: throw CoreException(CoreException.Code.Malformed, "unknown token")
        if (ext.refusal != null) throw CoreException(CoreException.Code.Refused, ext.refusal.text)
        return TrustSamples.outgoing("ext-$token")
    }

    override suspend fun issuedCerts(now: Long): List<IssuedCert> = certs.toList()

    override suspend fun revokeCert(certId: String, now: Long): Outgoing {
        val i = certs.indexOfFirst { it.certId == certId }
        if (i < 0 || certs[i].state != CertState.Active) throw CoreException(CoreException.Code.Refused, "Only an active certificate can be revoked.")
        certs[i] = certs[i].copy(state = CertState.Revoked)
        return TrustSamples.outgoing("revoke-$certId")
    }

    override suspend fun shortCodes(now: Long): List<ShortCodeRow> = codes.toList()

    override suspend fun declareShortCodeDead(host: String) {
        val i = codes.indexOfFirst { it.host == host && it.state == ShortCodeState.Live }
        if (i < 0) throw CoreException(CoreException.Code.Refused, "No live short code for $host.")
        codes[i] = codes[i].copy(state = ShortCodeState.DeclaredDead)
    }

    override suspend fun takeIssuedShortCode(now: Long): CodeIssuedView? = pendingCode.also { pendingCode = null }
}
