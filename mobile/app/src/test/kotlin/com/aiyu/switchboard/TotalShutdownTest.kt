package com.aiyu.switchboard

import com.aiyu.switchboard.net.Cmd
import com.aiyu.switchboard.net.Event
import com.aiyu.switchboard.net.TotalShutdown
import com.aiyu.switchboard.net.WireJson
import com.aiyu.switchboard.ui.system.TOTAL_DELAYS
import com.aiyu.switchboard.ui.system.TOTAL_WAITS
import com.aiyu.switchboard.ui.system.boxesOf
import com.aiyu.switchboard.ui.system.clockText
import com.aiyu.switchboard.ui.system.countdown
import com.aiyu.switchboard.ui.system.delayOf
import com.aiyu.switchboard.ui.system.spanText
import com.aiyu.switchboard.ui.system.timerText
import com.aiyu.switchboard.ui.system.totalSummary
import com.aiyu.switchboard.ui.system.totalWarning
import com.aiyu.switchboard.ui.system.waitChoices
import com.aiyu.switchboard.ui.system.waitText
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.ZoneId
import java.time.ZonedDateTime

class TotalShutdownTest {
    private fun state(extra: String) = WireJson.decodeFromString(
        Event.serializer(),
        """{"type":"state","volume":null,"displays":[],$extra"media":null,"stats":{"cpu":0.0,"mem_used":0,"mem_total":0,"uptime":0,"battery":null}}""",
    ) as Event.State

    private val zone = ZoneId.of("America/Sao_Paulo")

    /** A Monday evening: 22:10:30. */
    private val evening = ZonedDateTime.of(2026, 11, 16, 22, 10, 30, 0, zone)
    private val now = evening.toEpochSecond()

    @Test fun readsWhatTheComputerSends() {
        val t = state(""""total_shutdown":{"at":1800000600,"cut_after":300,"cut_at":null,"pending_update":false,"error":null},""").totalShutdown
        assertEquals(TotalShutdown(at = 1_800_000_600, cutAfter = 300), t)
        val t2 = state(""""total_shutdown":{"at":null,"cut_after":120,"cut_at":1800000120,"pending_update":true,"error":"It was skipped"},""").totalShutdown
        assertEquals(TotalShutdown(null, 120, 1_800_000_120, true, "It was skipped"), t2)
    }

    @Test fun aComputerWithNoPlugOffersNothing() {
        // An older computer, or one with no plug set up, sends nothing or null.
        assertNull(state("").totalShutdown)
        assertNull(state(""""total_shutdown":null,""").totalShutdown)
    }

    @Test fun commands() {
        assertEquals("""{"type":"total_shutdown","delay":0}""", Cmd.totalShutdown(0).toString())
        assertEquals("""{"type":"total_shutdown","delay":1800}""", Cmd.totalShutdown(1800).toString())
        // With the plug's wait chosen on the phone.
        assertEquals("""{"type":"total_shutdown","delay":1800,"cut_after":600}""", Cmd.totalShutdown(1800, 600).toString())
        // Kept to what the computer takes: not in the past, not more than a day ahead, a wait of one to thirty minutes.
        assertEquals("""{"type":"total_shutdown","delay":0}""", Cmd.totalShutdown(-5).toString())
        assertEquals("""{"type":"total_shutdown","delay":86400}""", Cmd.totalShutdown(999_999).toString())
        assertEquals("""{"type":"total_shutdown","delay":0,"cut_after":60}""", Cmd.totalShutdown(0, 5).toString())
        assertEquals("""{"type":"total_shutdown","delay":0,"cut_after":1800}""", Cmd.totalShutdown(0, 99_999).toString())
        assertEquals("""{"type":"cancel_total_shutdown"}""", Cmd.cancelTotalShutdown().toString())
    }

    @Test fun countsDown() {
        assertEquals("0:07", countdown(7))
        assertEquals("12:34", countdown(754))
        assertEquals("1:05:09", countdown(3909))
        assertEquals("24:00:00", countdown(86_400))
        // A phone clock a little ahead of the computer's must not show a negative time.
        assertEquals("0:00", countdown(-30))
    }

    @Test fun saysHowLongThePlugWaits() {
        assertEquals("5 min", waitText(300))
        assertEquals("1 min", waitText(60))
        assertEquals("90 s", waitText(90))
        // The choices are in order and inside what the computer takes.
        assertEquals(TOTAL_WAITS.sorted(), TOTAL_WAITS)
        assertTrue(TOTAL_WAITS.all { it in 60..1800 })
        assertEquals(TOTAL_WAITS, waitChoices(300))
        // A wait set from the computer that is none of them is still shown, in its place.
        assertEquals(listOf(60, 120, 300, 420, 600, 900, 1800), waitChoices(420))
    }

