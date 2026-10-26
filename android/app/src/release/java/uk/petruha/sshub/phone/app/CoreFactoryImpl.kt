package uk.petruha.sshub.phone.app

import android.content.Context
import uk.petruha.sshub.phone.core.PhoneCoreApi
import uk.petruha.sshub.phone.core.uniffi.CoreBleLinkFactory
import uk.petruha.sshub.phone.core.uniffi.UniffiPhoneCore
import uk.petruha.sshub.phone.platform.BleLinkFactory

/** Release builds run the Rust core through uniffi. `appContext` is set by `MainActivity` before the first `create()`. */
object CoreFactoryImpl : CoreFactory {
    lateinit var appContext: Context

    /** Release builds have no demo mode. */
    fun configure(intent: android.content.Intent?) {}

    private val core by lazy { UniffiPhoneCore(appContext) }

    override fun create(): PhoneCoreApi = core

    /** Bluetooth links over the same core: `AndroidPlatform(context, CoreFactoryImpl.bleLinks)`. */
    val bleLinks: BleLinkFactory by lazy { CoreBleLinkFactory(core) }
}
