package uk.petruha.sshub.phone.app

import android.content.Context
import android.content.Intent
import uk.petruha.sshub.phone.core.PhoneCoreApi
import uk.petruha.sshub.phone.core.uniffi.CoreBleLinkFactory
import uk.petruha.sshub.phone.core.uniffi.UniffiPhoneCore
import uk.petruha.sshub.phone.fake.FakePhoneCore
import uk.petruha.sshub.phone.platform.BleLinkFactory

/**
 * Debug builds run the REAL Rust core (the debug APK is what is installed for a device session). The Kotlin fake of the screen
 * groups is used only under instrumentation (`--es core fake` on the test's launch intent): the androidx.test registry class exists
 * only when the test APK is loaded, so a normal launch, even with the extra, always gets the real core.
 */
object CoreFactoryImpl : CoreFactory {
    lateinit var appContext: Context
    private var fake = false
    private val real by lazy { UniffiPhoneCore(appContext) }

    private val underInstrumentation by lazy { runCatching { Class.forName("androidx.test.platform.app.InstrumentationRegistry") }.isSuccess }

    fun configure(intent: Intent?) { fake = underInstrumentation && intent?.getStringExtra("core") == "fake" }
    private val fakeCore by lazy { FakePhoneCore() }
    override fun create(): PhoneCoreApi = if (fake) fakeCore else real
    val bleLinks: BleLinkFactory? get() = if (fake) null else CoreBleLinkFactory(real)
}
