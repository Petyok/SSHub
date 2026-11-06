package uk.petruha.sshub.phone.platform

import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

/**
 * L4: the screen-off receiver belongs to the process-wide platform (application context), not to an Activity. A real screen off
 * (power key through UiAutomator) must reach it with no Activity alive; before the fix only a running MainActivity registered one.
 */
class ScreenOffReceiverTest {
    private val inst = InstrumentationRegistry.getInstrumentation()
    private val device = UiDevice.getInstance(inst)
    private val locks = AtomicInteger()

    @After fun wake() { device.wakeUp() }

    @Test fun a_real_screen_off_locks_through_the_application_scoped_receiver() {
        AndroidPlatform.shared(inst.targetContext).lockPolicy = LockPolicy({ 60 }, { locks.incrementAndGet() })
        device.wakeUp()
        device.sleep()
        val end = System.currentTimeMillis() + 10_000
        while (locks.get() == 0 && System.currentTimeMillis() < end) Thread.sleep(100)
        assertTrue("the core must be locked when the screen turns off", locks.get() >= 1)
    }
}
