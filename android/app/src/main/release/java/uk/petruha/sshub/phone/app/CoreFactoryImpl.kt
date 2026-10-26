package uk.petruha.sshub.phone.app

import uk.petruha.sshub.phone.core.PhoneCoreApi

/** Replaced by the uniffi adapter when crates/sshub-phone-core is merged; until then a release build refuses to start instead of faking a core. */
object CoreFactoryImpl : CoreFactory {
    override fun create(): PhoneCoreApi = throw IllegalStateException("the native phone core is not part of this build")
}
