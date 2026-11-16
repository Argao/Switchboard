package com.aiyu.switchboard.ui.system

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Timer
import androidx.compose.material.icons.rounded.PowerOff
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.aiyu.switchboard.net.Cmd
import com.aiyu.switchboard.net.HostConnection
import com.aiyu.switchboard.net.TotalShutdown
import com.aiyu.switchboard.ui.kit.MonoCaption
import com.aiyu.switchboard.ui.kit.PodHeader
import com.aiyu.switchboard.ui.kit.SectionCard
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.time.Instant
import java.time.ZoneId
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter

/** Quick starts: seconds from now, and what the button says. Any other delay is typed in. */
internal val TOTAL_DELAYS = listOf(0 to "Now", 900 to "In 15 min", 1800 to "In 30 min", 3600 to "In 1 h")

/** How long the plug can be told to wait after the shutdown, in seconds. The computer takes one to thirty minutes. */
internal val TOTAL_WAITS = listOf(60, 120, 300, 600, 900, 1800)

/** The waits on offer, with the one in use added when it is none of them (it was set from the computer). */
internal fun waitChoices(current: Int): List<Int> = (TOTAL_WAITS + current).distinct().sorted()

/** `1:05:09`, `12:34`, `0:07`; never below zero. */
internal fun countdown(secs: Long): String {
    val s = secs.coerceAtLeast(0)
    val (h, m, sec) = Triple(s / 3600, (s % 3600) / 60, s % 60)
    return if (h > 0) "%d:%02d:%02d".format(h, m, sec) else "%d:%02d".format(m, sec)
}

/** `5 min`, `90 s`: how long the plug waits, as the computer was told in seconds. */
internal fun waitText(secs: Int): String = if (secs % 60 == 0) "${secs / 60} min" else "$secs s"

/** `1 h 12 min`, `45 min`, `2 h`: a stretch of time said the way a person would, to the minute. */
internal fun spanText(secs: Long): String {
    val minutes = (secs.coerceAtLeast(0) + 59) / 60
    val (h, m) = minutes / 60 to minutes % 60
    return when {
        minutes == 0L -> "less than a minute"
        h == 0L -> "$m min"
        m == 0L -> "$h h"
        else -> "$h h $m min"
    }
}

private val CLOCK = DateTimeFormatter.ofPattern("HH:mm")

/** The time of day of a moment the computer gave in Unix seconds, on this phone's clock. */
internal fun clockText(epochSecs: Long, zone: ZoneId): String = CLOCK.format(Instant.ofEpochSecond(epochSecs).atZone(zone))

/** Longest delay the computer takes: a day. */
internal const val MAX_DELAY = 86_400

/**
 * The delay typed as hours and minutes, in seconds. An empty box counts as
 * zero. Null when it is not a time that can be set: nothing typed, not a
 * number, no time at all (that is "Now"), or more than a day.
 */
internal fun delayOf(hours: String, minutes: String): Int? {
    val h = if (hours.isBlank()) 0 else hours.trim().toIntOrNull() ?: return null
    val m = if (minutes.isBlank()) 0 else minutes.trim().toIntOrNull() ?: return null
    if (h < 0 || m < 0 || h > 24 || m > 24 * 60) return null
    return (h * 3600 + m * 60).takeIf { it in 60..MAX_DELAY }
}

/** A delay as the hours and minutes to show in the two boxes; minutes are rounded up so a delay under way is not cut short. */
internal fun boxesOf(delay: Long): Pair<String, String> {
    val minutes = ((delay.coerceIn(0, MAX_DELAY.toLong()) + 59) / 60).toInt()
    return (minutes / 60).toString() to (minutes % 60).toString()
}

/** One line saying where a total shutdown stands at [now] (Unix seconds). */
internal fun totalSummary(t: TotalShutdown, now: Long, zone: ZoneId): String = when {
    t.cutAt != null -> "Shutting down. Power cut at ${clockText(t.cutAt, zone)}, in ${countdown(t.cutAt - now)}"
    t.at != null -> "Shuts down at ${clockText(t.at, zone)}, in ${countdown(t.at - now)}"
    t.pendingUpdate -> "Not now: an update is waiting to install"
    else -> "Shut down, then cut the power at the plug"
}

/** What is about to happen, for the question asked before starting one at once or after a delay. */
internal fun totalWarning(delay: Int, cutAfter: Int): String {
    val whenText = if (delay == 0) "now" else "in ${waitText(delay)}"
    return "The computer shuts down $whenText. ${waitText(cutAfter)} after that, its smart plug cuts the power to it and to everything else on that plug. It cannot be turned back on from here, and unsaved work is lost."
}

/** The two moments a delay leads to, shown under the boxes while it is being typed. */
internal fun timerText(delay: Int, cutAfter: Int, now: ZonedDateTime): String {
    val at = now.plusSeconds(delay.toLong())
    val day = if (at.toLocalDate() == now.toLocalDate()) "" else " tomorrow"
    return "Shuts down in ${spanText(delay.toLong())}, at ${CLOCK.format(at)}$day. The plug cuts the power at ${CLOCK.format(at.plusSeconds(cutAfter.toLong()))}."
}

