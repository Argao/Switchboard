package com.aiyu.switchboard

import com.aiyu.switchboard.net.AppWindow
import com.aiyu.switchboard.net.Cmd
import com.aiyu.switchboard.net.Event
import com.aiyu.switchboard.net.Screen
import com.aiyu.switchboard.net.WireJson
import com.aiyu.switchboard.ui.system.appsSummary
import com.aiyu.switchboard.ui.system.screenDetail
import com.aiyu.switchboard.ui.system.screenOf
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AppsTest {
    private fun state(extra: String) = WireJson.decodeFromString(
        Event.serializer(),
        """{"type":"state","volume":null,"displays":[],$extra"media":null,"stats":{"cpu":0.0,"mem_used":0,"mem_total":0,"uptime":0,"battery":null}}""",
    ) as Event.State

    private val main = Screen("""\\.\DISPLAY1""", "AW2725DM", true, 2560, 1440)
    private val side = Screen("""\\.\DISPLAY2""", "ASUS VG249", false, 1920, 1080)

    @Test fun readsWindowsAndScreens() {
        val s = state(
            """"windows":[{"id":"66312","app":"firefox","title":"Docs","screen":"\\\\.\\DISPLAY2","fullscreen":true},{"id":"7","app":"Code","title":"main.rs","screen":null,"fullscreen":false}],""" +
                """"screens":[{"id":"\\\\.\\DISPLAY1","name":"AW2725DM","primary":true,"width":2560,"height":1440},{"id":"\\\\.\\DISPLAY2","name":"ASUS VG249","primary":false,"width":1920,"height":1080}],""",
        )
        assertEquals(listOf(AppWindow("66312", "firefox", "Docs", side.id, true), AppWindow("7", "Code", "main.rs", null, false)), s.windows)
        assertEquals(listOf(main, side), s.screens)
    }

    @Test fun aComputerThatCannotManageWindowsIsNotOneWithNothingOpen() {
        // An older computer, or one on a system without support, sends no `windows` at all.
        assertNull(state("").windows)
        assertNull(state(""""windows":null,"screens":[],""").windows)
        assertEquals(emptyList<AppWindow>(), state(""""windows":[],"screens":[],""").windows)
    }

    @Test fun commands() {
        assertEquals("""{"type":"close_window","window":"66312"}""", Cmd.closeWindow("66312").toString())
        assertEquals("""{"type":"kill_app","window":"66312"}""", Cmd.killApp("66312").toString())
        assertEquals("""{"type":"set_window_fullscreen","window":"7","screen":"\\\\.\\DISPLAY2","on":true}""", Cmd.setWindowFullscreen("7", side.id, true).toString())
        // No screen named: the one it is on, or none needed to leave full screen.
        assertEquals("""{"type":"set_window_fullscreen","window":"7","on":true}""", Cmd.setWindowFullscreen("7", null, true).toString())
        assertEquals("""{"type":"set_window_fullscreen","window":"7","on":false}""", Cmd.setWindowFullscreen("7", null, false).toString())
    }

    @Test fun summaryCountsWindowsAndApps() {
        fun w(app: String) = AppWindow("1", app, "t")
        assertEquals("Nothing open", appsSummary(emptyList()))
        assertEquals("1 window · 1 app", appsSummary(listOf(w("Code"))))
        // Two windows of one program are one app, however it spells its name.
        assertEquals("3 windows · 2 apps", appsSummary(listOf(w("Zoom"), w("zoom"), w("firefox"))))
    }

    @Test fun screensAreToldApart() {
        assertEquals("2560×1440 · Main screen", screenDetail(main))
        assertEquals("1920×1080", screenDetail(side))
        assertEquals("", screenDetail(Screen("x", "Screen 1")))
        val w = AppWindow("1", "Code", "t", side.id)
        assertEquals("ASUS VG249", screenOf(w, listOf(main, side)))
        // With a single screen, or one that has gone, there is nothing worth saying.
        assertNull(screenOf(w, listOf(side)))
        assertNull(screenOf(w.copy(screen = "gone"), listOf(main, side)))
        assertNull(screenOf(w.copy(screen = null), listOf(main, side)))
    }
}
