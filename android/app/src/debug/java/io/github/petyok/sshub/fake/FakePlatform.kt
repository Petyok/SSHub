package io.github.petyok.sshub.fake

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.flowOf
import io.github.petyok.sshub.core.*
import io.github.petyok.sshub.platform.BleChannelState
import io.github.petyok.sshub.platform.BleScanError
import io.github.petyok.sshub.platform.ObservedBleChannel

/** Deterministic in-memory Platform for Compose tests and the debug app. Every port exposes its state as a MutableStateFlow the test can drive. */
class FakePlatform(var now: Long = 1_893_452_500L) : Platform {
    override val clock = Clock { now }
    override val clipboard = FakeClipboard()
    override val keys = FakeKeys()
    override val camera = FakeCamera()
    override val ble = FakeBle()
    override val audio = FakeAudio()
    override val system = FakeSystem()
}

class FakeClipboard : ClipboardPort {
    var text: String? = null; var label: String? = null; var sensitive = false
    val clears = MutableStateFlow<Int?>(null)
    override fun copySensitive(label: String, text: String) { this.label = label; this.text = text; sensitive = true; clears.value = 60 }
    override fun readText(): String? = text
    override val clearsInSeconds: StateFlow<Int?> get() = clears.asStateFlow()
}

class FakeKeys : KeyVault {
    val st = MutableStateFlow(VaultState.Unlocked)
    var secret: ByteArray? = ByteArray(32) { 0x42 }
    var nextResult: VaultResult? = null
    override val state: StateFlow<VaultState> get() = st.asStateFlow()
    override suspend fun store(secret: ByteArray): VaultResult { this.secret = secret.copyOf(); st.value = VaultState.Locked; return nextResult ?: VaultResult.Stored }
    override suspend fun unlock(title: String, subtitle: String): VaultResult = nextResult ?: secret?.let { st.value = VaultState.Unlocked; VaultResult.Secret(it.copyOf()) } ?: VaultResult.Failed("no key")
    override suspend fun erase() { secret = null; st.value = VaultState.NoKey }
    override fun lock() { if (st.value == VaultState.Unlocked) st.value = VaultState.Locked }
    override val strongBox = true
}

class FakeCamera : CameraPort {
    val perm = MutableStateFlow(CameraPermission.Granted)
    var requested = 0; var settingsOpened = 0
    override val permission: StateFlow<CameraPermission> get() = perm.asStateFlow()
    override fun requestPermission() { requested++ }
    override fun openAppSettings() { settingsOpened++ }
    override var hasCamera = true
    /** Last callback registered by a Preview; tests call it to inject frames. */
    var onLuma: ((Int, Int, ByteArray) -> Unit)? = null
    @Composable override fun Preview(modifier: Modifier, onLuma: (Int, Int, ByteArray) -> Unit) {
        this.onLuma = onLuma
        Box(modifier.fillMaxWidth().aspectRatio(1f).background(Color.DarkGray), Alignment.Center) { Text("fake camera") }
    }
}

class FakeBle : BlePort {
    val av = MutableStateFlow(BleAvailability.Ready)
    val found = MutableStateFlow(listOf<BleDevice>())
    var channel: FakeBleChannel? = FakeBleChannel()
    override val availability: StateFlow<BleAvailability> get() = av.asStateFlow()
    override fun requestPermissions() {}
    override fun scan(): Flow<List<BleDevice>> = found
    val scanErr = MutableStateFlow<BleScanError?>(null)
    override val scanError: StateFlow<BleScanError?> get() = scanErr.asStateFlow()
    override suspend fun connect(device: BleDevice): ObservedBleChannel? = channel
}
class FakeBleChannel : ObservedBleChannel {
    val st = MutableStateFlow<BleChannelState>(BleChannelState.Open(185))
    override val state: StateFlow<BleChannelState> get() = st.asStateFlow()
    var failure: String? = null
    var refused = 0
    override fun sendFailure(): String? = failure.also { failure = null }
    override fun frameRefused() { refused++ }
    val sent = mutableListOf<ByteArray>()
    val inbox = MutableStateFlow<ByteArray?>(null)
    override val incoming: Flow<ByteArray> = flowOf()
    override suspend fun send(frame: ByteArray): Boolean { sent += frame; return true }
    override fun close() {}
}

class FakeAudio : AudioPort {
    val played = mutableListOf<Pair<Int, Int>>() // (samples, rate)
    val p = MutableStateFlow(false)
    override suspend fun play(pcm: ShortArray, rate: Int) { played += pcm.size to rate }
    override fun stop() { p.value = false }
    override val playing: StateFlow<Boolean> get() = p.asStateFlow()
}

class FakeSystem : SystemPort {
    override var apkCertSha256 = "a3f9007c4e1b9d2e" + "0123456789abcdef".repeat(3)   // 64 hex digits, fixed for screenshots
    override val versionName = "0.1.0-debug"
    override fun licenceText(name: String) = "licence text of $name"
    override var reducedMotion = false
}
