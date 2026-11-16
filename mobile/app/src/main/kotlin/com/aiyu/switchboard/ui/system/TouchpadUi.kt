package com.aiyu.switchboard.ui.system

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.Mouse
import androidx.compose.material3.AssistChip
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.unit.dp
import com.aiyu.switchboard.net.Cmd
import com.aiyu.switchboard.net.HostConnection
import com.aiyu.switchboard.net.HostState
import com.aiyu.switchboard.ui.kit.MonoCaption
import com.aiyu.switchboard.ui.kit.PodHeader
import com.aiyu.switchboard.ui.kit.SectionCard
import kotlinx.coroutines.delay

/** Movement waiting to be sent: a finger can produce hundreds of events a second, the link needs far fewer. */
internal class Pending {
    var dx = 0f
    var dy = 0f

    /** Finger travel not yet sent as wheel steps; positive scrolls down. */
    private var scroll = 0f

    /** Two fingers on the pad: dragging up scrolls down, as on a phone. */
    fun twoFingerDrag(dy: Float) {
        scroll -= dy
    }

    /** One finger on the side bar: sliding down scrolls down, as on a scrollbar. */
    fun barDrag(dy: Float) {
        scroll += dy
    }

    /** Whole wheel steps ready to send; what is left of a step stays for the next call. */
    fun takeScrollSteps(): Int {
        val steps = (scroll / SCROLL_STEP).toInt()
        scroll -= steps * SCROLL_STEP
        return steps
    }
}

/** Pixels of finger travel per scroll step. */
internal const val SCROLL_STEP = 36f
private const val SPEED = 1.6f

/** Home card: a way in when the computer allows it, else what to switch on. */
@Composable
fun InputPod(s: HostState, onOpen: () -> Unit) {
    SectionCard {
        PodHeader(
            Icons.Rounded.Mouse,
            "Touchpad & keyboard",
            if (s.remoteInput) "Use this phone as the computer's mouse and keyboard" else "Turned off on the computer",
            shape = MaterialShapes.Cookie6Sided,
            action = if (s.remoteInput) "Open" else null,
            onAction = if (s.remoteInput) onOpen else null,
        )
    }
}

