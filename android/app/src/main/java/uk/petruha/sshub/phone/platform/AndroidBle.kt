package uk.petruha.sshub.phone.platform

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
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
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
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
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.ReceiveChannel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.selects.onTimeout
import kotlinx.coroutines.selects.select
import uk.petruha.sshub.phone.core.BleAvailability
import uk.petruha.sshub.phone.core.BleChannel
import uk.petruha.sshub.phone.core.BleDevice
import uk.petruha.sshub.phone.core.BlePort
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

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
    /** The core's sentence for why the session ended; meaningful once [closed] is true. */
    val closedReason: String? get() = null
    /** Why the last frame was refused or never acknowledged by the terminal, once. */
    fun sendFailure(): String? = null
}

/** Creates the link for a connection with the negotiated ATT MTU (the core's `ble_new(mtu)`). */
fun interface BleLinkFactory { fun create(mtu: Int): BleLink }

/** Where a [ObservedBleChannel] is: connecting, open with the negotiated MTU, or over with the reason to show the person. */
sealed interface BleChannelState {
    data object Connecting : BleChannelState
    data class Open(val mtu: Int) : BleChannelState
    data class Closed(val reason: String) : BleChannelState
}

/**
 * What `AndroidBle.connect` really returns. The state is [BleChannelState.Closed] with a sentence as soon as the session is over
 * (terminal BYE or timeout: the core's own reason; link lost, Bluetooth off, flood, too many refused frames: ours); `incoming` completes
 * after the frames received before that were delivered, so a screen collects `incoming` and watches `state`.
 */
interface ObservedBleChannel : BleChannel {
    val state: StateFlow<BleChannelState>
    /** Why the terminal refused the last frame sent (its `A_BUSY`, `A_TOO_LARGE`, ... sentence) or null; returns it once. */
    fun sendFailure(): String?
    /** The caller's core refused a frame this channel delivered; after a few of them the session is closed (a hostile peripheral). */
    fun frameRefused()
}

/** Why a scan stopped, with the platform's code and a sentence a screen can show. */
data class BleScanError(val code: Int, val text: String) {
    companion object {
        fun of(code: Int) = BleScanError(
            code,
            when (code) {
                ScanCallback.SCAN_FAILED_ALREADY_STARTED -> "A scan is already running."
                ScanCallback.SCAN_FAILED_APPLICATION_REGISTRATION_FAILED -> "Bluetooth could not start the scan. Turn Bluetooth off and on."
                ScanCallback.SCAN_FAILED_INTERNAL_ERROR -> "Bluetooth failed to scan. Turn Bluetooth off and on."
                ScanCallback.SCAN_FAILED_FEATURE_UNSUPPORTED -> "This phone cannot scan for Bluetooth LE devices."
                ScanCallback.SCAN_FAILED_OUT_OF_HARDWARE_RESOURCES -> "Bluetooth has no scan resources left. Close other Bluetooth apps."
                ScanCallback.SCAN_FAILED_SCANNING_TOO_FREQUENTLY -> "Android limits scan restarts. Wait 30 seconds and try again."
                else -> "The scan failed (code $code)."
            },
        )
    }
}

private val SERVICE: UUID = UUID.fromString("b62f7d3f-0001-42dd-8371-6ffd96411583")
private val RX: UUID = UUID.fromString("b62f7d3f-0002-42dd-8371-6ffd96411583")
private val TX: UUID = UUID.fromString("b62f7d3f-0003-42dd-8371-6ffd96411583")
private val INFO: UUID = UUID.fromString("b62f7d3f-0004-42dd-8371-6ffd96411583")
private val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

/** `SSHub-` plus four Crockford base32 characters (no I, L, O, U). */
private val NAME = Regex("SSHub-[0-9A-HJKMNP-TV-Z]{4}")

/** At most this many distinct terminals are listed; a hostile advertiser cannot grow the list or flood the UI. */
internal const val SCAN_LIST_CAP = 16

/** After this many frames the core refused (or the terminal refused to take), the session is closed. */
internal const val REFUSAL_LIMIT = 8

/** The scan result list: one entry per address, newest data wins, a new address beyond [cap] is ignored. */
internal class ScanList(private val cap: Int = SCAN_LIST_CAP) {
    private val found = LinkedHashMap<String, BleDevice>()
    /** True when the list changed. */
    fun offer(address: String, name: String): Boolean {
        if (address !in found && found.size >= cap) return false
        val d = BleDevice(address, name)
        if (found[address] == d) return false
        found[address] = d
        return true
    }
    fun snapshot(): List<BleDevice> = found.values.toList()
}