    @Test fun saysAStretchOfTimeToTheMinute() {
        assertEquals("less than a minute", spanText(0))
        assertEquals("1 min", spanText(1))
        assertEquals("45 min", spanText(45 * 60))
        assertEquals("2 h", spanText(7200))
        assertEquals("1 h 12 min", spanText(4290))
        assertEquals("24 h", spanText(86_400))
    }

    @Test fun anyDelayUpToADayCanBeTyped() {
        assertEquals(4800, delayOf("1", "20"))
        // Either box may be left empty, and minutes alone may run past the hour.
        assertEquals(7200, delayOf("2", ""))
        assertEquals(60, delayOf("", "1"))
        assertEquals(5400, delayOf("", "90"))
        assertEquals(47 * 60, delayOf(" 0 ", " 47 "))
        assertEquals(86_400, delayOf("24", "0"))
        assertEquals(86_400, delayOf("", "1440"))
        // Nothing, no time at all (that is "Now"), more than a day, and what is not a number.
        for ((h, m) in listOf("" to "", "0" to "0", "24" to "1", "25" to "", "" to "1441", "x" to "5", "1" to "-5", "1.5" to "")) {
            assertNull("$h:$m", delayOf(h, m))
        }
    }

    @Test fun aDelayFillsTheBoxes() {
        assertEquals("0" to "30", boxesOf(1800))
        assertEquals("1" to "20", boxesOf(4800))
        // Time left on one that is set is rounded up, so changing it without typing does not shorten it.
        assertEquals("1" to "20", boxesOf(4770))
        assertEquals("0" to "1", boxesOf(5))
        assertEquals("24" to "0", boxesOf(999_999))
        assertEquals("0" to "0", boxesOf(-10))
        // What the boxes show can be set again as it is.
        assertEquals(4800, boxesOf(4770).let { (h, m) -> delayOf(h, m) })
    }

    @Test fun showsClockTimesOnThisPhonesClock() {
        assertEquals("22:10", clockText(now, zone))
        assertEquals("01:10", clockText(now, ZoneId.of("UTC")))
        assertEquals("23:30", clockText(now + 4770, zone))
    }

    @Test fun summaryFollowsWhatIsHappening() {
        assertEquals("Shut down, then cut the power at the plug", totalSummary(TotalShutdown(), now, zone))
        assertEquals("Shuts down at 23:30, in 1:19:30", totalSummary(TotalShutdown(at = now + 4770), now, zone))
        assertEquals("Shutting down. Power cut at 22:12, in 2:00", totalSummary(TotalShutdown(cutAt = now + 120), now, zone))
        // Once the shutdown was sent, that is what matters, whatever else is set.
        assertEquals("Shutting down. Power cut at 22:10, in 0:05", totalSummary(TotalShutdown(at = now, cutAt = now + 5, pendingUpdate = true), now, zone))
        assertEquals("Not now: an update is waiting to install", totalSummary(TotalShutdown(pendingUpdate = true), now, zone))
    }

    @Test fun theQuestionSaysWhatWillHappen() {
        val at = totalWarning(0, 300)
        assertTrue(at, at.startsWith("The computer shuts down now. 5 min after that, its smart plug cuts the power"))
        val later = totalWarning(1800, 120)
        assertTrue(later, later.startsWith("The computer shuts down in 30 min. 2 min after that"))
        // The first choice is at once; the rest are later, in order.
        assertEquals(0, TOTAL_DELAYS.first().first)
        assertEquals(TOTAL_DELAYS.map { it.first }.sorted(), TOTAL_DELAYS.map { it.first })
        assertTrue(TOTAL_DELAYS.all { it.first <= 86_400 })
    }

    @Test fun theTimerSaysBothMoments() {
        // From 22:10:30.
        assertEquals("Shuts down in 1 h 20 min, at 23:30. The plug cuts the power at 23:35.", timerText(4800, 300, evening))
        // Past midnight it says so.
        assertEquals("Shuts down in 8 h, at 06:10 tomorrow. The plug cuts the power at 06:40.", timerText(8 * 3600, 1800, evening))
        // Shutting down today, cut after midnight.
        assertEquals("Shuts down in 1 h 45 min, at 23:55. The plug cuts the power at 00:10.", timerText(6300, 900, evening))
    }
}
