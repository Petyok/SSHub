package uk.petruha.sshub.phone.fake

import java.time.Instant
import uk.petruha.sshub.phone.core.ChainAdopted
import uk.petruha.sshub.phone.core.ChainCheck
import uk.petruha.sshub.phone.core.ChainTaken
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.IdentityApi
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.OpenedChain
import uk.petruha.sshub.phone.core.PhoneStatus
import uk.petruha.sshub.phone.core.ScanApi
import uk.petruha.sshub.phone.core.ScanEvent
import uk.petruha.sshub.phone.core.ThemeChoice
import uk.petruha.sshub.phone.core.WordState
import uk.petruha.sshub.phone.ui.screens.identity.AboutState
import uk.petruha.sshub.phone.ui.screens.identity.BackupPhase
import uk.petruha.sshub.phone.ui.screens.identity.BackupState
import uk.petruha.sshub.phone.ui.screens.identity.ChainHeadPhase
import uk.petruha.sshub.phone.ui.screens.identity.ChainHeadState
import uk.petruha.sshub.phone.ui.screens.identity.FirstRunPhase
import uk.petruha.sshub.phone.ui.screens.identity.FirstRunState
import uk.petruha.sshub.phone.ui.screens.identity.LicenceText
import uk.petruha.sshub.phone.ui.screens.identity.LockPhase
import uk.petruha.sshub.phone.ui.screens.identity.LockReason
import uk.petruha.sshub.phone.ui.screens.identity.LockState
import uk.petruha.sshub.phone.ui.screens.identity.ReplaceKeyState
import uk.petruha.sshub.phone.ui.screens.identity.RestorePhase
import uk.petruha.sshub.phone.ui.screens.identity.RestoreState
import uk.petruha.sshub.phone.ui.screens.identity.SecurityMenuState

/** The prototype's demo backup words (data.js). NOT a real key. The fake core accepts exactly these, as the demo list. */
object DemoWords {
    val backup = listOf(
        "edging", "evolution", "sanitizer", "valley", "sandfish", "surging", "twitch", "unsmooth", "dad", "reseal", "district", "rigor",
        "composer", "unfrozen", "disengage", "reload", "slackness", "dandruff", "clad", "blurb", "overexert", "turkey", "detract", "snare",
    )

    fun stateOf(word: String): WordState = when {
        word.isEmpty() -> WordState.Empty
        word in backup -> WordState.Ok
        backup.any { it.startsWith(word) } -> WordState.Partial
        else -> WordState.Bad
    }
}

/** Core identity group with the prototype's data: pixel-8, 4F2A-91C7, newest op seq 41 signed 2026-10-23 14:02Z. */
class FakeIdentityApi : IdentityApi {
    var deviceId = "4F2A-91C7"
    var deviceName = "pixel-8"
    var enrolled = true
    var headPending = false
    var unlocked = false
    /** Make `restoreFromWords` refuse (checksum wrong). */
    var rejectRestore = false
    var restoredWith: List<String>? = null
    var adoptedToken: Long? = null
    var neverSignedWith: String? = null
    val secret = ByteArray(32) { 0x42 }

    override fun status(now: Long) = PhoneStatus(deviceId, deviceName, enrolled, headPending, 7)
    override fun createSecret(): ByteArray = secret.copyOf()

    override fun restoreFromWords(words: List<String>): ByteArray {
        if (rejectRestore || words.size != 24 || words.any { DemoWords.stateOf(it) != WordState.Ok }) throw CoreException(CoreException.Code.Malformed, "these words are not a valid recovery phrase")
        restoredWith = words
        headPending = true
        return secret.copyOf()
    }

    override fun validateWord(word: String): WordState = DemoWords.stateOf(word)
    override fun backupWords(): List<CharArray> = DemoWords.backup.map { it.toCharArray() }
    override fun unlock(secret: ByteArray, now: Long): PhoneStatus { unlocked = true; return status(now) }
    override fun lock() { unlocked = false }

    override suspend fun neverSigned(typedId: String, now: Long): PhoneStatus {
        if (typedId.trim().uppercase() != deviceId) throw CoreException(CoreException.Code.Malformed, "the typed id does not match this phone")
        neverSignedWith = typedId
        headPending = false
        return status(now)
    }