@Composable
fun TouchpadContent(s: HostState, conn: HostConnection) {
    if (!s.remoteInput) {
        SectionCard {
            PodHeader(Icons.Rounded.Mouse, "Turned off", "Allow it in Settings on the computer", shape = MaterialShapes.Cookie6Sided)
            Spacer(Modifier.height(8.dp))
            Text(
                "Anything typed from a phone runs as the person signed in, so the computer keeps this off until you switch it on in its Switchboard settings.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        return
    }
    Touchpad(conn)
    MouseButtons(conn)
    KeyboardCard(conn)
}

@Composable
private fun Touchpad(conn: HostConnection) {
    val cs = MaterialTheme.colorScheme
    val haptic = LocalHapticFeedback.current
    val pending = remember { Pending() }
    // One sender: flush what has gathered about every 16 ms.
    LaunchedEffect(conn) {
        while (true) {
            delay(16)
            val dx = pending.dx.toInt()
            val dy = pending.dy.toInt()
            if (dx != 0 || dy != 0) {
                pending.dx -= dx
                pending.dy -= dy
                conn.post(Cmd.mouseMove(dx, dy))
            }
            val steps = pending.takeScrollSteps()
            if (steps != 0) conn.post(Cmd.scroll(0, steps))
        }
    }
    SectionCard {
        PodHeader(Icons.Rounded.Mouse, "Touchpad", "Drag to move, tap to click, two fingers to scroll or right-click, side bar to scroll", shape = MaterialShapes.Cookie9Sided)
        Spacer(Modifier.height(12.dp))
        Row(Modifier.fillMaxWidth().height(260.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(
            Modifier
                .weight(1f)
                .fillMaxHeight()
                .background(cs.surfaceContainerHighest, RoundedCornerShape(24.dp))
                .border(1.dp, cs.outlineVariant, RoundedCornerShape(24.dp))
                .pointerInput(conn) {
                    awaitEachGesture {
                        val down = awaitFirstDown(requireUnconsumed = false)
                        val start = down.position
                        val started = down.uptimeMillis
                        var moved = false
                        var twoFingers = false
                        while (true) {
                            val event = awaitPointerEvent()
                            val held = event.changes.filter { it.pressed }
                            if (held.isEmpty()) {
                                // Everything lifted: a short touch that did not travel is a click.
                                val quick = event.changes.maxOf { it.uptimeMillis } - started < 260
                                if (!moved && quick) {
                                    haptic.performHapticFeedback(HapticFeedbackType.ContextClick)
                                    conn.post(Cmd.mouseButton(if (twoFingers) "right" else "left", "click"))
                                }
                                break
                            }
                            if (held.size >= 2) twoFingers = true
                            val travelled = (held[0].position - start).getDistance() > 10f
                            if (twoFingers) {
                                val dy = held.map { it.position.y - it.previousPosition.y }.average().toFloat()
                                pending.twoFingerDrag(dy)
                                if (travelled) moved = true
                            } else {
                                val d: Offset = held[0].position - held[0].previousPosition
                                pending.dx += d.x * SPEED
                                pending.dy += d.y * SPEED
                                if (travelled) moved = true
                            }
                            event.changes.forEach { it.consume() }
                        }
                    }
                },
            contentAlignment = Alignment.Center,
        ) {
            MonoCaption("touchpad")
        }
        ScrollBar(pending)
        }
    }
}

/** A strip beside the pad: one finger sliding along it turns the wheel. */
@Composable
private fun ScrollBar(pending: Pending) {
    val cs = MaterialTheme.colorScheme
    Box(
        Modifier
            .width(44.dp)
            .fillMaxHeight()
            .background(cs.surfaceContainerHighest, RoundedCornerShape(22.dp))
            .border(1.dp, cs.outlineVariant, RoundedCornerShape(22.dp))
            .pointerInput(pending) {
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    down.consume()
                    while (true) {
                        val event = awaitPointerEvent()
                        val change = event.changes.firstOrNull { it.id == down.id }
                        if (change == null || !change.pressed) break
                        pending.barDrag(change.position.y - change.previousPosition.y)
                        event.changes.forEach { it.consume() }
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.width(4.dp).height(40.dp).background(cs.onSurfaceVariant.copy(alpha = 0.5f), RoundedCornerShape(2.dp)))
    }
}

@Composable
private fun MouseButtons(conn: HostConnection) {
    val cs = MaterialTheme.colorScheme
    val haptic = LocalHapticFeedback.current
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        listOf("left" to "Left", "middle" to "Middle", "right" to "Right").forEach { (button, label) ->
            Box(
                Modifier
                    .weight(1f)
                    .height(52.dp)
                    .background(cs.secondaryContainer, RoundedCornerShape(18.dp))
                    // Hold to hold the button down on the computer, so a second finger on the pad can drag.
                    .pointerInput(conn) {
                        detectTapGestures(onPress = {
                            haptic.performHapticFeedback(HapticFeedbackType.ContextClick)
                            conn.post(Cmd.mouseButton(button, "press"))
                            tryAwaitRelease()
                            conn.post(Cmd.mouseButton(button, "release"))
                        })
                    },
                contentAlignment = Alignment.Center,
            ) { Text(label, style = MaterialTheme.typography.labelLarge, color = cs.onSecondaryContainer) }
        }
    }
}

private val SPECIAL = listOf(
    "Esc" to "escape", "Tab" to "tab", "Enter" to "enter", "⌫" to "backspace", "Del" to "delete",
    "←" to "left", "↑" to "up", "↓" to "down", "→" to "right",
    "Home" to "home", "End" to "end", "PgUp" to "page_up", "PgDn" to "page_down",
)

private val SHORTCUTS = listOf("Copy" to 'c', "Paste" to 'v', "Cut" to 'x', "All" to 'a', "Undo" to 'z', "Find" to 'f')

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun KeyboardCard(conn: HostConnection) {
    var text by remember { mutableStateOf("") }
    var ctrl by remember { mutableStateOf(false) }
    var alt by remember { mutableStateOf(false) }
    var shift by remember { mutableStateOf(false) }
    var meta by remember { mutableStateOf(false) }
    fun release() {
        ctrl = false; alt = false; shift = false; meta = false
    }
    SectionCard {
        PodHeader(Icons.Rounded.Keyboard, "Keyboard", "What you type here is typed on the computer", shape = MaterialShapes.Clover4Leaf)
        Spacer(Modifier.height(12.dp))
        OutlinedTextField(
            value = text,
            onValueChange = { new ->
                // Send only what changed: added characters as text, removed ones as Backspace.
                val common = text.commonPrefixWith(new).length
                val removed = text.length - common
                repeat(removed) { conn.post(Cmd.key("backspace")) }
                val added = new.substring(common)
                if (added.isNotEmpty()) conn.post(Cmd.typeText(added))
                // Keep the field short; the computer already has the text.
                text = if (new.length > 40) new.takeLast(20) else new
            },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            placeholder = { Text("Type here") },
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send, capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
            keyboardActions = KeyboardActions(onSend = {
                conn.post(Cmd.key("enter"))
                text = ""
            }),
        )
        Spacer(Modifier.height(10.dp))
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterChip(ctrl, { ctrl = !ctrl }, label = { Text("Ctrl") })
            FilterChip(alt, { alt = !alt }, label = { Text("Alt") })
            FilterChip(shift, { shift = !shift }, label = { Text("Shift") })
            FilterChip(meta, { meta = !meta }, label = { Text("Win") })
        }
        Spacer(Modifier.height(6.dp))
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SPECIAL.forEach { (label, key) ->
                AssistChip(onClick = { conn.post(Cmd.key(key, ctrl, alt, shift, meta)); release() }, label = { Text(label) })
            }
        }
        Spacer(Modifier.height(6.dp))
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SHORTCUTS.forEach { (label, ch) ->
                AssistChip(onClick = { conn.post(Cmd.chord(ch, ctrl = true, alt = alt, shift = shift, meta = meta)); release() }, label = { Text("Ctrl+${ch.uppercaseChar()} $label") })
            }
            AssistChip(onClick = { conn.post(Cmd.key("tab", alt = true)) }, label = { Text("Alt+Tab") })
        }
        Spacer(Modifier.height(4.dp))
        MonoCaption("Ctrl, Alt, Shift and Win apply to the next key.", modifier = Modifier.padding(top = 4.dp))
    }
}
