package uk.petruha.sshub.phone.ui.screens.identity

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import uk.petruha.sshub.phone.core.ChainCheck
import uk.petruha.sshub.phone.core.CoreException
import uk.petruha.sshub.phone.core.IdentityApi
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.OpenedChain
import uk.petruha.sshub.phone.core.OpenedRefused
import uk.petruha.sshub.phone.core.Platform
import uk.petruha.sshub.phone.core.ScanApi
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

class RestoreViewModel(private val core: IdentityApi, private val platform: Platform, replaces: Boolean) : ViewModel() {
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
    private val flow = MutableStateFlow(ChainHeadState(deviceId = core.status(platform.clock.nowSeconds()).deviceId))
    val state: StateFlow<ChainHeadState> = flow.asStateFlow()

    /** A result of the scan screen or of the paste box: every list goes through `openFrame`, the core says what it is. */
    fun onOpened(opened: Opened) {
        val st = flow.value
        flow.value = when (opened) {
            is OpenedChain -> st.copy(
                phase = ChainHeadPhase.Read, lists = if (opened.chain.agreed) 2 else 1, chain = opened.chain, checks = opened.checks,
                notice = null, refusal = null, pasteText = "", busy = false,
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
        if (!chain.agreed || flow.value.busy) return
        flow.value = flow.value.copy(busy = true)
        viewModelScope.launch {
            try {
                val taken = core.adoptChain(chain.token, platform.clock.nowSeconds())
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
        try {
            core.neverSigned(st.typedId, platform.clock.nowSeconds())
            flow.value = st.copy(phase = ChainHeadPhase.Taken, neverSigned = true, takenSeq = null, typedId = "")
        } catch (e: CoreException) {
            flow.value = st.copy(notice = "Not confirmed: ${e.message}")
        }
    }

    /** Back to reading lists after a refusal. */
    fun tryAnother() {
        val st = flow.value
        flow.value = st.copy(phase = ChainHeadPhase.Read, refusal = null, checks = if (st.lists > 0) st.checks else emptyList(), notice = null)
    }
}

class SecurityMenuViewModel(private val core: IdentityApi, private val platform: Platform, private val settings: SettingsStore) : ViewModel() {
    private val status = core.status(platform.clock.nowSeconds())
    private val headPending = MutableStateFlow(status.headPending)

    val state: StateFlow<SecurityMenuState> = combine(
        settings.theme, settings.autoLockSeconds, settings.backupConfirmed, settings.signerMismatch, headPending,
    ) { theme, autoLock, backup, blocked, pending ->
        SecurityMenuState(status.deviceName, status.deviceId, theme, autoLock, backup, blocked, pending)
    }.stateIn(
        viewModelScope, SharingStarted.Eagerly,
        SecurityMenuState(
            status.deviceName, status.deviceId, settings.theme.value, settings.autoLockSeconds.value,
            settings.backupConfirmed.value, settings.signerMismatch.value, status.headPending,
        ),
    )

    /** The chain may have been taken since this model was made (Take chain, then back to the menu). */
    fun refresh() { headPending.value = core.status(platform.clock.nowSeconds()).headPending }

    fun setTheme(choice: ThemeChoice) = settings.setTheme(choice)
    fun setAutoLock(seconds: Int) = settings.setAutoLock(seconds)

    /** Wipes the secret from the core; the graph then shows the lock screen. */
    fun lock() = core.lock()
}

private data class AboutUi(val reference: String = "", val comparing: Boolean = false, val checked: Set<Int> = emptySet(), val licencesOpen: Boolean = false)

class AboutViewModel(private val core: IdentityApi, private val platform: Platform, private val settings: SettingsStore) : ViewModel() {
    private val status = core.status(platform.clock.nowSeconds())
    private val ui = MutableStateFlow(AboutUi())
    private val licences: List<LicenceText> by lazy {
        listOf(
            LicenceText("JetBrains Mono 2.304", "Copyright 2020 The JetBrains Mono Project Authors", "https://github.com/JetBrains/JetBrainsMono", platform.system.licenceText("JetBrainsMono-OFL.txt")),
            LicenceText("Inter 4.1", "Copyright (c) 2016 The Inter Project Authors", "https://github.com/rsms/inter", platform.system.licenceText("Inter-LICENSE.txt")),
        )
    }

    private fun build(u: AboutUi, autoLock: Int, backup: Boolean) = AboutState(
        versionName = platform.system.versionName, deviceName = status.deviceName, deviceId = status.deviceId,
        fingerprint = platform.system.apkCertSha256.lowercase(), reference = u.reference, comparing = u.comparing, checked = u.checked,
        autoLockSeconds = autoLock, strongBox = platform.keys.strongBox, backupConfirmed = backup,
        licencesOpen = u.licencesOpen, licences = if (u.licencesOpen) licences else emptyList(),
    )

    val state: StateFlow<AboutState> = combine(ui, settings.autoLockSeconds, settings.backupConfirmed) { u, a, b -> build(u, a, b) }
        .stateIn(viewModelScope, SharingStarted.Eagerly, build(ui.value, settings.autoLockSeconds.value, settings.backupConfirmed.value))

    fun toggleCompare() { ui.value = ui.value.copy(comparing = !ui.value.comparing) }

    /** D17: a typed value that differs sets the app-wide block, an equal or shorter one clears it. */
    fun onReference(text: String) {
        ui.value = ui.value.copy(reference = text)
        settings.setSignerMismatch(compareFingerprint(platform.system.apkCertSha256, text) == FingerprintCompare.Differ)
    }

    fun toggleGroup(index: Int) {
        val c = ui.value.checked
        ui.value = ui.value.copy(checked = if (index in c) c - index else c + index)
    }

    fun toggleLicences() { ui.value = ui.value.copy(licencesOpen = !ui.value.licencesOpen) }
}
