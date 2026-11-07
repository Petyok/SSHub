package uk.petruha.sshub.phone.platform

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import uk.petruha.sshub.phone.core.AutoLockChoices
import uk.petruha.sshub.phone.core.SettingsStore
import uk.petruha.sshub.phone.core.ThemeChoice
import java.io.File

/**
 * The settings and the D17 block in one small file (`key=value` lines) in `noBackupFilesDir`, replaced atomically on every change.
 * `signerMismatch` is sticky: once set it stays set across restarts until `clearSignerMismatch` or a reinstall (the file goes with it).
 */
class FileSettingsStore(private val file: File) : SettingsStore {
    private val values: Map<String, String> = atomicRead(file)?.toString(Charsets.UTF_8)?.lineSequence()
        ?.mapNotNull { l -> l.indexOf('=').takeIf { it > 0 }?.let { l.substring(0, it) to l.substring(it + 1) } }?.toMap() ?: emptyMap()

    private val themeFlow = MutableStateFlow(values["theme"]?.let { v -> ThemeChoice.entries.firstOrNull { it.name == v } } ?: ThemeChoice.Dark)
    private val autoLockFlow = MutableStateFlow(values["autolock"]?.toIntOrNull()?.takeIf { it in AutoLockChoices.seconds } ?: AutoLockChoices.DEFAULT)
    private val backupFlow = MutableStateFlow(values["backup"] == "1")
    private val mismatchFlow = MutableStateFlow(values["mismatch"] == "1")

    override val theme: StateFlow<ThemeChoice> get() = themeFlow.asStateFlow()
    override val autoLockSeconds: StateFlow<Int> get() = autoLockFlow.asStateFlow()
    override val backupConfirmed: StateFlow<Boolean> get() = backupFlow.asStateFlow()
    override val signerMismatch: StateFlow<Boolean> get() = mismatchFlow.asStateFlow()

    @Synchronized private fun save() {
        val text = "theme=${themeFlow.value.name}\nautolock=${autoLockFlow.value}\nbackup=${if (backupFlow.value) 1 else 0}\nmismatch=${if (mismatchFlow.value) 1 else 0}\n"
        atomicWrite(file, text.toByteArray(Charsets.UTF_8))
    }

    override fun setTheme(choice: ThemeChoice) { themeFlow.value = choice; save() }
    override fun setAutoLock(seconds: Int) { autoLockFlow.value = seconds.coerceIn(AutoLockChoices.seconds.first(), AutoLockChoices.seconds.last()); save() }
    override fun setBackupConfirmed(confirmed: Boolean) { backupFlow.value = confirmed; save() }
    override fun setSignerMismatch(mismatch: Boolean) { if (!mismatch || mismatchFlow.value) return; mismatchFlow.value = true; save() }
    override fun clearSignerMismatch() { if (!mismatchFlow.value) return; mismatchFlow.value = false; save() }

    companion object {
        const val FILE = "settings.txt"
        @Volatile private var instance: FileSettingsStore? = null
        fun shared(context: Context): FileSettingsStore =
            instance ?: synchronized(this) { instance ?: FileSettingsStore(File(context.applicationContext.noBackupFilesDir, FILE)).also { instance = it } }
    }
}
