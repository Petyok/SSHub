package uk.petruha.sshub.phone.ui.screens.identity

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlin.coroutines.cancellation.CancellationException
import uk.petruha.sshub.phone.core.AppSettings
import uk.petruha.sshub.phone.core.ChainCheck
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.IdentityApi
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.OpenedChain
import uk.petruha.sshub.phone.core.OpenedRefused
import uk.petruha.sshub.phone.core.Platform
import uk.petruha.sshub.phone.core.ScanApi
import uk.petruha.sshub.phone.core.PhoneStatus
import uk.petruha.sshub.phone.core.SettingsStore
import uk.petruha.sshub.phone.core.ThemeChoice
import uk.petruha.sshub.phone.core.VaultResult

/* View models of the Identity group: thin. They hold the group's core interface and the Platform, own no security decision, and zero secrets at once. */

class FirstRunViewModel(private val core: IdentityApi, private val platform: Platform) : ViewModel() {
    private val flow = MutableStateFlow(FirstRunState())
    val state: StateFlow<FirstRunState> = flow.asStateFlow()

    /** Creates the secret, wraps it with the Keystore key, unlocks the core. `onCreated` runs when the key is stored. */
    fun create(onCreated: () -> Unit) {
        if (flow.value.phase == FirstRunPhase.Creating) return
        flow.value = FirstRunState(FirstRunPhase.Creating)
        viewModelScope.launch {
            val secret = core.createSecret()
            try {
                when (val r = platform.keys.store(secret)) {
                    VaultResult.Stored -> {
                        core.unlock(secret, platform.clock.nowSeconds())
                        flow.value = FirstRunState()
                        onCreated()
                    }
                    VaultResult.NoSecureLock -> flow.value = FirstRunState(FirstRunPhase.NoSecureLock)
                    else -> flow.value = FirstRunState(FirstRunPhase.Failed, vaultMessage(r))
                }
            } catch (e: CoreException) {
                flow.value = FirstRunState(FirstRunPhase.Failed, "The core refused to start: ${e.message}")
            } finally {
                secret.fill(0)
            }
        }
    }
}

class RestoreViewModel(
    private val core: IdentityApi, private val platform: Platform, replaces: Boolean, private val settings: SettingsStore = AppSettings,
) : ViewModel() {
    private val flow = MutableStateFlow(RestoreState(replaces = replaces))
    val state: StateFlow<RestoreState> = flow.asStateFlow()

    private fun withWords(words: List<String>, base: RestoreState = flow.value): RestoreState =
        base.copy(words = words, wordStates = words.map { core.validateWord(it) }, phase = RestorePhase.Editing, error = null)

    /** A typed word, or text with spaces: the first token goes to this box and the rest to the following ones. */
    fun onWord(index: Int, text: String) {
        val words = flow.value.words.toMutableList()
        val tokens = text.trim().lowercase().split(Regex("\\s+")).filter { it.isNotEmpty() }
        if (text.any { it.isWhitespace() } && tokens.isNotEmpty()) {
            tokens.forEachIndexed { j, t -> if (index + j < RestoreState.WORDS) words[index + j] = t }
        } else {
            words[index] = text.trim().lowercase()
        }
        flow.value = withWords(words)
    }

    fun onPaste(text: String) { flow.value = flow.value.copy(paste = text) }

    fun onFill() {
        val tokens = flow.value.paste.lowercase().split(Regex("[\\s,]+")).filter { it.isNotEmpty() }.take(RestoreState.WORDS)
        flow.value = withWords(tokens + List(RestoreState.WORDS - tokens.size) { "" }).copy(paste = "")
    }

    fun onUsedPage(used: Boolean) { flow.value = flow.value.copy(usedPage = used, siteDataCleared = if (used) flow.value.siteDataCleared else false) }

    fun onSiteDataCleared(cleared: Boolean) { flow.value = flow.value.copy(siteDataCleared = cleared) }

    /** Restores, wraps the secret with the Keystore key, unlocks the core. `onRestored` runs when the phone holds its key (head pending). */
    fun restore(onRestored: () -> Unit) {
        val st = flow.value
        if (!st.canRestore) return
        flow.value = st.copy(phase = RestorePhase.Restoring, error = null)
        viewModelScope.launch {
            var secret: ByteArray? = null
            try {
                secret = core.restoreFromWords(st.words)
                when (val r = platform.keys.store(secret)) {
                    VaultResult.Stored -> {
                        core.unlock(secret, platform.clock.nowSeconds())
                        settings.setBackupVerified(true) // the user typed all 24 words just now
                        wipe()
                        onRestored()
                    }
                    VaultResult.NoSecureLock -> flow.value = st.copy(phase = RestorePhase.NoSecureLock)
                    else -> flow.value = st.copy(phase = RestorePhase.Failed, error = vaultMessage(r))
                }
            } catch (e: CoreException) {
                flow.value = st.copy(phase = RestorePhase.Failed, error = "These 24 words are not a valid recovery phrase. Check each word and the order.")
            } finally {
                secret?.fill(0)
            }
        }
    }

    private fun wipe() { flow.value = RestoreState(replaces = flow.value.replaces) }
    override fun onCleared() { wipe() }
}

