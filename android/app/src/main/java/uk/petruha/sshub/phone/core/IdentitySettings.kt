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
    /** The app signing fingerprint was compared with the terminal's value and differs. */
    val signerMismatch: StateFlow<Boolean>

    fun setTheme(choice: ThemeChoice)
    fun setAutoLock(seconds: Int)
    fun setBackupConfirmed(confirmed: Boolean)
    fun setSignerMismatch(mismatch: Boolean)
}

class InMemorySettingsStore : SettingsStore {
    private val themeFlow = MutableStateFlow(ThemeChoice.Dark)
    private val autoLockFlow = MutableStateFlow(AutoLockChoices.DEFAULT)
    private val backupFlow = MutableStateFlow(false)
    private val mismatchFlow = MutableStateFlow(false)
    override val theme: StateFlow<ThemeChoice> get() = themeFlow.asStateFlow()
    override val autoLockSeconds: StateFlow<Int> get() = autoLockFlow.asStateFlow()
    override val backupConfirmed: StateFlow<Boolean> get() = backupFlow.asStateFlow()
    override val signerMismatch: StateFlow<Boolean> get() = mismatchFlow.asStateFlow()
    override fun setTheme(choice: ThemeChoice) { themeFlow.value = choice }
    override fun setAutoLock(seconds: Int) { autoLockFlow.value = seconds.coerceIn(AutoLockChoices.seconds.first(), AutoLockChoices.seconds.last()) }
    override fun setBackupConfirmed(confirmed: Boolean) { backupFlow.value = confirmed }
    override fun setSignerMismatch(mismatch: Boolean) { mismatchFlow.value = mismatch }
}

/** Process-wide default used by `identityGraph` when the shell passes nothing. */
object AppSettings : SettingsStore by InMemorySettingsStore()
