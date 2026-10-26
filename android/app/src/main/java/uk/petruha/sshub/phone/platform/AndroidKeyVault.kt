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
 * null when the user cancelled, and throws `PromptLockedOut` when the sensor is locked out.
 */
typealias VaultPrompt = suspend (title: String, subtitle: String, cipher: Cipher) -> Cipher?

class PromptLockedOut : Exception()

private fun passThrough(requireAuth: Boolean): VaultPrompt {
    require(!requireAuth) { "a prompt is required when the key needs authentication" }
    return { _, _, c -> c }
}

/** Docs section 4. `secret.bin` = version u8 || iv(12) || AES-GCM(secret) with AAD `AAD`. */
class AndroidKeyVault(
    private val context: Context,
    private val dir: File = context.noBackupFilesDir,
    private val alias: String = ALIAS,
    private val requireAuth: Boolean = true,
    prompt: VaultPrompt? = null,
) : KeyVault {
    private val prompt: VaultPrompt = prompt ?: passThrough(requireAuth)
    private val file = File(dir, FILE)
    private val ks: KeyStore = KeyStore.getInstance(PROVIDER).apply { load(null) }
    private val _state = MutableStateFlow(VaultState.NoKey)
    override val state: StateFlow<VaultState> = _state.asStateFlow()

    init { refresh() }

    override val strongBox: Boolean
        get() {
            val key = secretKey()
            if (key == null) return context.packageManager.hasSystemFeature(PackageManager.FEATURE_STRONGBOX_KEYSTORE)
            return runCatching {
                val info = SecretKeyFactory.getInstance(key.algorithm, PROVIDER).getKeySpec(key, KeyInfo::class.java) as KeyInfo
                if (Build.VERSION.SDK_INT >= 31) info.securityLevel == KeyProperties.SECURITY_LEVEL_STRONGBOX else false
            }.getOrDefault(false)
        }

    /** The app locked (ON_STOP, screen off): the vault needs a prompt again. */
    override fun lock() { if (_state.value == VaultState.Unlocked) _state.value = VaultState.Locked }

    override suspend fun store(secret: ByteArray): VaultResult {
        if (requireAuth && !(context.getSystemService(KeyguardManager::class.java)?.isDeviceSecure ?: false)) return VaultResult.NoSecureLock
        erase()
        try {
            val key = generateKey()
            val cipher = Cipher.getInstance(TRANSFORM)
            cipher.init(Cipher.ENCRYPT_MODE, key)
            val authed: Cipher? = try {
                prompt("Protect the phone key", "", cipher)
            } catch (e: PromptLockedOut) {
                erase(); return VaultResult.LockedOut
            }
            if (authed == null) { erase(); return VaultResult.Canceled }
            authed.updateAAD(AAD)
            val sealed = authed.doFinal(secret)
            atomicWrite(file, byteArrayOf(VERSION) + authed.iv + sealed)
            _state.value = VaultState.Unlocked
            return VaultResult.Stored
        } catch (e: UserNotAuthenticatedException) {
            erase(); return VaultResult.Canceled
        } catch (e: GeneralSecurityException) {
            erase(); return VaultResult.Failed(e.javaClass.simpleName)
        }
    }

    override suspend fun unlock(title: String, subtitle: String): VaultResult {
        val blob = atomicRead(file)
        val key = secretKey()
        if (blob == null || key == null) { _state.value = VaultState.NoKey; return VaultResult.Failed("no key") }
        if (blob.size < 1 + IV_LEN + TAG_LEN || blob[0] != VERSION) return VaultResult.Failed("unknown secret.bin format")
        try {
            val cipher = Cipher.getInstance(TRANSFORM)
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_LEN * 8, blob, 1, IV_LEN))
            val authed: Cipher? = try {
                prompt(title, subtitle, cipher)
            } catch (e: PromptLockedOut) {
                return VaultResult.LockedOut
            }
            if (authed == null) return VaultResult.Canceled
            authed.updateAAD(AAD)
            val secret = authed.doFinal(blob, 1 + IV_LEN, blob.size - 1 - IV_LEN)
            _state.value = VaultState.Unlocked
            return VaultResult.Secret(secret)
        } catch (e: KeyPermanentlyInvalidatedException) {
            _state.value = VaultState.Invalidated
            return VaultResult.Invalidated
        } catch (e: UserNotAuthenticatedException) {
            return VaultResult.Canceled
        } catch (e: GeneralSecurityException) {
            return VaultResult.Failed(e.javaClass.simpleName)
        }
    }

    override suspend fun erase() {
        runCatching { if (ks.containsAlias(alias)) ks.deleteEntry(alias) }
        file.delete()
        File(dir, FILE + ".tmp").delete()
        _state.value = VaultState.NoKey
    }

    private fun secretKey(): SecretKey? = runCatching { ks.getKey(alias, null) as? SecretKey }.getOrNull()

    /** Initial state from disk and Keystore; a key the system invalidated (new biometric) shows up as an init failure. */
    private fun refresh() {
        val blob = atomicRead(file)
        val key = secretKey()
        _state.value = when {
            blob == null || key == null || blob.size < 1 + IV_LEN + TAG_LEN || blob[0] != VERSION -> VaultState.NoKey
            else -> try {
                Cipher.getInstance(TRANSFORM).init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_LEN * 8, blob, 1, IV_LEN))
                VaultState.Locked
            } catch (e: KeyPermanentlyInvalidatedException) {
                VaultState.Invalidated
            } catch (e: GeneralSecurityException) {
                VaultState.Locked
            }
        }
    }

    private fun generateKey(): SecretKey {
        fun spec(strong: Boolean): KeyGenParameterSpec {
            val b = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setKeySize(256)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setRandomizedEncryptionRequired(true)
            if (requireAuth) {
                b.setUserAuthenticationRequired(true)
                b.setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL)
                b.setInvalidatedByBiometricEnrollment(true)
            }
            if (strong) b.setIsStrongBoxBacked(true)
            return b.build()
        }
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
        const val ALIAS = "sshub.phone.secret.v1"
        const val FILE = "secret.bin"
        private const val PROVIDER = "AndroidKeyStore"
        private const val TRANSFORM = "AES/GCM/NoPadding"
        private const val VERSION: Byte = 1
        private const val IV_LEN = 12
        private const val TAG_LEN = 16
        private val AAD = "sshub/phone/secret/v1".toByteArray(Charsets.US_ASCII)
    }
}

/** BiometricPrompt (strong biometric or device credential) around a CryptoObject. Needs a FragmentActivity. */
class BiometricGate(private val activity: FragmentActivity) {
    suspend fun prompt(title: String, subtitle: String, cipher: Cipher): Cipher? = withContext(Dispatchers.Main) {
        suspendCancellableCoroutine { cont ->
            val callback = object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    if (cont.isActive) cont.resume(result.cryptoObject?.cipher)
                }
                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    if (!cont.isActive) return
                    if (errorCode == BiometricPrompt.ERROR_LOCKOUT || errorCode == BiometricPrompt.ERROR_LOCKOUT_PERMANENT) {
                        cont.resumeWithException(PromptLockedOut())
                    } else {
                        cont.resume(null)
                    }
                }
            }
            val bp = BiometricPrompt(activity, ContextCompat.getMainExecutor(activity), callback)
            val info = BiometricPrompt.PromptInfo.Builder()
                .setTitle(title)
                .setSubtitle(subtitle)
                .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL)
                .build()
            cont.invokeOnCancellation { activity.runOnUiThread { bp.cancelAuthentication() } }
            bp.authenticate(info, BiometricPrompt.CryptoObject(cipher))
        }
    }
}