class BackupViewModel(
    private val core: IdentityApi, private val platform: Platform, private val settings: SettingsStore, firstRun: Boolean,
) : ViewModel() {
    private val flow = MutableStateFlow(BackupState(firstRun = firstRun))
    val state: StateFlow<BackupState> = flow.asStateFlow()
    private var busy = false

    init { authenticate() }

    /** D16: the words are read only after a fresh system prompt, in `CharArray`s that are zeroed as soon as they are drawn. */
    fun authenticate() {
        if (busy) return
        busy = true
        flow.value = flow.value.copy(phase = BackupPhase.Authenticating, message = null)
        viewModelScope.launch {
            try {
                when (val r = platform.keys.unlock("Show the recovery words", "Confirm it is you")) {
                    is VaultResult.Secret -> {
                        r.bytes.fill(0)
                        val chars = core.backupWords()
                        val words = chars.map { String(it) }
                        chars.forEach { it.fill('\u0000') }
                        flow.value = flow.value.copy(words = words, phase = BackupPhase.Ready)
                    }
                    VaultResult.Canceled -> flow.value = flow.value.copy(phase = BackupPhase.Canceled, message = "Unlock was canceled. The words stay hidden.")
                    else -> flow.value = flow.value.copy(phase = BackupPhase.Failed, message = vaultMessage(r))
                }
            } catch (e: CoreException) {
                flow.value = flow.value.copy(phase = BackupPhase.Failed, message = "The core could not give the words: ${e.message}")
            } finally {
                busy = false
            }
        }
    }

    fun onPaperChecked(checked: Boolean) { flow.value = flow.value.copy(paperChecked = checked) }

    fun done(onDone: () -> Unit) {
        if (!flow.value.paperChecked) return
        settings.setBackupConfirmed(true)
        flow.value = flow.value.copy(words = emptyList())
        onDone()
    }

    override fun onCleared() { flow.value = flow.value.copy(words = emptyList()) }
}

class LockViewModel(private val core: IdentityApi, private val platform: Platform, reason: LockReason) : ViewModel() {
    private val flow = MutableStateFlow(LockState(reason = reason))
    val state: StateFlow<LockState> = flow.asStateFlow()
    private val unlockedFlow = MutableStateFlow(false)
    /** True once the core is unlocked; the graph then leaves the lock screen. */
    val unlocked: StateFlow<Boolean> = unlockedFlow.asStateFlow()
    private var busy = false

    init { unlock() }

    fun unlock() {
        if (busy || unlockedFlow.value) return
        busy = true
        flow.value = flow.value.copy(phase = LockPhase.Waiting, message = null)
        viewModelScope.launch {
            try {
                when (val r = platform.keys.unlock("Unlock SSHub", "Confirm your fingerprint, face or screen lock")) {
                    is VaultResult.Secret -> try {
                        core.unlock(r.bytes, platform.clock.nowSeconds())
                        unlockedFlow.value = true
                    } catch (e: CoreException) {
                        flow.value = flow.value.copy(phase = LockPhase.Failed, message = "The core refused the key: ${e.message}")
                    } finally {
                        r.bytes.fill(0)
                    }
                    VaultResult.Canceled -> flow.value = flow.value.copy(phase = LockPhase.Canceled)
                    VaultResult.LockedOut -> flow.value = flow.value.copy(phase = LockPhase.LockedOut)
                    VaultResult.Invalidated -> flow.value = flow.value.copy(phase = LockPhase.Invalidated)
                    else -> flow.value = flow.value.copy(phase = LockPhase.Failed, message = vaultMessage(r))
                }
            } finally {
                busy = false
            }
        }
    }
}

