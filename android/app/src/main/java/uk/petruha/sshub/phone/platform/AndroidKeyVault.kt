package uk.petruha.sshub.phone.platform

import android.app.KeyguardManager
import android.content.Context
import android.content.pm.PackageManager
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import android.security.keystore.UserNotAuthenticatedException
import android.os.Build
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import uk.petruha.sshub.phone.core.KeyVault
import uk.petruha.sshub.phone.core.VaultResult
import uk.petruha.sshub.phone.core.VaultState
import java.io.File
import java.io.IOException
import java.security.GeneralSecurityException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.GCMParameterSpec
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/**
 * Shows the authentication prompt for an initialised `cipher` and returns the authenticated cipher,
 * null when the user cancelled, throws `PromptLockedOut` when the sensor is locked out and `PromptFailed`
 * for every other failure that must reach the user with its reason.
 */
typealias VaultPrompt = suspend (title: String, subtitle: String, cipher: Cipher) -> Cipher?

class PromptLockedOut : Exception()

/** A prompt error that is neither a cancel nor a lockout. `code` is a `BiometricPrompt.ERROR_*` value, `text` the system's words. */
class PromptFailed(val code: Int, val text: String) : Exception(text)

/** The one place BiometricPrompt error codes become vault results. Cancels and lockouts keep their own results; the rest keep the system text. */
fun vaultResultForPromptError(code: Int, text: String): VaultResult = when (code) {
    BiometricPrompt.ERROR_USER_CANCELED, BiometricPrompt.ERROR_NEGATIVE_BUTTON, BiometricPrompt.ERROR_CANCELED -> VaultResult.Canceled
    BiometricPrompt.ERROR_LOCKOUT, BiometricPrompt.ERROR_LOCKOUT_PERMANENT -> VaultResult.LockedOut
    BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL -> VaultResult.NoSecureLock
    else -> VaultResult.Failed(text)
}

private fun passThrough(requireAuth: Boolean): VaultPrompt {
    require(!requireAuth) { "a prompt is required when the key needs authentication" }
    return { _, _, c -> c }
}

/**
 * Docs section 4. `secret.bin` is `1 || iv(12) || AES-GCM(secret)` (key under `alias`) or
 * `2 || which(1) || iv(12) || AES-GCM(secret)` (which 0 = `alias`, 1 = `alias.b`), both with AAD `AAD`.
 * The blob names its own key, so one atomic rename of `secret.bin` is the commit point of a replacement and a crash
 * leaves either the old blob with the old key or the new blob with the new key. Keystore keys cannot be renamed,
 * hence two alternating aliases.
 *
 * `store`, `unlock` and `erase` do their Keystore and file IO on `io`; the prompt itself is shown on the main thread by the gate.
 * Every function of `Atomic.kt` blocks and must run on `io` too.
 *
 * Known limit: the Rust core adapter still calls the vault's non-suspend members (`state`, `strongBox`, `lock`) on the main thread,
 * and the initial `refresh` in `init` reads the file and the Keystore synchronously.
 */