/** Shut down and then cut the power at the smart plug: at once, or after any delay up to a day. Shown only when the computer has a plug set up. */
@Composable
fun TotalShutdownPod(t: TotalShutdown, conn: HostConnection) {
    val cs = MaterialTheme.colorScheme
    val scope = rememberCoroutineScope()
    val zone = remember { ZoneId.systemDefault() }
    var ask by remember { mutableStateOf<Int?>(null) }
    var setTimer by remember { mutableStateOf(false) }
    var failed by remember { mutableStateOf<String?>(null) }
    var now by remember { mutableLongStateOf(System.currentTimeMillis() / 1000) }
    // Chosen here and sent with the request; starts from what the computer last used.
    var wait by remember(t.cutAfter) { mutableIntStateOf(t.cutAfter) }
    val counting = t.at != null || t.cutAt != null
    // The computer sends a time, not a countdown; tick here while there is one to show.
    LaunchedEffect(counting) {
        while (counting) {
            now = System.currentTimeMillis() / 1000
            delay(1000)
        }
    }
    val send: (kotlinx.serialization.json.JsonObject) -> Unit = { cmd ->
        scope.launch {
            val r = conn.send(cmd)
            failed = if (r.ok) null else r.error ?: "That did not work"
        }
    }
    SectionCard {
        PodHeader(Icons.Rounded.PowerOff, "Total shutdown", totalSummary(t, now, zone), shape = MaterialShapes.Cookie6Sided)
        Spacer(Modifier.height(12.dp))
        when {
            t.cutAt != null -> FilledTonalButton(onClick = { send(Cmd.cancelTotalShutdown()) }) { Text("Keep the power on") }
            t.at != null -> {
                MonoCaption("Power cut at ${clockText(t.at + t.cutAfter, zone)}, ${waitText(t.cutAfter)} later")
                Spacer(Modifier.height(10.dp))
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilledTonalButton(onClick = { setTimer = true }) { Text("Change") }
                    OutlinedButton(onClick = { send(Cmd.cancelTotalShutdown()) }) { Text("Cancel") }
                }
            }
            t.pendingUpdate -> Text(
                "Installing it during the shutdown can take longer than the plug waits, and cutting the power then could damage the computer. Restart or shut down normally first.",
                style = MaterialTheme.typography.bodySmall,
                color = cs.onSurfaceVariant,
            )
            else -> {
                Text("Shut down", style = MaterialTheme.typography.labelMedium, color = cs.onSurfaceVariant)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TOTAL_DELAYS.forEach { (delay, label) -> AssistChip(onClick = { ask = delay }, label = { Text(label) }) }
                    AssistChip(onClick = { setTimer = true }, label = { Text("Other") }, leadingIcon = { Icon(Icons.Rounded.Timer, null, Modifier.size(18.dp)) })
                }
                Spacer(Modifier.height(8.dp))
                Text("Then the plug waits", style = MaterialTheme.typography.labelMedium, color = cs.onSurfaceVariant)
                WaitChips(wait) { wait = it }
            }
        }
        // What went wrong just now, or why one set earlier did not happen.
        (failed ?: t.error)?.let { Text(it, color = cs.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 8.dp)) }
    }
    ask?.let { delay ->
        Confirm("Shut down and cut the power?", totalWarning(delay, wait), { ask = null }) { send(Cmd.totalShutdown(delay, wait)) }
    }
    if (setTimer) {
        // Changing one that is set starts from the time it has left; a new one from half an hour.
        val start = remember { t.at?.let { it - System.currentTimeMillis() / 1000 } ?: 1800L }
        TimerDialog(start, wait, zone, onDismiss = { setTimer = false }) { delay, cutAfter ->
            wait = cutAfter
            send(Cmd.totalShutdown(delay, cutAfter))
        }
    }
}

@Composable
private fun WaitChips(wait: Int, onPick: (Int) -> Unit) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        waitChoices(wait).forEach { secs -> FilterChip(selected = secs == wait, onClick = { onPick(secs) }, label = { Text(waitText(secs)) }) }
    }
}

/** How long from now, typed as hours and minutes, with the plug's wait under it and what the two add up to. */
@Composable
private fun TimerDialog(start: Long, wait: Int, zone: ZoneId, onDismiss: () -> Unit, onSet: (Int, Int) -> Unit) {
    val cs = MaterialTheme.colorScheme
    val (h0, m0) = remember { boxesOf(start) }
    var hours by remember { mutableStateOf(h0) }
    var minutes by remember { mutableStateOf(m0) }
    var cutAfter by remember { mutableIntStateOf(wait) }
    val delay = delayOf(hours, minutes)
    val digits: (String, Int) -> String = { text, max -> text.filter(Char::isDigit).take(max) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Shut down in") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(hours, { hours = digits(it, 2) }, Modifier.weight(1f), label = { Text("Hours") }, singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
                    OutlinedTextField(minutes, { minutes = digits(it, 4) }, Modifier.weight(1f), label = { Text("Minutes") }, singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
                }
                Spacer(Modifier.height(12.dp))
                Text("Then the plug waits", style = MaterialTheme.typography.labelMedium, color = cs.onSurfaceVariant)
                WaitChips(cutAfter) { cutAfter = it }
                Spacer(Modifier.height(10.dp))
                Text(
                    if (delay != null) timerText(delay, cutAfter, ZonedDateTime.now(zone)) else "Type a time from 1 minute to 24 hours.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = if (delay != null) cs.onSurface else cs.error,
                )
                Spacer(Modifier.height(6.dp))
                Text(
                    "Everything else on that plug loses power too. The computer cannot be turned back on from here, and unsaved work is lost.",
                    style = MaterialTheme.typography.bodySmall,
                    color = cs.onSurfaceVariant,
                )
            }
        },
        confirmButton = { Button(enabled = delay != null, onClick = { onDismiss(); delay?.let { onSet(it, cutAfter) } }) { Text("Set") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
