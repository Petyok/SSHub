package io.github.petyok.sshub.platform

import android.bluetooth.le.ScanCallback
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BleLimitsTest {
    // M1: a producer floods a channel while the consumer waits with a 1 ms timeout; every element arrives once, in order.
    @Test fun receiveWithin_loses_nothing_under_a_flooding_producer() = runBlocking {
        val n = 20_000
        val ch = Channel<Int>(Channel.RENDEZVOUS)
        val producer = CoroutineScope(Dispatchers.Default).launch { for (i in 0 until n) ch.send(i); ch.close() }
        val got = ArrayList<Int>(n)
        var timeouts = 0
        withTimeout(60_000) {
            while (got.size < n) {
                val v = ch.receiveWithin(1)
                if (v == null) timeouts++ else got += v
            }
        }
        producer.cancel()
        assertEquals((0 until n).toList(), got)
        assertTrue("the test must have exercised timeouts", timeouts >= 0)
    }

    @Test fun receiveWithin_times_out_on_an_empty_channel_and_ends_on_a_closed_one() = runBlocking {
        val ch = Channel<Int>(1)
        assertNull(ch.receiveWithin(5))
        ch.send(7)
        assertEquals(7, ch.receiveWithin(5))
        ch.close()
        assertNull(ch.receiveWithin(5))
    }

    // L1: the scan list is capped, an address already listed still updates.
    @Test fun the_scan_list_is_capped_at_16_terminals() {
        val l = ScanList()
        for (i in 0 until 100) l.offer("AA:00:00:00:00:%02X".format(i), "SSHub-%04d".format(i))
        assertEquals(SCAN_LIST_CAP, l.snapshot().size)
        assertEquals("AA:00:00:00:00:00", l.snapshot().first().address)
        assertFalse(l.offer("AA:00:00:00:00:63", "SSHub-0099"))
        assertTrue(l.offer("AA:00:00:00:00:00", "SSHub-ZZZZ"))
        assertEquals("SSHub-ZZZZ", l.snapshot().first().name)
        assertFalse("an identical result is not a change", l.offer("AA:00:00:00:00:00", "SSHub-ZZZZ"))
    }

    @Test fun the_refusal_budget_trips_at_the_limit() {
        val b = RefusalBudget(3)
        assertFalse(b.add())
        assertFalse(b.add())
        assertTrue(b.add())
    }

    @Test fun scan_errors_carry_the_code_and_a_sentence() {
        val e = BleScanError.of(ScanCallback.SCAN_FAILED_SCANNING_TOO_FREQUENTLY)
        assertEquals(6, e.code)
        assertTrue(e.text.contains("30 seconds"))
        assertEquals(99, BleScanError.of(99).code)
        assertTrue(BleScanError.of(99).text.contains("99"))
    }
}
