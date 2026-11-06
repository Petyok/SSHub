package uk.petruha.sshub.phone.platform

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.CopyOnWriteArrayList

/** A scripted GATT: the connect results come from [results] (0 comes up, anything else is a Down with that status). */
internal class FakeGattOps(private val results: List<Int> = listOf(0), private val neverUp: Boolean = false) : GattOps {
    @Volatile var events: GattEvents? = null
    val log = CopyOnWriteArrayList<String>()
    val writes = CopyOnWriteArrayList<ByteArray>()
    @Volatile var connects = 0
    @Volatile var closes = 0

    override fun connect(events: GattEvents): Boolean {
        this.events = events
        connects++
        log += "connect"
        if (!neverUp) {
            val st = results.getOrElse(connects - 1) { 0 }
            events.connection(st, st == 0)
        }
        return true
    }
    override fun requestMtu(mtu: Int): Boolean { events!!.mtu(247, 0); return true }
    override fun discoverServices(): Boolean { events!!.services(0); return true }
    override fun resolve() = true
    override fun readInfo(): Boolean { events!!.info(0, ByteArray(9).also { it[0] = 1 }); return true }
    override fun enableNotifications(): Boolean { events!!.descriptor(0); return true }
    override fun write(message: ByteArray): Boolean { writes += message; events!!.written(0); return true }
    override fun disconnect() { log += "disconnect" }
    override fun close() { log += "close"; closes++ }
}

internal class FakeLink : BleLink {
    val got = CopyOnWriteArrayList<ByteArray>()
    private val frames = ArrayList<ByteArray>()
    private val out = ArrayList<ByteArray>()
    @Volatile override var closed = false
    @Volatile override var closedReason: String? = null
    @Volatile var failure: String? = null
    @Volatile var byeCalled = false
    @Volatile var slowMs = 0L

    override fun onMessage(b: ByteArray) {
        if (slowMs > 0) Thread.sleep(slowMs)
        got += b
        if (b.size == 1 && b[0] == 'X'.code.toByte()) { closed = true; closedReason = "The terminal ended the session." } else frames += b
    }
    override fun takeOutbox(): List<ByteArray> = out.toList().also { out.clear() }
    override fun sendFrame(f: ByteArray): Boolean { out += f; return true }
    override fun takeFrames(): List<ByteArray> = frames.toList().also { frames.clear() }
    override fun tick(nowMillis: Long) {}
    override fun bye() { byeCalled = true }
    override fun sendFailure(): String? = failure.also { failure = null }
}

class BleChannelTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val link = FakeLink()

    @After fun stop() { scope.cancel() }

    private fun channel(ops: GattOps, refusalLimit: Int = REFUSAL_LIMIT) =
        GattChannel(ops, { link }, scope, retryDelayMs = 1, refusalLimit = refusalLimit)

    private fun waitFor(what: String, ms: Long = 5_000, cond: () -> Boolean) {
        val end = System.nanoTime() + ms * 1_000_000
        while (!cond()) {
            if (System.nanoTime() > end) throw AssertionError("timed out waiting for $what")
            Thread.sleep(2)
        }
    }

    private fun opened(ops: GattOps = FakeGattOps(), refusalLimit: Int = REFUSAL_LIMIT): GattChannel =
        channel(ops, refusalLimit).also { assertTrue(runBlocking { it.open() }) }

    private fun closedReason(ch: GattChannel): String {
        waitFor("the channel to close") { ch.state.value is BleChannelState.Closed }
        return (ch.state.value as BleChannelState.Closed).reason
    }

    // H4: a cancelled connect closes the GATT client, every time.
    @Test fun cancelling_while_connecting_closes_the_gatt_every_time() {
        repeat(10) {
            val ops = FakeGattOps(neverUp = true)
            val ch = channel(ops)
            runBlocking {
                val job = launch(Dispatchers.Default) { ch.open() }
                waitFor("the connect") { ops.connects == 1 }
                job.cancelAndJoin()
            }
            assertEquals(1, ops.closes)
            assertTrue("disconnect before close", ops.log.indexOf("disconnect") < ops.log.indexOf("close"))
            assertTrue(ch.state.value is BleChannelState.Closed)
        }
    }

    // M2: one retry on status 133, after closing the failed client.
    @Test fun status_133_is_retried_once_after_closing() {
        val ops = FakeGattOps(results = listOf(133, 0))
        val ch = opened(ops)
        assertEquals(2, ops.connects)
        assertEquals(listOf("connect", "disconnect", "close", "connect"), ops.log.take(4))
        assertEquals(BleChannelState.Open(247), ch.state.value)
        ch.close()
    }

    @Test fun status_133_twice_gives_up_and_leaves_nothing_open() {
        val ops = FakeGattOps(results = listOf(133, 133, 0))
        val ch = channel(ops)
        assertFalse(runBlocking { ch.open() })
        assertEquals(2, ops.connects)
        assertTrue(ops.closes >= 2)
        assertTrue(ch.state.value is BleChannelState.Closed)
    }

    @Test fun another_failure_status_is_not_retried() {
        val ops = FakeGattOps(results = listOf(8, 0))
        assertFalse(runBlocking { channel(ops).open() })
        assertEquals(1, ops.connects)
    }

    // M2: the state names the reason, from the core when the core ended it, ours otherwise.
    @Test fun the_core_reason_is_the_closed_state() {
        val ops = FakeGattOps()
        val ch = opened(ops)
        ops.events!!.notified(byteArrayOf('X'.code.toByte()))
        assertEquals("The terminal ended the session.", closedReason(ch))
        assertEquals(1, ops.closes)
    }

    @Test fun a_lost_connection_is_a_closed_state() {
        val ops = FakeGattOps()
        val ch = opened(ops)
        ops.events!!.connection(8, false)
        assertEquals(GattChannel.LOST, closedReason(ch))
    }

    @Test fun bluetooth_off_is_a_closed_state() {
        val ch = opened()
        ch.bluetoothOff()
        assertEquals(GattChannel.BT_OFF, closedReason(ch))
    }

    @Test fun closing_here_says_so_and_closes_the_gatt() {
        val ops = FakeGattOps()
        val ch = opened(ops)
        ch.close()
        assertEquals(GattChannel.CLOSED_HERE, closedReason(ch))
        assertTrue(link.byeCalled)
        assertEquals(1, ops.closes)
    }

    @Test fun incoming_completes_after_the_frames_received_before_the_end() {
        val ops = FakeGattOps()
        val ch = opened(ops)
        ops.events!!.notified(byteArrayOf(1))
        ops.events!!.notified(byteArrayOf(2))
        ops.events!!.notified(byteArrayOf('X'.code.toByte()))
        val got = runBlocking { withTimeout(5_000) { ch.incoming.toList() } }
        assertEquals(listOf(1, 2), got.map { it[0].toInt() })
    }

    // M2: the terminal's refusal sentence reaches the caller, once.
    @Test fun the_send_failure_sentence_is_returned_once() {
        val ch = opened()
        link.failure = "The terminal is busy."
        assertTrue(runBlocking { ch.send(byteArrayOf(9)) })
        var sentence: String? = null
        waitFor("the sentence") { sentence = ch.sendFailure() ?: sentence; sentence != null }
        assertEquals("The terminal is busy.", sentence)
        assertNull(ch.sendFailure())
    }

    // M1: every notification reaches the link exactly once, in order, across many ticks and timeouts.
    @Test fun no_notification_is_lost_across_ticks() {
        val ops = FakeGattOps()
        val ch = opened(ops)
        scope.launch { ch.incoming.collect { } } // the consumer: the inbox is bounded, a channel nobody reads is a flood
        val n = 3_000
        for (i in 0 until n) {
            ops.events!!.notified(byteArrayOf((i shr 8).toByte(), i.toByte()))
            if (i % 100 == 99) waitFor("the link to catch up") { link.got.size == i + 1 }
            if (i % 700 == 0) Thread.sleep(250) // longer than a tick: the loop sits in its timeout
        }
        waitFor("all notifications") { link.got.size == n }
        assertEquals((0 until n).toList(), link.got.map { ((it[0].toInt() and 0xff) shl 8) or (it[1].toInt() and 0xff) })
        ch.close()
    }

    // L1: a flood the loop cannot keep up with ends the session; the queues stay bounded.
    @Test fun a_flood_closes_the_session() {
        val ops = FakeGattOps()
        val ch = opened(ops)
        link.slowMs = 2
        repeat(5_000) { ops.events!!.notified(byteArrayOf(1)) }
        assertEquals(GattChannel.FLOOD, closedReason(ch))
        assertTrue(link.got.size < 5_000)
    }

    // L1 (security): after N refused frames the session is closed.
    @Test fun too_many_refused_frames_close_the_session() {
        val ch = opened(refusalLimit = 3)
        ch.frameRefused()
        ch.frameRefused()
        assertTrue(ch.state.value is BleChannelState.Open)
        ch.frameRefused()
        assertEquals(GattChannel.REFUSED, closedReason(ch))
        assertTrue(link.byeCalled)
    }
}
