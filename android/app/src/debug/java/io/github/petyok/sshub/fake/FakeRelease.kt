package io.github.petyok.sshub.fake

import io.github.petyok.sshub.core.ChannelKind
import io.github.petyok.sshub.core.CoreException
import io.github.petyok.sshub.core.IssuedShortCode
import io.github.petyok.sshub.core.Opened
import io.github.petyok.sshub.core.OpenedRed
import io.github.petyok.sshub.core.OpenedRefused
import io.github.petyok.sshub.core.OpenedRelease
import io.github.petyok.sshub.core.Outgoing
import io.github.petyok.sshub.core.ProofState
import io.github.petyok.sshub.core.RedField
import io.github.petyok.sshub.core.RedView
import io.github.petyok.sshub.core.RefusalReason
import io.github.petyok.sshub.core.RefusalRow
import io.github.petyok.sshub.core.ReleaseApi
import io.github.petyok.sshub.core.ReleaseChannel
import io.github.petyok.sshub.core.ReleaseCheck
import io.github.petyok.sshub.core.ReleaseDecision
import io.github.petyok.sshub.core.ReleaseOutcome
import io.github.petyok.sshub.core.ReleaseView
import io.github.petyok.sshub.core.ShareWords
import io.github.petyok.sshub.ui.components.QrMatrix
import io.github.petyok.sshub.ui.screens.release.RedState
import io.github.petyok.sshub.ui.screens.release.ReleaseState

/** Deterministic Release group core: serves the prototype's sample notices and answers decisions without Rust. */
class FakeReleaseApi : ReleaseApi {
    val views = mutableMapOf<Long, Opened>(
        ReleaseSamples.TOKEN_REVOKE to OpenedRed(ReleaseSamples.redRevoke),
        ReleaseSamples.TOKEN_MODE to OpenedRed(ReleaseSamples.redMode),
        ReleaseSamples.TOKEN_DBS to OpenedRelease(ReleaseSamples.releaseDbs),
        ReleaseSamples.TOKEN_BASTION to OpenedRelease(ReleaseSamples.releaseBastion),
    )
    /** Every decision the UI sent, in order. */
    val decisions = mutableListOf<Triple<Long, ReleaseDecision, Long>>()

    override fun pendingView(token: Long): Opened? = views[token]

    override suspend fun decide(token: Long, decision: ReleaseDecision, now: Long): ReleaseOutcome {
        decisions += Triple(token, decision, now)
        val view = views[token] ?: throw CoreException(CoreException.Code.Refused, "No such request.")
        if (decision is ReleaseDecision.Decline) return ReleaseOutcome(null)
        decision as ReleaseDecision.Approve
        return when (view) {
            is OpenedRed -> {
                if (now >= view.view.lapsesAt) throw CoreException(CoreException.Code.Expired, "The notice lapsed.")
                ReleaseOutcome(ReleaseSamples.outgoing(), note = "phone-proof " + view.view.proposerId)
            }
            is OpenedRelease -> {
                val v = view.view
                if (now >= v.expiresAt) throw CoreException(CoreException.Code.Expired, "The challenge expired.")
                if (v.mustType != null && decision.typedId?.trim()?.uppercase() != v.mustType) {
                    throw CoreException(CoreException.Code.Refused, "The typed id does not match.")
                }
                val c = decision.channels
                ReleaseOutcome(
                    outgoing = ReleaseSamples.outgoing(
                        audioFast = if (c.soundFast) ShortArray(16) else null, audioSlow = if (c.soundSlow) ShortArray(16) else null,
                    ),
                    shortCode = if (decision.shortCode) IssuedShortCode(ReleaseSamples.SHORT_CODE, now + 7 * 86_400, weak = false) else null,
                    byHand = if (decision.byHand) ShareWords(2, ReleaseSamples.words24) else null,
                )
            }
            else -> throw CoreException(CoreException.Code.Internal, "Not a release group token.")
        }
    }
}

/** Prototype data (design-demos/data.js) as models and screen states. Times are relative to `T0`, the FakePlatform clock. */
object ReleaseSamples {
    const val T0 = 1_893_452_500L
    const val TOKEN_REVOKE = 1L
    const val TOKEN_MODE = 2L
    const val TOKEN_DBS = 3L
    const val TOKEN_BASTION = 4L
    const val SHORT_CODE = "K7Q2-9FMX-C41T-8PWD"
    const val THREE_DAYS = 3 * 86_400L

    val words24 = listOf(
        "edging", "evolution", "sanitizer", "valley", "sandfish", "surging", "twitch", "unsmooth", "dad", "reseal", "district", "rigor",
        "composer", "unfrozen", "disengage", "reload", "slackness", "dandruff", "clad", "blurb", "overexert", "turkey", "detract", "snare",
    )

    /** One QR frame only: a looping QR would keep a Compose test from going idle. */
    fun qr(): QrMatrix {
        val n = 25
        var a = 12345
        val bits = BooleanArray(n * n) {
            a = a * 1103515245 + 12345
            (a ushr 16) and 1 == 1
        }
        return QrMatrix(n, bits)
    }

