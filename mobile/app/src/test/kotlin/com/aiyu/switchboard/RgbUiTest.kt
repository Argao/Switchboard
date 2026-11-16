package com.aiyu.switchboard

import androidx.compose.ui.graphics.Color
import com.aiyu.switchboard.net.Cmd
import com.aiyu.switchboard.net.Event
import com.aiyu.switchboard.net.WireJson
import com.aiyu.switchboard.ui.system.rgbColor
import com.aiyu.switchboard.ui.system.rgbEffectName
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RgbUiTest {
    @Test fun everyEffectTheComputerDrawsHasAName() {
        val names = listOf("static", "breathing", "rainbow", "rainbow_wave", "neon_shift").map(::rgbEffectName)
        assertEquals(listOf("Solid color", "Breathing", "Rainbow", "Rainbow wave", "Neon Shift"), names)
        // One a newer computer adds is shown as it was sent.
        assertEquals("aurora", rgbEffectName("aurora"))
    }

    @Test fun readsHexColors() {
        assertEquals(Color(0xFFB30071), rgbColor("#b30071"))
        assertEquals(Color(0xFF0040FF), rgbColor("0040FF"))
        assertEquals(Color.Black, rgbColor("#000000"))
        // Anything else is white.
        for (bad in listOf("", "#fff", "#gggggg", "#ff80000", "red")) assertEquals(bad, Color.White, rgbColor(bad))
    }

    @Test fun powerAndSpeedCommands() {
        assertEquals("""{"type":"set_rgb_power","on":false}""", Cmd.setRgbPower(false).toString())
        assertEquals("""{"type":"set_rgb_power","on":true}""", Cmd.setRgbPower(true).toString())
        assertEquals("""{"type":"set_rgb_speed","level":70}""", Cmd.setRgbSpeed(70).toString())
        // Levels are kept on the slider's scale.
        assertEquals("""{"type":"set_rgb_speed","level":100}""", Cmd.setRgbSpeed(250).toString())
        assertEquals("""{"type":"set_rgb_speed","level":0}""", Cmd.setRgbSpeed(-3).toString())
        assertEquals("""{"type":"set_rgb_brightness","level":0}""", Cmd.setRgbBrightness(-1).toString())
    }

    private fun state(extra: String) = WireJson.decodeFromString(
        Event.serializer(),
        """{"type":"state","volume":null,$extra"media":null,"stats":{"cpu":0.0,"mem_used":0,"mem_total":0,"uptime":0,"battery":null}}""",
    ) as Event.State

    @Test fun noLightingOnAnOlderComputer() {
        assertNull(state(""""displays":[],""").rgb)
        assertNull(state(""""displays":[],"rgb":null,""").rgb)
    }

    @Test fun monitorPowerIsReadPerDisplay() {
        val d = state(
            """"displays":[{"id":"ddc:1","name":"A","kind":"external","brightness":50,"contrast":40,"power":false},""" +
                """{"id":"ddc:2","name":"B","kind":"external","brightness":50,"contrast":null,"power":true},""" +
                """{"id":"internal","name":"Built-in display","kind":"internal","brightness":50,"contrast":null}],""",
        ).displays
        assertEquals(listOf(false, true, null), d.map { it.power })
        // Only an external monitor can be switched; the panel sends no `power` at all.
        assertTrue(d[0].external && d[1].external && !d[2].external)
    }
}
