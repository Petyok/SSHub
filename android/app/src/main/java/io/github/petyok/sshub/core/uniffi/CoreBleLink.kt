package io.github.petyok.sshub.core.uniffi

import android.os.SystemClock
import io.github.petyok.sshub.platform.BleLink
import io.github.petyok.sshub.platform.BleLinkFactory
import uniffi.sshub_phone_core.BleSession as RsBleSession
import uniffi.sshub_phone_core.BleState as RsBleState
import uniffi.sshub_phone_core.CoreException as RsCoreException

/**
 * [BleLink] over the sans-IO `BleSession` of the Rust core (contract C20, phone role). The session is started at once: HELLO is the
 * first message of `takeOutbox`. Every call carries a monotonic clock in milliseconds; `tick` passes the caller's value, the others read
 * `SystemClock.elapsedRealtime()`, the clock `AndroidBle` ticks with.
 */
class CoreBleLink(private val session: RsBleSession, private val clock: () -> Long = SystemClock::elapsedRealtime) : BleLink {
    init { session.start(clock().toULong()) }

    override fun onMessage(b: ByteArray) = session.onMessage(b, clock().toULong())

    override fun takeOutbox(): List<ByteArray> = session.takeOutbox()

    /** False when the session is not open or the frame is too large; the core's sentence is in [sendFailure] only for a refusal by the terminal. */
    override fun sendFrame(f: ByteArray): Boolean = try {
        session.sendFrame(f, clock().toULong())
        true
    } catch (_: RsCoreException) {
        false
    }

    override fun takeFrames(): List<ByteArray> = session.takeFrames()

    override fun tick(nowMillis: Long) = session.tick(nowMillis.toULong())

    override val closed: Boolean get() = session.state() is RsBleState.Closed

    /** Why the last frame was refused or never acknowledged, once. */
    override fun sendFailure(): String? = session.takeSendFailure()

    /** The core's sentence for the end of the session (BYE or a timer), once [closed] is true. */
    override val closedReason: String? get() = (session.state() as? RsBleState.Closed)?.reason

    /** The page after its last step: BYE and done. The BYE is in the next `takeOutbox`. `BleLink` has no `bye` yet (docs 2.3). */
    override fun bye() = session.bye(clock().toULong())
}

/** The links of the one core of this process (`AndroidPlatform(context, CoreBleLinkFactory(core))`). */
class CoreBleLinkFactory(private val core: UniffiPhoneCore) : BleLinkFactory {
    override fun create(mtu: Int): BleLink = CoreBleLink(core.newBleSession(mtu))
}
