package com.aiyu.switchboard

import com.aiyu.switchboard.net.Cmd
import com.aiyu.switchboard.net.POSTPONE
import com.aiyu.switchboard.net.WARN_BEFORE
import com.aiyu.switchboard.net.postponeDelay
import com.aiyu.switchboard.net.timerBody
import com.aiyu.switchboard.net.timerTitle
import com.aiyu.switchboard.net.warnTime
import com.aiyu.switchboard.net.warningTitle
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.time.ZoneId
import java.time.ZonedDateTime

class ShutdownTimerTest {
    private val zone = ZoneId.of("America/Sao_Paulo")

    /** A Monday evening: 22:10:30. */
    private val now = ZonedDateTime.of(2026, 11, 16, 22, 10, 30, 0, zone).toEpochSecond()

    @Test fun warnsFiveMinutesBefore() {
        assertEquals(300L, WARN_BEFORE)
        assertEquals(now + 3300, warnTime(now + 3600, now))
        assertEquals(now + 1, warnTime(now + 301, now))
        // Set, or first seen, inside the last five minutes: that moment is gone, so no warning is due.
        assertNull(warnTime(now + 300, now))
        assertNull(warnTime(now + 60, now))
        assertNull(warnTime(now - 10, now))
    }

    @Test fun postponingAddsHalfAnHourToTheTimeSet() {
        assertEquals(1800L, POSTPONE)
        // Five minutes left: 35 minutes from now.
        assertEquals(2100, postponeDelay(now + 300, now))
        // Tapped on the first notification, with an hour to go.
        assertEquals(5400, postponeDelay(now + 3600, now))
        // The time has just passed on this phone's clock: half an hour from now, not less.
        assertEquals(1800, postponeDelay(now - 20, now))
        // Never more than the day the computer takes.
        assertEquals(86_400, postponeDelay(now + 86_000, now))
        // And it is asked for with the ordinary command, leaving the plug's wait as it was.
        assertEquals("""{"type":"total_shutdown","delay":2100}""", Cmd.totalShutdown(postponeDelay(now + 300, now)).toString())
    }

    @Test fun saysWhichComputerAndWhen() {
        assertEquals("Desk shuts down at 23:30", timerTitle("Desk", now + 4770, zone))
        assertEquals("Desk is about to shut down", warningTitle("Desk"))
        assertEquals("Total shutdown. The plug cuts the power at 23:35.", timerBody(now + 4770, 300, zone))
        // Shutting down today, cut after midnight.
        assertEquals("Total shutdown. The plug cuts the power at 00:10.", timerBody(now + 6270, 900, zone))
    }
}