/** Counts refusals; [add] is true once the limit is reached. */
internal class RefusalBudget(private val limit: Int = REFUSAL_LIMIT) {
    private var n = 0
    fun add(): Boolean { n++; return n >= limit }
}

/**
 * Receives the next element or null after [ms] (or when the channel is closed). Unlike `withTimeoutOrNull { receive() }` an element that
 * arrives together with the timeout is never taken and dropped: `select` completes with exactly one of the two clauses.
 */
@OptIn(ExperimentalCoroutinesApi::class)
internal suspend fun <T : Any> ReceiveChannel<T>.receiveWithin(ms: Long): T? = select {
    onReceiveCatching { it.getOrNull() }
    onTimeout(ms) { null }
}

/**
 * GATT central for C20. With no `links` factory the port reports Unsupported. API 30 is Unsupported too: scanning there needs
 * location and the legacy Bluetooth permissions, which the manifest does not declare (the app's minSdk is 30, so on Android 11 the
 * Bluetooth channel is simply not offered; the other channels work).
 *
 * The availability follows the adapter live (an `ACTION_STATE_CHANGED` receiver on the application context), not only at resume;
 * [release] unregisters it (the platform object lives as long as the process, tests release theirs).
 */
class AndroidBle(private val context: Context, private val links: BleLinkFactory? = null) : BlePort {
    private val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter
    private val _availability = MutableStateFlow(BleAvailability.Unsupported)
    override val availability: StateFlow<BleAvailability> = _availability.asStateFlow()
    private val _scanError = MutableStateFlow<BleScanError?>(null)
    /** Why the last scan stopped (null while it runs or after a clean stop); reset when a scan starts. */
    val scanError: StateFlow<BleScanError?> = _scanError.asStateFlow()
    private var launcher: ActivityResultLauncher<Array<String>>? = null
    private var denied = false
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val channels = ConcurrentHashMap.newKeySet<GattChannel>()
    private val appContext = context.applicationContext ?: context
    private val stateReceiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context?, i: Intent?) {
            refresh()
            if (_availability.value != BleAvailability.Ready) channels.forEach { it.bluetoothOff() }
        }
    }

    init {
        refresh()
        appContext.registerReceiver(stateReceiver, IntentFilter(BluetoothAdapter.ACTION_STATE_CHANGED), Context.RECEIVER_NOT_EXPORTED)
    }

    private var host: ComponentActivity? = null
    private var observer: LifecycleEventObserver? = null

    /** Stops tracking the adapter and closes the open channels. */
    fun release() {
        runCatching { appContext.unregisterReceiver(stateReceiver) }
        channels.forEach { it.close() }
    }

    /** Points the port at the new Activity; the previous launcher and observer are released first. */
    fun attach(activity: ComponentActivity) {
        detach()
        host = activity
        launcher = activity.registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { res ->
            denied = !res.values.all { it }
            refresh()
        }
        val o = LifecycleEventObserver { _, e -> if (e == Lifecycle.Event.ON_RESUME) refresh() }
        observer = o
        activity.lifecycle.addObserver(o)
    }

    fun detach() {
        observer?.let { o -> host?.lifecycle?.removeObserver(o) }
        observer = null
        runCatching { launcher?.unregister() }
        launcher = null
        host = null
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
        _scanError.value = null
        val scanner = if (hasPermissions()) adapter?.bluetoothLeScanner else null
        if (scanner == null) {
            close()
            return@callbackFlow
        }
        val found = ScanList()
        val cb = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                val name = result.scanRecord?.deviceName ?: return
                if (!NAME.matches(name)) return
                if (found.offer(result.device.address, name)) trySend(found.snapshot())
            }
            override fun onScanFailed(errorCode: Int) {
                _scanError.value = BleScanError.of(errorCode)
                close()
            }
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
    override suspend fun connect(device: BleDevice): ObservedBleChannel? {
        val factory = links ?: return null
        if (_availability.value != BleAvailability.Ready) return null
        val remote: BluetoothDevice? = try { adapter?.getRemoteDevice(device.address) } catch (_: IllegalArgumentException) { null }
        if (remote == null) return null
        val ch = GattChannel(RealGattOps(context, remote), factory, scope, onClosed = { channels.remove(it) })
        channels.add(ch)
        // A cancellation while connecting closes the GATT client inside open() (and rethrows); the channel is not leaked.
        return if (ch.open()) ch else { channels.remove(ch); null }
    }

    private companion object {
        val PERMISSIONS = arrayOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT)
    }
}