class ReplaceKeyViewModel : ViewModel() {
    private val flow = MutableStateFlow(ReplaceKeyState())
    val state: StateFlow<ReplaceKeyState> = flow.asStateFlow()
    fun onTyped(text: String) { flow.value = ReplaceKeyState(text) }
}

class ChainHeadViewModel(private val core: IdentityApi, private val scan: ScanApi, private val platform: Platform) : ViewModel() {
    private val flow = MutableStateFlow(ChainHeadState())
    val state: StateFlow<ChainHeadState> = flow.asStateFlow()

    init { viewModelScope.launch { val id = core.status(platform.clock.nowSeconds()).deviceId; flow.update { it.copy(deviceId = id) } } }

    /** A result of the scan screen or of the paste box: every list goes through `openFrame`, the core says what it is. */
    fun onOpened(opened: Opened) {
        val st = flow.value
        flow.value = when (opened) {
            is OpenedChain -> st.copy(
                phase = ChainHeadPhase.Read, lists = if (opened.chain.agreed) 2 else 1, chain = opened.chain, checks = opened.checks,
                senders = if (opened.chain.agreed) st.senders.takeLast(1) + opened.chain.sender else listOf(opened.chain.sender),
                notice = null, refusal = null, pasteText = "", busy = false,
                singleTicked = false, nowSeconds = platform.clock.nowSeconds(),
            )
            is OpenedRefused -> st.copy(
                phase = ChainHeadPhase.Refused, refusal = opened.reason.title, checks = opened.checks.map { ChainCheck(it.text, it.ok) },
                notice = null, busy = false,
            )
            is Opened.Ignored -> st.copy(phase = if (st.phase == ChainHeadPhase.Paste) ChainHeadPhase.Paste else ChainHeadPhase.Read, notice = "Ignored: ${opened.reason}", busy = false)
            else -> st.copy(notice = "Ignored: this is not an ops list of this phone.", busy = false)
        }
    }

    fun onOpenPaste() { flow.value = flow.value.copy(phase = ChainHeadPhase.Paste, notice = null) }
    fun onClosePaste() { flow.value = flow.value.copy(phase = ChainHeadPhase.Read, pasteText = "") }
    fun onPasteText(text: String) { flow.value = flow.value.copy(pasteText = text) }
    fun onTypedId(text: String) { flow.value = flow.value.copy(typedId = text) }
    fun onTickSingle(on: Boolean) { flow.value = flow.value.copy(singleTicked = on) }

    fun checkPaste() {
        val st = flow.value
        if (!st.canCheckPaste) return
        flow.value = st.copy(busy = true)
        viewModelScope.launch {
            val frame = scan.frameFromText(st.pasteText)
            if (frame == null) {
                flow.value = flow.value.copy(busy = false, notice = "Ignored: that is not text a terminal shows. Copy the whole text.")
                return@launch
            }
            try {
                onOpened(scan.openFrame(frame, platform.clock.nowSeconds()))
            } catch (e: CoreException) {
                flow.value = flow.value.copy(busy = false, notice = "Ignored: ${e.message}")
            }
        }
    }

    /** The user accepts the agreed head: the core's `decide(token, Approve)`. */
    fun adopt() {
        val chain = flow.value.chain ?: return
        if (!chain.agreed || !flow.value.canAdopt) return
        flow.value = flow.value.copy(busy = true)
        viewModelScope.launch {
            try {
                val taken = core.adoptChain(chain.token, platform.clock.nowSeconds(), chain.singleSource)
                flow.value = flow.value.copy(phase = ChainHeadPhase.Taken, takenSeq = taken.headSeq, neverSigned = false, busy = false)
            } catch (e: CoreException) {
                flow.value = flow.value.copy(busy = false, notice = "The core did not adopt it: ${e.message}")
            }
        }
    }

    /** Only for a key that never signed: the core checks the typed id. */
    fun neverSigned() {
        val st = flow.value
        if (!st.canNeverSigned) return
        viewModelScope.launch {
            try {
                core.neverSigned(st.typedId, platform.clock.nowSeconds())
                flow.value = st.copy(phase = ChainHeadPhase.Taken, neverSigned = true, takenSeq = null, typedId = "")
            } catch (e: CoreException) {
                flow.value = st.copy(notice = "Not confirmed: ${e.message}")
            }
        }
    }

