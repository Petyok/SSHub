package uk.petruha.sshub.phone.app

import android.content.Intent
import android.view.View
import android.view.WindowManager
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertTrue
import org.junit.Test
import uk.petruha.sshub.phone.MainActivity

class WindowHardeningTest {
    @Test
    fun mainActivityIsSecureAndDropsObscuredTouches() {
        val ctx = InstrumentationRegistry.getInstrumentation().targetContext
        ActivityScenario.launch<MainActivity>(Intent(ctx, MainActivity::class.java)).use { scenario ->
            scenario.onActivity { a ->
                assertTrue("FLAG_SECURE", a.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
                assertTrue("filterTouchesWhenObscured", a.findViewById<View>(android.R.id.content).filterTouchesWhenObscured)
            }
        }
    }
}