class AndroidKeyVault(
    private val context: Context,
    private val dir: File = context.noBackupFilesDir,
    private val alias: String = ALIAS,
    private val requireAuth: Boolean = true,
    prompt: VaultPrompt? = null,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : KeyVault {
    private val prompt: VaultPrompt = prompt ?: passThrough(requireAuth)
    private val file = File(dir, FILE)
    private val newFile = File(dir, "$FILE.new")
    private val altAlias = "$alias.b"
    private val ks: KeyStore = KeyStore.getInstance(PROVIDER).apply { load(null) }
    private val _state = MutableStateFlow(VaultState.NoKey)
    override val state: StateFlow<VaultState> = _state.asStateFlow()

    init { refresh() }

    override val strongBox: Boolean
        get() {
            val key = secretKey(atomicRead(file))
            if (key == null) return context.packageManager.hasSystemFeature(PackageManager.FEATURE_STRONGBOX_KEYSTORE)
            return runCatching {
                val info = SecretKeyFactory.getInstance(key.algorithm, PROVIDER).getKeySpec(key, KeyInfo::class.java) as KeyInfo
                if (Build.VERSION.SDK_INT >= 31) info.securityLevel == KeyProperties.SECURITY_LEVEL_STRONGBOX else false
            }.getOrDefault(false)
        }

    /** The app locked (ON_STOP, screen off): the vault needs a prompt again. */
    override fun lock() { if (_state.value == VaultState.Unlocked) _state.value = VaultState.Locked }

    override suspend fun store(secret: ByteArray): VaultResult = withContext(io) {
        if (requireAuth && !(context.getSystemService(KeyguardManager::class.java)?.isDeviceSecure ?: false)) return@withContext VaultResult.NoSecureLock
        val old = liveAlias(atomicRead(file))
        val fresh = if (old == alias && file.isFile) altAlias else alias
        var committed = false
        try {
            // Leftovers of an earlier crash under the alias we are about to use are not live: the blob names the live one.
            deleteKey(fresh); newFile.delete()
            val key = generateKey(fresh)
            val cipher = Cipher.getInstance(TRANSFORM)
            cipher.init(Cipher.ENCRYPT_MODE, key)
            val authed: Cipher = try {
                prompt("Protect the phone key", "", cipher) ?: return@withContext VaultResult.Canceled
            } catch (e: PromptLockedOut) {
                return@withContext VaultResult.LockedOut
            } catch (e: PromptFailed) {
                return@withContext vaultResultForPromptError(e.code, e.text)
            }
            authed.updateAAD(AAD)
            val sealed = authed.doFinal(secret)
            val header = if (fresh == alias) byteArrayOf(VERSION) else byteArrayOf(VERSION2, 1)
            atomicWrite(newFile, header + authed.iv + sealed)
            atomicMove(newFile, file)
            committed = true
            if (old != fresh) deleteKey(old)
            _state.value = VaultState.Unlocked
            VaultResult.Stored
        } catch (e: UserNotAuthenticatedException) {
            VaultResult.Canceled
        } catch (e: GeneralSecurityException) {
            VaultResult.Failed(e.javaClass.simpleName)
        } catch (e: IOException) {
            VaultResult.Failed(e.javaClass.simpleName)
        } finally {
            // Only the new key and its temp file go; the old key, blob and state stay untouched on every earlier exit.
            if (!committed) { deleteKey(fresh); newFile.delete(); File(dir, "$FILE.new.tmp").delete() }
        }
    }

    override suspend fun unlock(title: String, subtitle: String): VaultResult = withContext(io) {
        val blob = atomicRead(file)
        if (blob == null) { _state.value = VaultState.NoKey; return@withContext VaultResult.Failed("no key") }
        val hdr = headerLength(blob) ?: return@withContext VaultResult.Failed("unknown secret.bin format")
        // The blob is fine but its key is gone (Android dropped it): a different state from "never set up".
        val key = secretKey(blob)
        if (key == null) { _state.value = VaultState.Invalidated; return@withContext VaultResult.Invalidated }
        try {
            val cipher = Cipher.getInstance(TRANSFORM)
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_LEN * 8, blob, hdr, IV_LEN))
            val authed: Cipher = try {
                prompt(title, subtitle, cipher) ?: return@withContext VaultResult.Canceled
            } catch (e: PromptLockedOut) {
                return@withContext VaultResult.LockedOut
            } catch (e: PromptFailed) {
                return@withContext vaultResultForPromptError(e.code, e.text)
            }
            authed.updateAAD(AAD)
            val secret = authed.doFinal(blob, hdr + IV_LEN, blob.size - hdr - IV_LEN)
            _state.value = VaultState.Unlocked
            VaultResult.Secret(secret)
        } catch (e: KeyPermanentlyInvalidatedException) {
            _state.value = VaultState.Invalidated
            VaultResult.Invalidated
        } catch (e: UserNotAuthenticatedException) {
            VaultResult.Canceled
        } catch (e: GeneralSecurityException) {
            VaultResult.Failed(e.javaClass.simpleName)
        }
    }

    override suspend fun erase() = withContext(io) {
        deleteKey(alias); deleteKey(altAlias)
        file.delete()
        File(dir, "$FILE.tmp").delete()
        newFile.delete()
        File(dir, "$FILE.new.tmp").delete()
        _state.value = VaultState.NoKey
    }

    private fun deleteKey(a: String) { runCatching { if (ks.containsAlias(a)) ks.deleteEntry(a) } }

    /** Bytes before the IV, or null when the blob is not a secret.bin this version can read. */
    private fun headerLength(blob: ByteArray): Int? {
        val h = when {
            blob.size >= 2 && blob[0] == VERSION2 && (blob[1] == 0.toByte() || blob[1] == 1.toByte()) -> 2
            blob.isNotEmpty() && blob[0] == VERSION -> 1
            else -> return null
        }
        return if (blob.size < h + IV_LEN + TAG_LEN) null else h
    }

    /** The alias the blob names; `alias` for version 1 files, for an absent file and for anything unreadable. */
    private fun liveAlias(blob: ByteArray?): String =
        if (blob != null && headerLength(blob) == 2 && blob[1] == 1.toByte()) altAlias else alias

    private fun secretKey(blob: ByteArray?): SecretKey? = runCatching { ks.getKey(liveAlias(blob), null) as? SecretKey }.getOrNull()

    /** Initial state from disk and Keystore; a key the system invalidated (new biometric) shows up as an init failure. */
    private fun refresh() {
        val blob = atomicRead(file)
        val hdr = blob?.let { headerLength(it) }
        val key = secretKey(blob)
        _state.value = when {
            blob == null || hdr == null -> VaultState.NoKey
            key == null -> VaultState.Invalidated
            else -> try {
                Cipher.getInstance(TRANSFORM).init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_LEN * 8, blob, hdr, IV_LEN))
                VaultState.Locked
            } catch (e: KeyPermanentlyInvalidatedException) {
                VaultState.Invalidated
            } catch (e: GeneralSecurityException) {
                VaultState.Locked
            }
        }
    }

    private fun generateKey(keyAlias: String): SecretKey {
        fun spec(strong: Boolean) = keySpec(keyAlias, strong, requireAuth)
        val kg = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER)
        if (context.packageManager.hasSystemFeature(PackageManager.FEATURE_STRONGBOX_KEYSTORE)) {
            try {
                kg.init(spec(true))
                return kg.generateKey()
            } catch (e: StrongBoxUnavailableException) {
                // fall through to the TEE
            }
        }
        kg.init(spec(false))
        return kg.generateKey()
    }

    companion object {
        /**
         * The key spec. Per-use authentication stays (strong biometric or the device credential, bound to a CryptoObject), but adding a
         * fingerprint does NOT invalidate the key: an ordinary Android action must not destroy the phone's share (UX review U13-6). The
         * device-credential fallback already bounds what that invalidation protected (residual M6). The key is still lost when the
         * secure screen lock is removed, which Android enforces and the app cannot prevent.
         */
        internal fun keySpec(keyAlias: String, strong: Boolean, requireAuth: Boolean): KeyGenParameterSpec {
            val b = KeyGenParameterSpec.Builder(keyAlias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setKeySize(256)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setRandomizedEncryptionRequired(true)
            if (requireAuth) {
                b.setUserAuthenticationRequired(true)
                b.setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL)
                b.setInvalidatedByBiometricEnrollment(false)
            }
            if (strong) b.setIsStrongBoxBacked(true)
            return b.build()
        }

        const val ALIAS = "sshub.phone.secret.v1"
        const val FILE = "secret.bin"
        private const val PROVIDER = "AndroidKeyStore"
        private const val TRANSFORM = "AES/GCM/NoPadding"
        private const val VERSION: Byte = 1
        private const val VERSION2: Byte = 2
        private const val IV_LEN = 12
        private const val TAG_LEN = 16
        private val AAD = "sshub/phone/secret/v1".toByteArray(Charsets.US_ASCII)
    }
}

