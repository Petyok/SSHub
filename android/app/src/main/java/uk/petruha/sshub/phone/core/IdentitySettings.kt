package uk.petruha.sshub.phone.core

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

enum class ThemeChoice { System, Dark, Light }

/** Auto-lock after the app leaves the foreground: 30 s to 5 min (spec 11, default 1 min). */
object AutoLockChoices {
    val seconds: List<Int> = listOf(30, 60, 120, 300)
    const val DEFAULT = 60
}

/**
 * Small user settings and two flags other screens read. The default is in memory; the app shell may pass a persistent implementation
 * to `identityGraph`. `signerMismatch` is the D17 block: while it is true every flow except About and Lock must stay closed.
 */
interface SettingsStore {
    val theme: StateFlow<ThemeChoice>
    val autoLockSeconds: StateFlow<Int>
    /** The user ticked "I wrote them on paper" at least once. */
    val backupConfirmed: StateFlow<Boolean>
    /** The user typed two random words of the 24 correctly. Sticky: only `setBackupVerified(true)` changes it (the verify screen, or a restore where all 24 were typed). Until then no approval screen opens. */
    val backupVerified: StateFlow<Boolean>
    /** Name of the device whose removal left this phone's shares out of date, or null. Another screen reads it to ask for an update. */
    val updateNeededAfter: StateFlow<String?>
    /** The app signing fingerprint was compared with the terminal's value and differs. Sticky: only `clearSignerMismatch`, after a matching compare behind the system lock, clears it; reinstalling also does. `setSignerMismatch(false)` does nothing. */
    val signerMismatch: StateFlow<Boolean>

    fun setTheme(choice: ThemeChoice)
    fun setAutoLock(seconds: Int)
    fun setBackupConfirmed(confirmed: Boolean)
    /** `false` does nothing: the flag never goes back. */
    fun setBackupVerified(v: Boolean)
    /** A name (spaces allowed, newlines are replaced by spaces) or null / blank to clear. */
    fun setUpdateNeededAfter(deviceName: String?)
    fun setSignerMismatch(mismatch: Boolean)
    /** The only way to lower the block without a reinstall. The caller must have seen an equal compare and passed the system lock. */
    fun clearSignerMismatch()
}

/** Shared by both stores: one line, trimmed, blank is null. */
internal fun cleanDeviceName(name: String?): String? = name?.replace(Regex("[\\r\\n]+"), " ")?.trim()?.takeIf { it.isNotEmpty() }

class InMemorySettingsStore(verified: Boolean = false) : SettingsStore {
    private val themeFlow = MutableStateFlow(ThemeChoice.Dark)
    private val autoLockFlow = MutableStateFlow(AutoLockChoices.DEFAULT)
    private val backupFlow = MutableStateFlow(false)
    private val verifiedFlow = MutableStateFlow(verified)
    private val updateFlow = MutableStateFlow<String?>(null)
    private val mismatchFlow = MutableStateFlow(false)
    override val theme: StateFlow<ThemeChoice> get() = themeFlow.asStateFlow()
    override val autoLockSeconds: StateFlow<Int> get() = autoLockFlow.asStateFlow()
    override val backupConfirmed: StateFlow<Boolean> get() = backupFlow.asStateFlow()
    override val backupVerified: StateFlow<Boolean> get() = verifiedFlow.asStateFlow()
    override val updateNeededAfter: StateFlow<String?> get() = updateFlow.asStateFlow()
    override val signerMismatch: StateFlow<Boolean> get() = mismatchFlow.asStateFlow()
    override fun setTheme(choice: ThemeChoice) { themeFlow.value = choice }
    override fun setAutoLock(seconds: Int) { autoLockFlow.value = seconds.coerceIn(AutoLockChoices.seconds.first(), AutoLockChoices.seconds.last()) }
    override fun setBackupConfirmed(confirmed: Boolean) { backupFlow.value = confirmed }
    override fun setBackupVerified(v: Boolean) { if (v) verifiedFlow.value = true }
    override fun setUpdateNeededAfter(deviceName: String?) { updateFlow.value = cleanDeviceName(deviceName) }
    override fun setSignerMismatch(mismatch: Boolean) { if (mismatch) mismatchFlow.value = true } // sticky, like the persistent store
    override fun clearSignerMismatch() { mismatchFlow.value = false }
}

/** Process-wide default used by `identityGraph` when the shell passes nothing. */
object AppSettings : SettingsStore by InMemorySettingsStore()
