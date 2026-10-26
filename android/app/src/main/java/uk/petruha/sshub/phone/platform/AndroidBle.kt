package uk.petruha.sshub.phone.platform

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothStatusCodes
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.ParcelUuid
import android.os.SystemClock
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import uk.petruha.sshub.phone.core.BleAvailability
import uk.petruha.sshub.phone.core.BleChannel
import uk.petruha.sshub.phone.core.BleDevice
import uk.petruha.sshub.phone.core.BlePort
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The sans-IO link of contract C20 (Rust `BleSession`, phone role). All calls come from one coroutine; the adapter moves GATT bytes and
 * calls `tick`. `takeOutbox` returns the messages to write to RX, in order; `takeFrames` the whole frames received since the last call.
 */
interface BleLink {
    fun onMessage(b: ByteArray)
    fun takeOutbox(): List<ByteArray>
    fun sendFrame(f: ByteArray): Boolean
    fun takeFrames(): List<ByteArray>
    fun tick(nowMillis: Long)
    /** Ends the session politely (BYE) before the disconnect. */
    fun bye()
    val closed: Boolean
}

/** Creates the link for a connection with the negotiated ATT MTU (the core's `ble_new(mtu)`). */
fun interface BleLinkFactory { fun create(mtu: Int): BleLink }

private val SERVICE: UUID = UUID.fromString("b62f7d3f-0001-42dd-8371-6ffd96411583")
private val RX: UUID = UUID.fromString("b62f7d3f-0002-42dd-8371-6ffd96411583")
private val TX: UUID = UUID.fromString("b62f7d3f-0003-42dd-8371-6ffd96411583")
private val INFO: UUID = UUID.fromString("b62f7d3f-0004-42dd-8371-6ffd96411583")
private val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

/** `SSHub-` plus four Crockford base32 characters (no I, L, O, U). */
private val NAME = Regex("SSHub-[0-9A-HJKMNP-TV-Z]{4}")

/**
 * GATT central for C20. With no `links` factory the port reports Unsupported. API 30 is Unsupported too: scanning there needs
 * location and the legacy Bluetooth permissions, which the manifest does not declare.
 */
class AndroidBle(private val context: Context, private val links: BleLinkFactory? = null) : BlePort {
    private val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter
    private val _availability = MutableStateFlow(BleAvailability.Unsupported)
    override val availability: StateFlow<BleAvailability> = _availability.asStateFlow()
    private var launcher: ActivityResultLauncher<Array<String>>? = null
    private var denied = false
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    init { refresh() }

    fun attach(activity: ComponentActivity) {
        launcher = activity.registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { res ->
            denied = !res.values.all { it }
            refresh()
        }
        activity.lifecycle.addObserver(LifecycleEventObserver { _, e -> if (e == Lifecycle.Event.ON_RESUME) refresh() })
    }

    fun refresh() {
        _availability.value = when {
            links == null || adapter == null || Build.VERSION.SDK_INT < 31 ||
                !context.packageManager.hasSystemFeature(PackageManager.FEATURE_BLUETOOTH_LE) -> BleAvailability.Unsupported
            !hasPermissions() -> if (denied) BleAvailability.PermissionDenied else BleAvailability.PermissionNeeded
            adapter?.isEnabled != true -> BleAvailability.Off
            else -> BleAvailability.Ready
        }
    }

    private fun hasPermissions() = PERMISSIONS.all { ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }

    override fun requestPermissions() {
        if (hasPermissions()) { refresh(); return }
        launcher?.launch(PERMISSIONS)
    }

    @SuppressLint("MissingPermission")
    override fun scan(): Flow<List<BleDevice>> = callbackFlow {
        val scanner = if (hasPermissions()) adapter?.bluetoothLeScanner else null
        if (scanner == null) {
            close()
            return@callbackFlow
        }
        val found = LinkedHashMap<String, BleDevice>()
        val cb = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                val name = result.scanRecord?.deviceName ?: return
                if (!NAME.matches(name)) return
                found[result.device.address] = BleDevice(result.device.address, name)
                trySend(found.values.toList())
            }
            override fun onScanFailed(errorCode: Int) { close() }
        }
        val filters = listOf(ScanFilter.Builder().setServiceUuid(ParcelUuid(SERVICE)).build())
        val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
        try {
            scanner.startScan(filters, settings, cb)
        } catch (_: SecurityException) {
            close()
            return@callbackFlow
        }
        awaitClose { try { scanner.stopScan(cb) } catch (_: SecurityException) { } }
    }

    @SuppressLint("MissingPermission")
    override suspend fun connect(device: BleDevice): BleChannel? {
        val factory = links ?: return null
        if (_availability.value != BleAvailability.Ready) return null
        val remote: BluetoothDevice? = try { adapter?.getRemoteDevice(device.address) } catch (_: IllegalArgumentException) { null }
        if (remote == null) return null
        val ch = GattChannel(context, remote, factory, scope)
        return if (ch.open()) ch else null
    }

    private companion object {
        val PERMISSIONS = arrayOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT)
    }
}

