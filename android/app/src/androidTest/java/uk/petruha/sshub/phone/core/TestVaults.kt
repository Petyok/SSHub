package uk.petruha.sshub.phone.core

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** A vault that answers every prompt with a secret (or `result`), counting the prompts and the locks. For tests of the core adapter. */
internal class PassVault(var result: VaultResult? = null) : KeyVault {
    val prompts = mutableListOf<Pair<String, String>>()
    var locks = 0
    override val state: StateFlow<VaultState> = MutableStateFlow(VaultState.Unlocked)
    override val strongBox = false
    override suspend fun store(secret: ByteArray) = VaultResult.Stored
    override suspend fun unlock(title: String, subtitle: String): VaultResult { prompts += title to subtitle; return result ?: VaultResult.Secret(ByteArray(32) { 1 }) }
    override suspend fun erase() {}
    override fun lock() { locks++ }
}
