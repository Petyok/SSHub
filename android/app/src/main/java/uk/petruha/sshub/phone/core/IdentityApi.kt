package uk.petruha.sshub.phone.core

/**
 * Identity group of the core (docs/host-sync-android.md sections 2.1, 4, 5): the 32-byte secret, the 24 words, unlock and lock, and head adoption.
 *
 * Rust owns every decision: the word list and checksum, which ops are accepted, what "never signed" means. Kotlin only moves bytes between the
 * Keystore (`KeyVault`) and the core, and renders what comes back. `now` is unix seconds from `platform.clock`.
 *
 * Errors are `CoreException`: `Malformed` (bad words, bad typed id), `Refused` (a list that does not verify), `Internal`.
 */
interface IdentityApi {
    /** What the core knows after unlock; drives the red "head pending" card (C21). */
    fun status(now: Long): PhoneStatus

    /** 32 CSPRNG bytes. The caller wraps them with `KeyVault.store`, hands them to `unlock` and zeroes them. */
    fun createSecret(): ByteArray

    /** Restore on a fresh store: returns the secret to wrap, sets `headPending`. Throws `Malformed` for a wrong word or checksum. */
    fun restoreFromWords(words: List<String>): ByteArray

    /** Word-list check of one typed word, for the restore grid. The list lives in the core, never in Kotlin. */
    fun validateWord(word: String): WordState

    /** The 24 words, one `CharArray` each. The caller zeroes every array as soon as it has drawn them (spec 11: never kept, never logged). */
    fun backupWords(): List<CharArray>

    /** Unlocks the core with the secret the Keystore released. The caller zeroes `secret` afterwards. */
    fun unlock(secret: ByteArray, now: Long): PhoneStatus

    /** Wipes the secret and every derived key from the core's memory. */
    fun lock()

    /** Only for a key that never signed anything: the user typed the device id, the core checks it. Clears `headPending`. Throws `Malformed` on a mismatch. */
    suspend fun neverSigned(typedId: String, now: Long): PhoneStatus

    /** True after an unlock whose sealed state could not be read: every flow stays closed until `confirmStateLost`. */
    fun stateLost(): Boolean = false

    /** The user read the stop screen: the empty state replaces the unreadable one (a copy stays as `state.bin.lost`). */
    fun confirmStateLost() {}

    /**
     * The user accepts the chain head of an `OpenedChain` (the core's `decide(token, Decision.Approve)`). Returns only after the core
     * adopted it; `headPending` is false afterwards.
     */
    suspend fun adoptChain(token: Long, now: Long): ChainTaken
}