    fun outgoing(audioFast: ShortArray? = null, audioSlow: ShortArray? = null) =
        Outgoing(frame = ByteArray(8) { it.toByte() }, text = "U1NIdWItc2FtcGxlLXJlcGx5", qr = listOf(qr()), audioFast = audioFast, audioSlow = audioSlow)

    private val okChecksRed = listOf(
        ReleaseCheck("Within its 3 day window", true),
        ReleaseCheck("Hash matches the operation", true),
        ReleaseCheck("Signature of the proposer verifies", true),
        ReleaseCheck("Is a Red operation", true),
    )

    val redRevoke = RedView(
        token = TOKEN_REVOKE, kind = "Revoke holder", title = "Revoke device laptop-old",
        proposerName = "laptop-main", proposerId = "7B3E-0D15", lamport = 1042, seq = 57,
        proposedAt = T0 - 360, lapsesAt = T0 - 360 + THREE_DAYS,
        hash = "1bf0d57ef9dd2818002634d3f9ae2a7ee505dd0696c437b74cbb61358097b930",
        fields = listOf(
            RedField("Operation", null, "Revoke holder"),
            RedField("Device", null, "laptop-old [1D88-A340]", mono = true),
            RedField("Its shares", "valid (epoch 7)", "void"),
            RedField("Holders", "4", "3"),
            RedField("Ring epoch", "7", "8 (reshare starts)"),
            RedField("Credentials affected", null, "9 Quorum credentials are reshared"),
        ),
        note = "laptop-old has not been seen for 41 days. After this the ring has three holders and needs 2 of 3.",
        phoneProof = ProofState.YourTurn, checks = okChecksRed,
    )

    val redMode = RedView(
        token = TOKEN_MODE, kind = "Credential mode", title = "Switch credential mode to Quorum",
        proposerName = "workstation", proposerId = "9C61-E2A8", lamport = 1039, seq = 31,
        proposedAt = T0 - 4320, lapsesAt = T0 - 4320 + THREE_DAYS,
        hash = "37fa9efb25777ad17430fdc55addb7829cb6d4e53da61b9a4760ddbebb691daf",
        fields = listOf(
            RedField("Operation", null, "Change credential mode"),
            RedField("Credential", null, "deploy@db-staging", mono = true),
            RedField("Mode", "Single", "Quorum"),
            RedField("Threshold", "1 holder", "2 of 3 holders"),
            RedField("Holders", "laptop-main", "laptop-main, workstation, pixel-8"),
            RedField("Unlock", "any terminal", "two holders must release a share"),
        ),
        note = "After this the terminal cannot open deploy@db-staging alone.",
        phoneProof = ProofState.YourTurn, checks = okChecksRed,
    )

    /** A Red notice whose hash check fails: Approve must stay disabled and say why (D8). */
    val redFailing = redRevoke.copy(checks = okChecksRed.dropLast(1) + ReleaseCheck("Hash does not match what the terminal shows", false))
    val redLapsed = redRevoke.copy(proposedAt = T0 - THREE_DAYS - 60, lapsesAt = T0 - 60)
    /** A proposer this phone never met: the core has no name for it. */
    val redUnknownProposer = redRevoke.copy(proposerName = null, mustType = redRevoke.proposerId)

    /** The host of the sample release: 64 hex digits, as the core gives it. */
    const val HOST_UID = "0a0b0c0d0e0f1011121314151617181920212223242526272829303132333435"

    val releaseDbs = ReleaseView(
        token = TOKEN_DBS, requesterName = "workstation", requesterId = "9C61-E2A8", requesterKnown = true, requesterKey = "9c61e2a8" + "fedcba9876543210".repeat(3) + "01234567",
        shareKind = "hybrid", shareEpoch = 7, shareIndex = 2,
        signedAt = T0 - 38, expiresAt = T0 - 38 + 120,
        words = listOf("compile", "trash", "chop", "valley"),
        checks = listOf(
            ReleaseCheck("Not expired", true), ReleaseCheck("Request and requester agree", true),
            ReleaseCheck("Signature of 9C61-E2A8 verifies", true), ReleaseCheck("Own share unwrapped", true),
        ),
        channels = listOf(
            ReleaseChannel(ChannelKind.Qr, "QR code"), ReleaseChannel(ChannelKind.SoundFast, "Sound (fast)"),
            ReleaseChannel(ChannelKind.SoundSlow, "Sound (slow)"),
        ),
        mustType = null, shortCodeOffer = "Opens this share later without the phone, 7 days",
        hostUid = HOST_UID, bindingHash = "ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12cd34ef56ab12",
        opRows = listOf(
            RedField("Op signed by", null, "workstation [9C61-E2A8]", danger = false),
            RedField("host_uid", null, HOST_UID, mono = true),
            RedField("requester", null, "workstation [9C61-E2A8]"),
        ),
    )