    /** Back to reading lists after a refusal. */
    fun tryAnother() {
        val st = flow.value
        flow.value = st.copy(phase = ChainHeadPhase.Read, refusal = null, checks = if (st.lists > 0) st.checks else emptyList(), notice = null)
    }
}

class SecurityMenuViewModel(private val core: IdentityApi, private val platform: Platform, private val settings: SettingsStore) : ViewModel() {
    /** Read from the core's thread; the menu shows without the name and id until it answers. */
    private val status = MutableStateFlow<PhoneStatus?>(null)

    val state: StateFlow<SecurityMenuState> = combine(
        settings.theme, settings.autoLockSeconds, settings.backupConfirmed, settings.signerMismatch, status,
    ) { theme, autoLock, backup, blocked, st ->
        SecurityMenuState(st?.deviceName.orEmpty(), st?.deviceId.orEmpty(), theme, autoLock, backup, blocked, st?.headPending ?: false)
    }.stateIn(
        viewModelScope, SharingStarted.Eagerly,
        SecurityMenuState(
            "", "", settings.theme.value, settings.autoLockSeconds.value,
            settings.backupConfirmed.value, settings.signerMismatch.value, false,
        ),
    )

    init { refresh() }

    /** The chain may have been taken since this model was made (Take chain, then back to the menu). */
    fun refresh() { viewModelScope.launch { status.value = core.status(platform.clock.nowSeconds()) } }

    fun setTheme(choice: ThemeChoice) = settings.setTheme(choice)
    fun setAutoLock(seconds: Int) = settings.setAutoLock(seconds)

    /** Wipes the secret from the core; the graph then shows the lock screen. */
    fun lock() = core.lock()
}

private data class AboutUi(
    val reference: String = "", val comparing: Boolean = false, val checked: Set<Int> = emptySet(), val licencesOpen: Boolean = false,
    val confirmed: Boolean = false, val busy: Boolean = false, val message: String? = null,
)

class AboutViewModel(private val core: IdentityApi, private val platform: Platform, private val settings: SettingsStore) : ViewModel() {
    private val status = MutableStateFlow<PhoneStatus?>(null)
    private val ui = MutableStateFlow(AboutUi())

    init { viewModelScope.launch { status.value = core.status(platform.clock.nowSeconds()) } }
    private val licences: List<LicenceText> by lazy {
        listOf(
            LicenceText("JetBrains Mono 2.304", "Copyright 2020 The JetBrains Mono Project Authors", "https://github.com/JetBrains/JetBrainsMono", platform.system.licenceText("JetBrainsMono-OFL.txt")),
            LicenceText("Inter 4.1", "Copyright (c) 2016 The Inter Project Authors", "https://github.com/rsms/inter", platform.system.licenceText("Inter-LICENSE.txt")),
        )
    }

    private fun build(u: AboutUi, autoLock: Int, backup: Boolean, blocked: Boolean, st: PhoneStatus?) = AboutState(
        versionName = platform.system.versionName, deviceName = st?.deviceName.orEmpty(), deviceId = st?.deviceId.orEmpty(),
        fingerprint = platform.system.apkCertSha256.lowercase(), reference = u.reference, comparing = u.comparing || blocked, checked = u.checked,
        autoLockSeconds = autoLock, strongBox = platform.keys.strongBox, backupConfirmed = backup,
        licencesOpen = u.licencesOpen, licences = if (u.licencesOpen) licences else emptyList(),
        blocked = blocked, confirmed = u.confirmed, busy = u.busy, message = u.message,
    )

    val state: StateFlow<AboutState> = combine(ui, settings.autoLockSeconds, settings.backupConfirmed, settings.signerMismatch, status) { u, a, b, m, st -> build(u, a, b, m, st) }
        .stateIn(viewModelScope, SharingStarted.Eagerly, build(ui.value, settings.autoLockSeconds.value, settings.backupConfirmed.value, settings.signerMismatch.value, null))

    fun toggleCompare() { ui.value = ui.value.copy(comparing = !ui.value.comparing) }

    /** Typing only compares. A new value drops the "compared digit by digit" tick, so one typo cannot carry an earlier tick into the block. */
    fun onReference(text: String) { ui.value = ui.value.copy(reference = text, confirmed = false, message = null) }

    fun onConfirmed(on: Boolean) { ui.value = ui.value.copy(confirmed = on) }

