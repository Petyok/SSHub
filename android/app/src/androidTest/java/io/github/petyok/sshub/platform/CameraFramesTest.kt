package io.github.petyok.sshub.platform

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer

class CameraFramesTest {
    private fun plane(w: Int, h: Int, stride: Int): ByteBuffer {
        val b = ByteArray(stride * h)
        for (y in 0 until h) for (x in 0 until stride) b[y * stride + x] = if (x < w) (y * 16 + x).toByte() else 0x7f
        return ByteBuffer.wrap(b)
    }

    private fun expected(w: Int, h: Int) = ByteArray(w * h) { ((it / w) * 16 + it % w).toByte() }

    // M8: one buffer per resolution when the stride equals the width.
    @Test fun the_buffer_is_reused_while_the_resolution_stays() {
        val c = LumaCopier()
        val a = c.copy(plane(8, 4, 8), 8, 4, 8)
        assertArrayEquals(expected(8, 4), a)
        val b = c.copy(plane(8, 4, 8), 8, 4, 8)
        assertSame(a, b)
        val d = c.copy(plane(16, 4, 16), 16, 4, 16)
        assertNotSame(a, d)
        assertEquals(64, d.size)
    }

    @Test fun padding_at_the_end_of_a_row_is_not_copied() {
        val c = LumaCopier()
        assertArrayEquals(expected(8, 4), c.copy(plane(8, 4, 12), 8, 4, 12))
    }

    @Test fun a_failing_frame_is_dropped_not_thrown() {
        var after = false
        dropOnFailure { throw IllegalStateException("decoder panic") }
        dropOnFailure { throw OutOfMemoryError("no memory") }
        dropOnFailure { after = true }
        assertTrue("the next frame still runs", after)
    }
}