    override suspend fun adoptChain(token: Long, now: Long): ChainTaken {
        adoptedToken = token
        headPending = false
        return ChainTaken(IdentitySamples.SEQ, status(now))
    }
}

/**
 * The `ScanApi` half the chain-head screen needs: frame text is a script. "LIST" frames are good own-ops lists (the second one agrees with
 * the first), "BAD" is a list whose signature does not verify, anything else is ignored. Everything else is inert.
 */
class FakeChainScan : ScanApi {
    var listsRead = 0
    override fun scanGray(width: Int, height: Int, luma: ByteArray): ScanEvent = ScanEvent.None
    override fun resetScan() {}
    override fun frameFromText(text: String): ByteArray? = text.trim().takeIf { it.length >= 16 }?.toByteArray()

    override suspend fun openFrame(frame: ByteArray, now: Long): Opened {
        val text = String(frame)
        return when {
            text.contains("LIST") -> { listsRead++; IdentitySamples.chainList(agreed = listsRead >= 2) }
            text.contains("BAD") -> IdentitySamples.refusedList()
            else -> uk.petruha.sshub.phone.core.Opened.Ignored("This is not an ops list of this phone.")
        }
    }
}

/** Sample states for previews and tests. */
object IdentitySamples {
    const val SEQ = 41L
    val SIGNED: Long = Instant.parse("2026-10-23T14:02:00Z").epochSecond
    const val FINGERPRINT = "e32892795e94861978c8b5edccca2b1099173e6316591ebf4ce5f2121c189d7c"
    /** One digit changed in group 4. */
    const val OTHER_FINGERPRINT = "e32892795e94961978c8b5edccca2b1099173e6316591ebf4ce5f2121c189d7c"

    private val goodChecks = listOf(
        ChainCheck("Every op is by this key", true),
        ChainCheck("Each signature verifies", true),
        ChainCheck("The ops form one chain", true),
        ChainCheck("The list continues after the newest op, seq $SEQ", true),
    )
    val badChecks = listOf(
        ChainCheck("Every op is by this key", true),
        ChainCheck("The signature of op seq 40 does not verify", false),
        ChainCheck("The ops form one chain", null),
        ChainCheck("The list continues after the newest op, seq $SEQ", null),
    )

    /** The core's own wording, as the page shows it (assets/companion/src/app.mjs scanHead). */
    const val SUMMARY = "Newest op in this list: seq 41, signed 2026-10-23 14:02Z (8 op(s) read)."
    const val NOTE_NEXT = "Take a second list, from another terminal or from the same one later, before this chain head is trusted."
    const val NOTE_AGREED = "This phone will continue after seq 41. If any terminal holds a higher seq of this phone, do not adopt this: signing on top of an old head forks the chain. Take a newer list instead."

    fun chain(agreed: Boolean, summary: String = SUMMARY, note: String = if (agreed) NOTE_AGREED else NOTE_NEXT) =
        ChainAdopted(token = 7, agreed = agreed, headSeq = SEQ, newestAt = SIGNED, ops = 8, previousSeq = null, summary = summary, note = note)
    fun chainList(agreed: Boolean): Opened = OpenedChain(chain(agreed), goodChecks)

    /** A refused list in the release group's value type, like the real core returns it. */
    fun refusedList(): Opened = uk.petruha.sshub.phone.core.OpenedRefused(
        uk.petruha.sshub.phone.core.RefusalReason(
            key = "badsig", label = "bad signature", title = "The signature of op seq 40 does not verify.",
            text = "An op in the list does not verify. Nothing was adopted.", compared = emptyList(), mismatch = false,
        ),
        badChecks.map { uk.petruha.sshub.phone.core.ReleaseCheck(it.text, it.ok == true) },
    )

    val firstRun = FirstRunState()
    val firstRunNoLock = FirstRunState(FirstRunPhase.NoSecureLock)
    val firstRunCreating = FirstRunState(FirstRunPhase.Creating)