    /** D17: the block is set only after the tick and the system lock; a cancel or failure sets nothing. */
    fun block() {
        val u = ui.value
        if (u.busy || !u.confirmed || settings.signerMismatch.value) return
        if (compareFingerprint(platform.system.apkCertSha256, u.reference) != FingerprintCompare.Differ) return
        gated { settings.setSignerMismatch(true) }
    }

    /** The way out without a reinstall: the typed value now equals this app's fingerprint, behind the system lock. */
    fun unblock() {
        val u = ui.value
        if (u.busy || !settings.signerMismatch.value) return
        if (compareFingerprint(platform.system.apkCertSha256, u.reference) != FingerprintCompare.Equal) return
        gated { settings.clearSignerMismatch(); ui.value = ui.value.copy(reference = "", comparing = false, confirmed = false) }
    }

    private fun gated(action: () -> Unit) {
        ui.value = ui.value.copy(busy = true, message = null)
        viewModelScope.launch {
            try {
                when (val r = platform.keys.unlock("Confirm the fingerprint check", "Confirm it is you")) {
                    is VaultResult.Secret -> { r.bytes.fill(0); action(); ui.value = ui.value.copy(busy = false) }
                    else -> ui.value = ui.value.copy(busy = false, message = vaultMessage(r))
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                ui.value = ui.value.copy(busy = false, message = e.message ?: "The system lock failed.")
            }
        }
    }

    fun toggleGroup(index: Int) {
        val c = ui.value.checked
        ui.value = ui.value.copy(checked = if (index in c) c - index else c + index)
    }

    fun toggleLicences() { ui.value = ui.value.copy(licencesOpen = !ui.value.licencesOpen) }
}

/** Asks for two random words of the 24 after the system lock. Only those two words are kept, and only until the check passes. */
class BackupVerifyViewModel(
    private val core: IdentityApi, private val platform: Platform, private val settings: SettingsStore,
    random: (Int) -> Int = { kotlin.random.Random.nextInt(it) },
) : ViewModel() {
    private val flow = MutableStateFlow(BackupVerifyState(numbers = pickCheckWords(random)))
    val state: StateFlow<BackupVerifyState> = flow.asStateFlow()
    private var real: List<String> = emptyList()
    private var busy = false

    init { authenticate() }

    fun authenticate() {
        if (busy) return
        busy = true
        flow.value = flow.value.copy(phase = VerifyPhase.Authenticating, message = null)
        viewModelScope.launch {
            try {
                when (val r = platform.keys.unlock("Check the recovery words", "Confirm it is you")) {
                    is VaultResult.Secret -> {
                        r.bytes.fill(0)
                        val chars = core.backupWords()
                        real = flow.value.numbers.map { String(chars[it - 1]) }
                        chars.forEach { it.fill('\u0000') }
                        flow.value = flow.value.copy(phase = VerifyPhase.Ready)
                    }
                    VaultResult.Canceled -> flow.value = flow.value.copy(phase = VerifyPhase.Canceled, message = "Unlock was canceled. The check needs it.")
                    else -> flow.value = flow.value.copy(phase = VerifyPhase.Failed, message = vaultMessage(r))
                }
            } catch (e: CoreException) {
                flow.value = flow.value.copy(phase = VerifyPhase.Failed, message = "The core could not give the words: ${e.message}")
            } finally {
                busy = false
            }
        }
    }

    fun onTyped(index: Int, text: String) {
        val st = flow.value
        flow.value = st.copy(typed = st.typed.mapIndexed { i, t -> if (i == index) text else t }, wrong = st.wrong - st.numbers[index])
    }

    /** Both right: the flag is set for good and `onVerified` runs. Otherwise the wrong numbers are marked. */
    fun check(onVerified: () -> Unit) {
        val st = flow.value
        if (st.phase != VerifyPhase.Ready || real.size != st.numbers.size) return
        val wrong = st.numbers.filterIndexed { i, _ -> !sameWord(st.typed[i], real[i]) }.toSet()
        if (wrong.isNotEmpty()) { flow.value = st.copy(wrong = wrong); return }
        settings.setBackupVerified(true)
        real = emptyList()
        flow.value = st.copy(typed = listOf("", ""), wrong = emptySet())
        onVerified()
    }

    override fun onCleared() { real = emptyList(); flow.value = flow.value.copy(typed = listOf("", "")) }
}
