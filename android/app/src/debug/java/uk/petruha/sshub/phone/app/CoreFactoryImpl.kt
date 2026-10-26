package uk.petruha.sshub.phone.app

import android.content.Context
import uk.petruha.sshub.phone.core.PhoneCoreApi
import uk.petruha.sshub.phone.fake.FakePhoneCore
import uk.petruha.sshub.phone.platform.BleLinkFactory

/** Debug builds run the Kotlin fake; release builds run the Rust core (src/release). Same surface so MainActivity is shared. */
object CoreFactoryImpl : CoreFactory {
    lateinit var appContext: Context
    override fun create(): PhoneCoreApi = FakePhoneCore()
    val bleLinks: BleLinkFactory? = null
}
