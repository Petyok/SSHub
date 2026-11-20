package io.github.petyok.sshub.recovery

import org.junit.Assert.assertEquals
import org.junit.Test
import io.github.petyok.sshub.ui.screens.recovery.formatAgo
import io.github.petyok.sshub.ui.screens.recovery.formatRemaining

class RecoveryFormatTest {
    @Test fun remaining_uses_days_hours_minutes() {
        assertEquals("6 d 3 h", formatRemaining(6 * 86_400L + 3 * 3_600))
        assertEquals("5 h 12 min", formatRemaining(5 * 3_600L + 12 * 60))
        assertEquals("12 min", formatRemaining(12 * 60L))
        assertEquals("1 min", formatRemaining(5))
    }

    @Test fun remaining_at_or_below_zero_is_ended() {
        assertEquals("ended", formatRemaining(0))
        assertEquals("ended", formatRemaining(-30))
    }

    @Test fun ago_uses_minutes_hours_days() {
        assertEquals("5 min ago", formatAgo(5 * 60L))
        assertEquals("21 h ago", formatAgo(21 * 3_600L))
        assertEquals("3 d ago", formatAgo(3 * 86_400L))
    }
}