    fun restore(
        words: List<String>, cleared: Boolean = false, replaces: Boolean = false, phase: RestorePhase = RestorePhase.Editing,
        usedPage: Boolean? = true,
    ): RestoreState {
        val padded = words + List(24 - words.size) { "" }
        return RestoreState(padded, padded.map { DemoWords.stateOf(it) }, usedPage = usedPage, siteDataCleared = cleared, replaces = replaces, phase = phase)
    }
    val restoreEmpty = restore(emptyList())
    val restorePartial = restore(DemoWords.backup.take(9))
    val restoreBad = restore(DemoWords.backup.toMutableList().also { it[7] = "twich"; it[19] = "blurbb" }, cleared = true)
    val restoreFull = restore(DemoWords.backup, cleared = true)
    val restoreFullNotCleared = restore(DemoWords.backup, cleared = false)
    /** All words, the question not answered yet. */
    val restoreFullUnanswered = restore(DemoWords.backup, usedPage = null)
    /** All words, the page never used: no box to tick. */
    val restoreFullNoPage = restore(DemoWords.backup, usedPage = false)
    val restoreOver = restore(emptyList(), replaces = true)

    val backupAuthenticating = BackupState(phase = BackupPhase.Authenticating, firstRun = true)
    val backupReady = BackupState(DemoWords.backup, BackupPhase.Ready, firstRun = true)
    val backupFromMenu = BackupState(DemoWords.backup, BackupPhase.Ready)
    val backupCanceled = BackupState(phase = BackupPhase.Canceled, message = "Unlock was canceled. The words stay hidden.")

    val lockWaiting = LockState(LockPhase.Waiting, LockReason.ScreenOff)
    val lockCanceled = LockState(LockPhase.Canceled, LockReason.ScreenOff)
    val lockLockedOut = LockState(LockPhase.LockedOut)
    val lockInvalidated = LockState(LockPhase.Invalidated)

    val replaceKey = ReplaceKeyState()
    val replaceKeyReady = ReplaceKeyState("ERASE")

    private const val DEVICE_ID = "4F2A-91C7"
    val chainHeadStart = ChainHeadState(deviceId = DEVICE_ID)
    val chainHeadOneList = ChainHeadState(lists = 1, chain = chain(false), checks = goodChecks, deviceId = DEVICE_ID)
    val chainHeadAgree = ChainHeadState(lists = 2, chain = chain(true), checks = goodChecks, deviceId = DEVICE_ID)
    val chainHeadPaste = ChainHeadState(phase = ChainHeadPhase.Paste, deviceId = DEVICE_ID)
    val chainHeadTaken = ChainHeadState(phase = ChainHeadPhase.Taken, lists = 2, chain = chain(true), takenSeq = SEQ, deviceId = DEVICE_ID)
    val chainHeadNeverSigned = ChainHeadState(phase = ChainHeadPhase.Taken, neverSigned = true, deviceId = DEVICE_ID)
    val chainHeadRefused = ChainHeadState(
        phase = ChainHeadPhase.Refused, lists = 0, checks = badChecks, deviceId = DEVICE_ID,
        refusal = "The signature of op seq 40 does not verify.",
    )

    val menu = SecurityMenuState("pixel-8", DEVICE_ID, ThemeChoice.Dark, 60, backupConfirmed = false)
    val menuBlocked = menu.copy(blocked = true)

    val licences = listOf(
        LicenceText("JetBrains Mono 2.304", "Copyright 2020 The JetBrains Mono Project Authors", "https://github.com/JetBrains/JetBrainsMono", "licence text of JetBrainsMono-OFL.txt"),
        LicenceText("Inter 4.1", "Copyright (c) 2016 The Inter Project Authors", "https://github.com/rsms/inter", "licence text of Inter-LICENSE.txt"),
    )
    val about = AboutState(
        versionName = "0.1.0 (build 1)", deviceName = "pixel-8", deviceId = DEVICE_ID, fingerprint = FINGERPRINT,
        autoLockSeconds = 60, strongBox = true, licences = licences,
    )
    val aboutComparingEqual = about.copy(comparing = true, reference = FINGERPRINT.chunked(4).joinToString(" "), checked = setOf(0, 1, 2))
    val aboutComparingDiffer = about.copy(comparing = true, reference = OTHER_FINGERPRINT)
    val aboutComparingPartial = about.copy(comparing = true, reference = FINGERPRINT.take(20))
    val aboutLicences = about.copy(licencesOpen = true)
}