/** What the GATT callbacks report, as plain values: the channel logic below never sees a `BluetoothGatt`. */
internal interface GattEvents {
    fun connection(status: Int, connected: Boolean)
    fun mtu(mtu: Int, status: Int)
    fun services(status: Int)
    fun info(status: Int, value: ByteArray)
    fun descriptor(status: Int)
    fun written(status: Int)
    fun notified(value: ByteArray)
}

/** The GATT calls the channel makes. [RealGattOps] is the one over `BluetoothGatt`; tests script their own. */
internal interface GattOps {
    /** Starts a connection (a fresh client each time); false when the system gave none. */
    fun connect(events: GattEvents): Boolean
    fun requestMtu(mtu: Int): Boolean
    fun discoverServices(): Boolean
    /** Finds the C20 service, RX, TX, INFO and the TX descriptor on the discovered services. */
    fun resolve(): Boolean
    fun readInfo(): Boolean
    /** Local notification flag plus the descriptor write that turns TX notifications on. */
    fun enableNotifications(): Boolean
    /** Queues a write to RX (no response); the result comes back through [GattEvents.written]. */
    fun write(message: ByteArray): Boolean
    fun disconnect()
    fun close()
}

@SuppressLint("MissingPermission")
@Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
private class RealGattOps(private val context: Context, private val device: BluetoothDevice) : GattOps {
    private var gatt: BluetoothGatt? = null
    private var rx: BluetoothGattCharacteristic? = null
    private var tx: BluetoothGattCharacteristic? = null
    private var info: BluetoothGattCharacteristic? = null
    private var cccd: BluetoothGattDescriptor? = null

    override fun connect(events: GattEvents): Boolean {
        val callback = object : BluetoothGattCallback() {
            override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) {
                events.connection(status, newState == BluetoothProfile.STATE_CONNECTED)
            }
            override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) = events.mtu(mtu, status)
            override fun onServicesDiscovered(g: BluetoothGatt, status: Int) = events.services(status)
            override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray, status: Int) =
                events.info(status, value)
            override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) =
                events.info(status, c.value ?: ByteArray(0))
            override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) = events.descriptor(status)
            override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) = events.written(status)
            override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) = events.notified(value)
            override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) = events.notified(c.value ?: ByteArray(0))
        }
        gatt = device.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
        return gatt != null
    }

    override fun requestMtu(mtu: Int) = gatt?.requestMtu(mtu) ?: false

    override fun discoverServices() = gatt?.discoverServices() ?: false

    override fun resolve(): Boolean {
        val svc = gatt?.getService(SERVICE) ?: return false
        rx = svc.getCharacteristic(RX) ?: return false
        val t = svc.getCharacteristic(TX) ?: return false
        tx = t
        info = svc.getCharacteristic(INFO) ?: return false
        cccd = t.getDescriptor(CCCD) ?: return false
        return true
    }

    override fun readInfo(): Boolean = info?.let { gatt?.readCharacteristic(it) } ?: false

    override fun enableNotifications(): Boolean {
        val g = gatt ?: return false
        val t = tx ?: return false
        val d = cccd ?: return false
        if (!g.setCharacteristicNotification(t, true)) return false
        return if (Build.VERSION.SDK_INT >= 33) {
            g.writeDescriptor(d, BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE) == BluetoothStatusCodes.SUCCESS
        } else {
            d.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
            g.writeDescriptor(d)
        }
    }

    override fun write(message: ByteArray): Boolean {
        val g = gatt ?: return false
        val c = rx ?: return false
        return if (Build.VERSION.SDK_INT >= 33) {
            g.writeCharacteristic(c, message, BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE) == BluetoothStatusCodes.SUCCESS
        } else {
            c.writeType = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
            c.value = message
            g.writeCharacteristic(c)
        }
    }

    override fun disconnect() { runCatching { gatt?.disconnect() } }

    override fun close() {
        runCatching { gatt?.close() }
        gatt = null
    }
}

internal const val GATT_ERROR = 133
private const val GATT_OK = 0

/**
 * One C20 connection over [GattOps]. Everything that touches the link runs in the single loop coroutine. All queues are bounded: a
 * peripheral that floods notifications or frames ends the session instead of growing memory.
 */
