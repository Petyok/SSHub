package io.github.petyok.sshub.release

import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import io.github.petyok.sshub.core.InMemorySettingsStore
import io.github.petyok.sshub.core.RedField
import io.github.petyok.sshub.fake.FakePlatform
import io.github.petyok.sshub.fake.FakeReleaseApi
import io.github.petyok.sshub.fake.ReleaseSamples
import io.github.petyok.sshub.ui.screens.release.RedViewModel
import io.github.petyok.sshub.ui.screens.release.removedDeviceOf

/** U13-23: approving the removal of a device leaves this phone out of the new sharing: Home must say so. */
class UpdateMarkerTest {
    private val inst = InstrumentationRegistry.getInstrumentation()

    @Test fun the_removed_device_is_read_from_the_device_id_leaf() {
        val v = ReleaseSamples.redRevoke.copy(fields = listOf(RedField("device_id", null, "723ed85f0566df0c")))
        assertEquals("723ed85f0566df0c", removedDeviceOf(v))
        assertEquals("a device", removedDeviceOf(v.copy(fields = emptyList())))
    }

    @Test fun approving_a_revoke_sets_the_marker_and_other_ops_do_not() {
        val settings = InMemorySettingsStore()
        val core = FakeReleaseApi()
        core.views[ReleaseSamples.TOKEN_REVOKE] = io.github.petyok.sshub.core.OpenedRed(
            ReleaseSamples.redRevoke.copy(kind = "DeviceRevoke", fields = listOf(RedField("device_id", null, "723ed85f0566df0c"))),
        )
        lateinit var vm: RedViewModel
        inst.runOnMainSync { vm = RedViewModel(core, FakePlatform(), ReleaseSamples.TOKEN_REVOKE, settings) }
        assertNull(settings.updateNeededAfter.value)
        // the sample's gate may need the typed id; reaching approve() is enough through the view model API
        inst.runOnMainSync { vm.typed(ReleaseSamples.redRevoke.mustType.orEmpty()); vm.approve() }
        val end = System.currentTimeMillis() + 5_000
        while (settings.updateNeededAfter.value == null && System.currentTimeMillis() < end) Thread.sleep(25)
        assertEquals("723ed85f0566df0c", settings.updateNeededAfter.value)

        val other = InMemorySettingsStore()
        val core2 = FakeReleaseApi()
        lateinit var vm2: RedViewModel
        inst.runOnMainSync { vm2 = RedViewModel(core2, FakePlatform(), ReleaseSamples.TOKEN_MODE, other) }
        inst.runOnMainSync { vm2.typed(ReleaseSamples.redMode.mustType.orEmpty()); vm2.approve() }
        Thread.sleep(500)
        assertNull(other.updateNeededAfter.value)
    }
}