/** BiometricPrompt (strong biometric or device credential) around a CryptoObject. Needs a FragmentActivity. */
class BiometricGate(private val activity: FragmentActivity) {
    private val authenticators = BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL

    /** Fails before any prompt when the device cannot authenticate at all, with a text that names the actual reason. */
    private fun checkAvailable() {
        when (val r = BiometricManager.from(activity).canAuthenticate(authenticators)) {
            BiometricManager.BIOMETRIC_SUCCESS -> Unit
            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED, BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE ->
                throw PromptFailed(BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL, "This phone has no screen lock or enrolled fingerprint.")
            BiometricManager.BIOMETRIC_ERROR_HW_UNAVAILABLE ->
                throw PromptFailed(BiometricPrompt.ERROR_HW_UNAVAILABLE, "The biometric sensor is unavailable right now.")
            BiometricManager.BIOMETRIC_ERROR_SECURITY_UPDATE_REQUIRED ->
                throw PromptFailed(BiometricPrompt.ERROR_HW_UNAVAILABLE, "A security update is required before this phone can unlock.")
            else -> throw PromptFailed(BiometricPrompt.ERROR_HW_UNAVAILABLE, "This phone cannot authenticate here (status $r).")
        }
    }

    suspend fun prompt(title: String, subtitle: String, cipher: Cipher): Cipher? = withContext(Dispatchers.Main) {
        checkAvailable()
        suspendCancellableCoroutine { cont ->
            val callback = object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    if (cont.isActive) cont.resume(result.cryptoObject?.cipher)
                }
                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    if (!cont.isActive) return
                    when (vaultResultForPromptError(errorCode, errString.toString())) {
                        VaultResult.Canceled -> cont.resume(null)
                        VaultResult.LockedOut -> cont.resumeWithException(PromptLockedOut())
                        else -> cont.resumeWithException(PromptFailed(errorCode, errString.toString()))
                    }
                }
            }
            val bp = BiometricPrompt(activity, ContextCompat.getMainExecutor(activity), callback)
            val info = BiometricPrompt.PromptInfo.Builder()
                .setTitle(title)
                .setSubtitle(subtitle)
                .setAllowedAuthenticators(authenticators)
                .build()
            cont.invokeOnCancellation { activity.runOnUiThread { bp.cancelAuthentication() } }
            bp.authenticate(info, BiometricPrompt.CryptoObject(cipher))
        }
    }
}