@SuppressLint("MissingPermission")
@Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
private class GattChannel(
    private val context: Context,
    private val device: BluetoothDevice,
    private val factory: BleLinkFactory,
    private val scope: CoroutineScope,
) : BleChannel {
    private sealed interface Ev {
        data object Up : Ev
        data object Down : Ev
        data object Close : Ev
        class Mtu(val mtu: Int, val status: Int) : Ev
        class Services(val status: Int) : Ev
        class Info(val status: Int, val value: ByteArray) : Ev
        class Desc(val status: Int) : Ev
        class Notify(val value: ByteArray) : Ev
        class Send(val frame: ByteArray, val done: CompletableDeferred<Boolean>) : Ev
    }

    private val events = Channel<Ev>(Channel.UNLIMITED)
    private val acks = Channel<Int>(Channel.UNLIMITED)
    private val inbox = Channel<ByteArray>(Channel.UNLIMITED)
    private val cleaned = AtomicBoolean(false)
    private var gatt: BluetoothGatt? = null
    private var rx: BluetoothGattCharacteristic? = null
    private var link: BleLink? = null
    private var job: Job? = null

    override val incoming: Flow<ByteArray> = inbox.receiveAsFlow()

    private val callback = object : BluetoothGattCallback() {
        override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) {
            events.trySend(if (newState == BluetoothProfile.STATE_CONNECTED && status == BluetoothGatt.GATT_SUCCESS) Ev.Up else Ev.Down)
        }
        override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) { events.trySend(Ev.Mtu(mtu, status)) }
        override fun onServicesDiscovered(g: BluetoothGatt, status: Int) { events.trySend(Ev.Services(status)) }
        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray, status: Int) {
            events.trySend(Ev.Info(status, value))
        }
        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            events.trySend(Ev.Info(status, c.value ?: ByteArray(0)))
        }
        override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) { events.trySend(Ev.Desc(status)) }
        override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) { acks.trySend(status) }
        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) {
            events.trySend(Ev.Notify(value))
        }
        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
            events.trySend(Ev.Notify(c.value ?: ByteArray(0)))
        }
    }

    private suspend inline fun <reified T : Ev> awaitEv(ms: Long): T? = withTimeoutOrNull(ms) {
        var r: T? = null
        while (r == null) {
            val e = events.receive()
            if (e is Ev.Down) break
            if (e is T) r = e
        }
        r
    }

    /** Connect, MTU 517, discover, read INFO, enable TX notifications, then start the link loop (HELLO leaves with the first flush). */
    suspend fun open(): Boolean {
        val g = device.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE) ?: return fail()
        gatt = g
        if (awaitEv<Ev.Up>(15_000) == null) return fail()
        g.requestMtu(517)
        val mtu = awaitEv<Ev.Mtu>(5_000)?.takeIf { it.status == BluetoothGatt.GATT_SUCCESS }?.mtu ?: 23
        if (!g.discoverServices()) return fail()
        val found = awaitEv<Ev.Services>(10_000)
        if (found == null || found.status != BluetoothGatt.GATT_SUCCESS) return fail()
        val svc = g.getService(SERVICE) ?: return fail()
        val rxc = svc.getCharacteristic(RX) ?: return fail()
        val txc = svc.getCharacteristic(TX) ?: return fail()
        val infoc = svc.getCharacteristic(INFO) ?: return fail()
        if (!g.readCharacteristic(infoc)) return fail()
        val info = awaitEv<Ev.Info>(5_000)
        if (info == null || info.status != BluetoothGatt.GATT_SUCCESS || info.value.size < 9 || info.value[0] != 1.toByte()) return fail()
        val cccd = txc.getDescriptor(CCCD) ?: return fail()
        if (!g.setCharacteristicNotification(txc, true)) return fail()
        val queued = if (Build.VERSION.SDK_INT >= 33) {
            g.writeDescriptor(cccd, BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE) == BluetoothStatusCodes.SUCCESS
        } else {
            cccd.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
            g.writeDescriptor(cccd)
        }
        if (!queued) return fail()
        val d = awaitEv<Ev.Desc>(5_000)
        if (d == null || d.status != BluetoothGatt.GATT_SUCCESS) return fail()
        rx = rxc
        link = factory.create(mtu)
        job = scope.launch { loop() }
        return true
    }

    private fun fail(): Boolean { cleanup(); return false }

    private suspend fun loop() {
        val l = link!!
        var up = true
        try {
            while (!l.closed) {
                val ev = withTimeoutOrNull(TICK_MS) { events.receive() }
                when (ev) {
                    is Ev.Notify -> l.onMessage(ev.value)
                    is Ev.Send -> ev.done.complete(l.sendFrame(ev.frame))
                    Ev.Down -> { up = false; break }
                    Ev.Close -> { l.bye(); flush(l); break }
                    else -> {}
                }
                l.tick(SystemClock.elapsedRealtime())
                if (!flush(l)) { up = false; break }
                for (f in l.takeFrames()) inbox.trySend(f)
            }
            if (up) flush(l)
        } finally {
            cleanup()
        }
    }

    private suspend fun flush(l: BleLink): Boolean {
        for (m in l.takeOutbox()) if (!write(m)) return false
        return true
    }

    private suspend fun write(m: ByteArray): Boolean {
        val g = gatt ?: return false
        val c = rx ?: return false
        val queued = if (Build.VERSION.SDK_INT >= 33) {
            g.writeCharacteristic(c, m, BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE) == BluetoothStatusCodes.SUCCESS
        } else {
            c.writeType = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
            c.value = m
            g.writeCharacteristic(c)
        }
        if (!queued) return false
        return withTimeoutOrNull(5_000) { acks.receive() } == BluetoothGatt.GATT_SUCCESS
    }

    override suspend fun send(frame: ByteArray): Boolean {
        val done = CompletableDeferred<Boolean>()
        if (!events.trySend(Ev.Send(frame, done)).isSuccess) return false
        return done.await()
    }

    override fun close() {
        events.trySend(Ev.Close)
        if (job == null) cleanup()
    }

    private fun cleanup() {
        if (!cleaned.compareAndSet(false, true)) return
        runCatching { gatt?.disconnect() }
        runCatching { gatt?.close() }
        events.close()
        while (true) {
            val e = events.tryReceive().getOrNull() ?: break
            if (e is Ev.Send) e.done.complete(false)
        }
        inbox.close()
    }

    private companion object { const val TICK_MS = 200L }
}