    val releaseBastion = ReleaseView(
        token = TOKEN_BASTION, requesterName = null, requesterId = "E7A4-6B02", requesterKnown = false, requesterKey = null,
        shareKind = "envelope", shareEpoch = 7, shareIndex = 1,
        signedAt = T0 - 89, expiresAt = T0 - 89 + 120,
        words = listOf("pursuit", "anyhow", "sanding", "exhaust"),
        checks = listOf(
            ReleaseCheck("Not expired", true), ReleaseCheck("Request and requester agree", true),
            ReleaseCheck("Signature of E7A4-6B02 cannot be checked: device never met", false, needsTypedId = true),
            ReleaseCheck("Own share unwrapped", true),
        ),
        channels = listOf(ReleaseChannel(ChannelKind.Qr, "QR code")),
        mustType = "E7A4-6B02", shortCodeOffer = "Opens this share later without the phone, 7 days",
    )

    fun redState(view: RedView = redRevoke, now: Long = T0, openedAt: Long = now) = RedState(view, now, openedAt)
    fun releaseState(view: ReleaseView = releaseDbs, now: Long = T0, openedAt: Long = now) = ReleaseState(view, now, openedAt)

    private fun refusal(key: String, label: String, title: String, text: String, mismatch: Boolean = false, vararg rows: Pair<String, String>) =
        OpenedRefused(RefusalReason(key, label, title, text, rows.map { RefusalRow(it.first, it.second) }, mismatch), emptyList())

    /** The 18 refusals of the prototype, in its order. Keys are the ones the core sends. */
    val refusals: List<OpenedRefused> = listOf(
        refusal("malformed", "malformed", "The challenge is malformed.", "The QR content is not a valid request, or it is damaged."),
        refusal("expired", "expired", "The challenge expired 47 s ago.", "The request is older than its expiry on this phone's clock. Ask the terminal again.", false, "Expired" to "47 s ago", "A request lives" to "about 120 s"),
        refusal("lives", "long expiry", "The challenge claims to live 3600 s.", "The expiry is much longer than any terminal issues.", false, "Claimed life" to "3600 s", "A terminal issues" to "about 120 s"),
        refusal("notrelease", "not a release", "This is a Recovery operation, not a share release.", "The request is for something this phone does not release."),
        refusal("reqid", "request id", "The request asks for a different request id.", "The operation and the request disagree."),
        refusal("signer", "signer differs", "Signed by 7B3E-0D15 but names 9C61-E2A8 as the requester.", "The device that signed is not the one named as asking.", true, "Signed by" to "laptop-main 7B3E-0D15", "Names as requester" to "workstation 9C61-E2A8"),
        refusal("badsig", "bad signature", "The signature of 9C61-E2A8 does not verify.", "The operation was altered after it was signed, or the signature is wrong.", true, "Signature of" to "workstation 9C61-E2A8", "Result" to "does not verify"),
        refusal("wrongcred", "other credential", "The share inside the material belongs to a different credential.", "The terminal paired the description of one credential with the share of another.", true, "Request names" to "deploy@db-staging", "Share belongs to" to "ops@bastion"),
        refusal("notforme", "not for this phone", "This phone cannot release for this request.", "The material was not made for this phone's key, or the share does not belong to this credential."),
        refusal("redold", "Red notice old", "This Red notice is older than 3 days.", "A Red notice lives three days. Ask the terminal to propose it again.", false, "Notice age" to "4 d 2 h", "Limit" to "3 d"),
        refusal("notred", "not a Red op", "This is not a Red operation.", "This notice does not describe a Red operation, so this phone has no proof to add to it."),
        refusal("certother", "cert by other", "A trust certificate must be issued by the phone itself. This one was written by another device.", "The phone writes its own certificates. Do not approve this one.", true, "Written by" to "laptop-main 7B3E-0D15", "Must be written by" to "this phone 4F2A-91C7"),
        refusal("clock", "clock", "The challenge expired 212 s ago. Check the phone's clock.", "If the terminal showed this code a moment ago, this phone's clock may be ahead. Set the time, then scan again.", false, "Expired" to "212 s ago", "Phone clock" to "compare with the terminal"),
        refusal("old12", "signed too long ago", "The request was signed 14 min ago.", "A request signed more than 12 minutes ago is refused (120 s plus 600 s of clock skew).", false, "Signed" to "14 min ago", "Limit" to "12 min"),
        refusal("future", "dated ahead", "The request is dated in the future.", "It was signed 3 min after this phone's clock now reads. Check the phone's clock.", false, "Signed" to "3 min in the future"),
        refusal("wrapped", "old, fresh expiry", "The challenge expires more than 12 minutes after it was signed.", "An old signed request was wrapped in a fresh challenge.", false, "Signed" to "31 min ago", "Challenge expires" to "in 90 s"),
        refusal("shape", "wrong shape", "This document has the wrong shape.", "A broken op, a list that is not JSON, or a number outside the 64 bit fields ends on this screen."),
        refusal("size", "over 256 KiB", "This document is over 256 KiB.", "An op may be at most 256 KiB; a list carries at most 8 ops and 1 MiB in all.", false, "This op" to "412 KiB", "Limit" to "256 KiB"),
    )

    fun refusal(key: String): OpenedRefused = refusals.first { it.reason.key == key }
}