internal class GattChannel(
    private val ops: GattOps,
    private val factory: BleLinkFactory,
    private val scope: CoroutineScope,
    private val retryDelayMs: Long = 400,
    private val refusalLimit: Int = REFUSAL_LIMIT,
    private val onClosed: (GattChannel) -> Unit = {},
) : ObservedBleChannel, GattEvents {
    private sealed interface Ev {
        data object Up : Ev
        class Down(val status: Int) : Ev
        data object Close : Ev
        data object Refused : Ev
        class Mtu(val mtu: Int, val status: Int) : Ev
        class Services(val status: Int) : Ev
        class Info(val status: Int, val value: ByteArray) : Ev
        class Desc(val status: Int) : Ev
        class Notify(val value: ByteArray) : Ev
        class Send(val frame: ByteArray, val done: CompletableDeferred<Boolean>) : Ev
    }

    private val events = Channel<Ev>(EVENTS_CAP)
    private val acks = Channel<Int>(ACKS_CAP)
    private val inbox = Channel<ByteArray>(INBOX_CAP)
    private val cleaned = AtomicBoolean(false)
    @Volatile private var overflow = false
    @Volatile private var lastDown = GATT_OK
    private val failure = AtomicReference<String?>(null)
    private var link: BleLink? = null
    private var job: Job? = null
    private val _state = MutableStateFlow<BleChannelState>(BleChannelState.Connecting)

    override val incoming: Flow<ByteArray> = inbox.receiveAsFlow()
    override val state: StateFlow<BleChannelState> = _state.asStateFlow()

    // The GATT callbacks run on a Binder thread: they only queue, and a full queue is a flood.
    private fun offer(e: Ev) {
        val r = events.trySend(e)
        if (r.isFailure && !r.isClosed) overflow = true
    }

    override fun connection(status: Int, connected: Boolean) = offer(if (connected && status == GATT_OK) Ev.Up else Ev.Down(status))
    override fun mtu(mtu: Int, status: Int) = offer(Ev.Mtu(mtu, status))
    override fun services(status: Int) = offer(Ev.Services(status))
    override fun info(status: Int, value: ByteArray) = offer(Ev.Info(status, value))
    override fun descriptor(status: Int) = offer(Ev.Desc(status))
    override fun written(status: Int) { acks.trySend(status) }
    override fun notified(value: ByteArray) = offer(Ev.Notify(value))

    private suspend inline fun <reified T : Ev> awaitEv(ms: Long): T? {
        val end = System.nanoTime() + ms * 1_000_000
        while (true) {
            val left = (end - System.nanoTime()) / 1_000_000
            if (left <= 0) return null
            val e = events.receiveWithin(left) ?: return null
            if (e is Ev.Down) { lastDown = e.status; return null }
            if (e is T) return e
        }
    }

    /**
     * Connect (one retry after status 133, with the failed client closed first), MTU 517, discover, read INFO, enable TX notifications,
     * then start the link loop (HELLO leaves with the first flush). A cancellation closes the GATT client and rethrows.
     */
    suspend fun open(): Boolean {
        try {
            if (!connectWithRetry()) return fail("Could not connect to the terminal.")
            ops.requestMtu(517)
            val mtu = awaitEv<Ev.Mtu>(5_000)?.takeIf { it.status == GATT_OK }?.mtu ?: 23
            if (!ops.discoverServices()) return fail("The terminal's services could not be read.")
            val found = awaitEv<Ev.Services>(10_000)
            if (found == null || found.status != GATT_OK) return fail("The terminal's services could not be read.")
            if (!ops.resolve()) return fail("This is not an SSHub terminal.")
            if (!ops.readInfo()) return fail("The terminal did not answer.")
            val info = awaitEv<Ev.Info>(5_000)
            if (info == null || info.status != GATT_OK || info.value.size < 9 || info.value[0] != 1.toByte()) {
                return fail("This terminal speaks another protocol version.")
            }
            if (!ops.enableNotifications()) return fail("The terminal did not accept notifications.")
            val d = awaitEv<Ev.Desc>(5_000)
            if (d == null || d.status != GATT_OK) return fail("The terminal did not accept notifications.")
            link = factory.create(mtu)
            _state.value = BleChannelState.Open(mtu)
            job = scope.launch { loop() }
            return true
        } catch (e: CancellationException) {
            cleanup("Connecting was cancelled.")
            throw e
        } catch (e: Exception) {
            return fail("Bluetooth failed: ${e.message ?: e.javaClass.simpleName}.")
        }
    }

    private suspend fun connectWithRetry(): Boolean {
        for (attempt in 0..1) {
            lastDown = GATT_OK
            if (ops.connect(this) && awaitEv<Ev.Up>(15_000) != null) return true
            val retry = attempt == 0 && lastDown == GATT_ERROR
            ops.disconnect()
            ops.close()
            if (!retry) return false
            while (events.tryReceive().isSuccess) { /* stale events of the failed client */ }
            delay(retryDelayMs)
        }
        return false
    }

    private fun fail(reason: String): Boolean { cleanup(reason); return false }

    private suspend fun loop() {
        val l = link!!
        val refusals = RefusalBudget(refusalLimit)
        var why = ""
        var up = true
        try {
            run session@{
                while (!l.closed) {
                    if (overflow) { why = FLOOD; return@session }
                    val ev = events.receiveWithin(TICK_MS)
                    when (ev) {
                        is Ev.Notify -> l.onMessage(ev.value)
                        is Ev.Send -> ev.done.complete(l.sendFrame(ev.frame))
                        is Ev.Refused -> if (refusals.add()) { why = REFUSED; l.bye(); return@session }
                        is Ev.Down -> { up = false; why = LOST; return@session }
                        Ev.Close -> { l.bye(); flush(l); why = CLOSED_HERE; up = false; return@session }
                        else -> {}
                    }
                    l.tick(SystemClock.elapsedRealtime())
                    if (!flush(l)) { up = false; why = LOST; return@session }
                    l.sendFailure()?.let { failure.set(it); if (refusals.add()) { why = REFUSED; l.bye(); return@session } }
                    for (f in l.takeFrames()) if (!inbox.trySend(f).isSuccess) { why = FLOOD; return@session }
                }
            }
            if (up) flush(l)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // a failure inside the link (a core exception) ends this session; it must not take the process down on the IO scope
            why = "The Bluetooth link failed: ${e.message ?: e.javaClass.simpleName}."
        } finally {
            val reason = why.ifEmpty { if (l.closed) l.closedReason ?: "The session ended." else LOST }
            cleanup(reason)
        }
    }

    private suspend fun flush(l: BleLink): Boolean {
        for (m in l.takeOutbox()) if (!write(m)) return false
        return true
    }

    private suspend fun write(m: ByteArray): Boolean {
        while (acks.tryReceive().isSuccess) { /* a late ack of an earlier write must not answer this one */ }
        if (!ops.write(m)) return false
        return acks.receiveWithin(ACK_MS) == GATT_OK
    }

    override suspend fun send(frame: ByteArray): Boolean {
        val done = CompletableDeferred<Boolean>()
        if (!events.trySend(Ev.Send(frame, done)).isSuccess) return false
        return done.await()
    }

    override fun sendFailure(): String? = failure.getAndSet(null)

    override fun frameRefused() { events.trySend(Ev.Refused) }

    /** Bluetooth was switched off under the connection. */
    fun bluetoothOff() { reasonOverride = BT_OFF; events.trySend(Ev.Down(GATT_OK)) }

    @Volatile private var reasonOverride: String? = null

    override fun close() {
        events.trySend(Ev.Close)
        if (job == null) cleanup(CLOSED_HERE)
    }

    private fun cleanup(reason: String) {
        if (!cleaned.compareAndSet(false, true)) return
        _state.value = BleChannelState.Closed(reasonOverride ?: reason)
        runCatching { ops.disconnect() }
        runCatching { ops.close() }
        events.close()
        while (true) {
            val e = events.tryReceive().getOrNull() ?: break
            if (e is Ev.Send) e.done.complete(false)
        }
        acks.close()
        inbox.close()
        onClosed(this)
    }

    internal companion object {
        const val TICK_MS = 200L
        const val ACK_MS = 5_000L
        const val EVENTS_CAP = 512
        const val ACKS_CAP = 16
        const val INBOX_CAP = 8
        const val LOST = "The Bluetooth connection was lost."
        const val CLOSED_HERE = "The connection was closed on this phone."
        const val BT_OFF = "Bluetooth was turned off."
        const val FLOOD = "The terminal sent too much data. The connection was closed."
        const val REFUSED = "The terminal sent too many refused frames. The connection was closed."
    }
}
