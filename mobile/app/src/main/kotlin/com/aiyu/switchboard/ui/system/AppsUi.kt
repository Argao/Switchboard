package com.aiyu.switchboard.ui.system

import androidx.annotation.DrawableRes
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Apps
import androidx.compose.material.icons.rounded.Fullscreen
import androidx.compose.material.icons.rounded.FullscreenExit
import androidx.compose.material.icons.rounded.LiveTv
import androidx.compose.material.icons.rounded.Monitor
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.sp
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.aiyu.switchboard.net.AppWindow
import com.aiyu.switchboard.net.Cmd
import com.aiyu.switchboard.net.HostConnection
import com.aiyu.switchboard.net.HostState
import com.aiyu.switchboard.net.Screen
import com.aiyu.switchboard.R
import com.aiyu.switchboard.ui.kit.MonoBadge
import com.aiyu.switchboard.ui.kit.MonoCaption
import com.aiyu.switchboard.ui.kit.PodHeader
import com.aiyu.switchboard.ui.kit.SectionCard
import com.aiyu.switchboard.ui.kit.bouncyClickable
import kotlinx.coroutines.launch

/** "3 windows · 2 apps": one program often has several windows. */
internal fun appsSummary(windows: List<AppWindow>): String {
    if (windows.isEmpty()) return "Nothing open"
    val apps = windows.map { it.app.lowercase() }.distinct().size
    return "${windows.size} window${if (windows.size == 1) "" else "s"} · $apps app${if (apps == 1) "" else "s"}"
}

/** What tells one screen from another when picking: its size, and which is the main one. */
internal fun screenDetail(s: Screen): String = listOfNotNull("${s.width}×${s.height}".takeIf { s.width > 0 && s.height > 0 }, "Main screen".takeIf { s.primary }).joinToString(" · ")

/** The name of the screen a window is on; nothing when there is only one to be on. */
internal fun screenOf(w: AppWindow, screens: List<Screen>): String? = screens.takeIf { it.size > 1 }?.find { it.id == w.screen }?.name

/**
 * How a streaming site looks on its button: its own colours, and its mark,
 * [width] of the button wide (a wordmark needs more of it than a badge).
 * One with no mark to draw shows a few [letters] instead.
 */
internal data class StreamBrand(val name: String, val back: Long, val fore: Long, @DrawableRes val icon: Int? = null, val width: Float = 0.5f, val letters: String = "")

private val STREAMS = mapOf(
    "netflix" to StreamBrand("Netflix", 0xFF141414, 0xFFE50914, R.drawable.ic_stream_netflix),
    "prime_video" to StreamBrand("Prime Video", 0xFF1A98FF, 0xFFFFFFFF, R.drawable.ic_stream_prime_video, 0.62f),
    "disney_plus" to StreamBrand("Disney+", 0xFF0B1B5C, 0xFFFFFFFF, R.drawable.ic_stream_disney_plus, 0.74f),
    "hbo_max" to StreamBrand("HBO Max", 0xFF000000, 0xFFFFFFFF, R.drawable.ic_stream_hbo_max),
    "youtube" to StreamBrand("YouTube", 0xFFFF0000, 0xFFFFFFFF, R.drawable.ic_stream_youtube),
    "globoplay" to StreamBrand("Globoplay", 0xFFF61E22, 0xFFFFFFFF, R.drawable.ic_stream_globoplay, 0.8f),
    "apple_tv" to StreamBrand("Apple TV", 0xFF1C1C1E, 0xFFFFFFFF, R.drawable.ic_stream_apple_tv),
    "paramount_plus" to StreamBrand("Paramount+", 0xFF0064FF, 0xFFFFFFFF, R.drawable.ic_stream_paramount_plus),
    "crunchyroll" to StreamBrand("Crunchyroll", 0xFFF47521, 0xFFFFFFFF, R.drawable.ic_stream_crunchyroll),
    "twitch" to StreamBrand("Twitch", 0xFF9146FF, 0xFFFFFFFF, R.drawable.ic_stream_twitch),
)

/** A site a newer computer offers that this app has not heard of still gets a button, named from its id. */
internal fun streamBrand(id: String): StreamBrand = STREAMS[id] ?: run {
    val name = id.split('_').filter { it.isNotEmpty() }.joinToString(" ") { it.replaceFirstChar(Char::uppercase) }.ifEmpty { "Unknown" }
    StreamBrand(name, 0xFF3A3A3C, 0xFFFFFFFF, letters = name.take(2))
}

/** Buttons in a row of the streaming grid. */
private const val STREAM_COLUMNS = 4

/** The streaming sites, as the buttons on a television remote: one tap opens the site in full screen on the computer. */
@Composable
private fun StreamsCard(streams: List<String>, screens: List<Screen>, run: (kotlinx.serialization.json.JsonObject) -> Unit) {
    // The site waiting for a screen to be chosen.
    var pick by remember { mutableStateOf<String?>(null) }
    // With one screen there is nothing to choose.
    val open: (String) -> Unit = { id -> if (screens.size > 1) pick = id else run(Cmd.openStream(id, screens.firstOrNull()?.id)) }
    SectionCard {
        PodHeader(Icons.Rounded.LiveTv, "Streaming", if (screens.size > 1) "Opens in full screen on the screen you pick" else "Opens in full screen on the computer", shape = MaterialShapes.Pill)
        Spacer(Modifier.height(14.dp))
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            streams.chunked(STREAM_COLUMNS).forEach { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    row.forEach { id -> StreamButton(streamBrand(id), Modifier.weight(1f)) { open(id) } }
                    // A short last row keeps its buttons the size of the others.
                    repeat(STREAM_COLUMNS - row.size) { Spacer(Modifier.weight(1f)) }
                }
            }
        }
    }
    pick?.let { id ->
        ScreenPicker("Open on", streamBrand(id).name, screens, here = null, onDismiss = { pick = null }) { run(Cmd.openStream(id, it.id)) }
    }
}

@Composable
private fun StreamButton(b: StreamBrand, modifier: Modifier, onClick: () -> Unit) {
    Column(modifier.bouncyClickable(onClick = onClick), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            Modifier.fillMaxWidth().aspectRatio(1f).clip(RoundedCornerShape(20.dp)).background(Color(b.back)),
            contentAlignment = Alignment.Center,
        ) {
            if (b.icon != null) {
                Icon(painterResource(b.icon), null, Modifier.fillMaxWidth(b.width).aspectRatio(1f), tint = Color(b.fore))
            } else {
                Text(b.letters, color = Color(b.fore), fontWeight = FontWeight.Black, fontSize = if (b.letters.length > 2) 15.sp else 24.sp, maxLines = 1)
            }
        }
        Spacer(Modifier.height(6.dp))
        Text(b.name, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center)
    }
}

/** Home card: how much is open, and the way in. */
@Composable
fun AppsPod(open: List<AppWindow>, streams: Int, onOpen: () -> Unit) {
    SectionCard {
        PodHeader(Icons.Rounded.Apps, "Apps", listOfNotNull(appsSummary(open), "Streaming".takeIf { streams > 0 }).joinToString(" · "), shape = MaterialShapes.Pill, action = "Manage", onAction = onOpen)
    }
}

/** The Apps screen: the streaming sites to open, then every open window, with what can be done to it. */
@Composable
fun AppsContent(s: HostState, conn: HostConnection) {
    val cs = MaterialTheme.colorScheme
    val windows = s.windows
    val scope = rememberCoroutineScope()
    // The computer may refuse (a program running as administrator); say why.
    var note by remember { mutableStateOf<String?>(null) }
    val run: (kotlinx.serialization.json.JsonObject) -> Unit = { cmd ->
        scope.launch {
            val r = conn.send(cmd)
            note = if (r.ok) null else r.error ?: "That did not work"
        }
    }
    if (s.streams.isNotEmpty()) StreamsCard(s.streams, s.screens, run)
    if (windows == null) {
        note?.let { SectionCard { Text(it, style = MaterialTheme.typography.bodySmall, color = cs.error) } }
        SectionCard { Text("This computer cannot list its open apps yet. For now that works on Windows.", color = cs.onSurfaceVariant) }
        return
    }
    SectionCard {
        PodHeader(Icons.Rounded.Apps, "Open apps", appsSummary(windows), shape = MaterialShapes.Pill)
        note?.let {
            Spacer(Modifier.height(10.dp))
            Text(it, style = MaterialTheme.typography.bodySmall, color = cs.error)
        }
    }
    windows.forEach { w ->
        androidx.compose.runtime.key(w.id) { WindowCard(w, s.screens, run) }
    }
}

@Composable
private fun WindowCard(w: AppWindow, screens: List<Screen>, run: (kotlinx.serialization.json.JsonObject) -> Unit) {
    val cs = MaterialTheme.colorScheme
    var pick by remember { mutableStateOf(false) }
    var askEnd by remember { mutableStateOf(false) }
    // With one screen there is nothing to choose.
    val goFull = { if (screens.size > 1) pick = true else run(Cmd.setWindowFullscreen(w.id, null, true)) }
    SectionCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(w.app, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(w.title, style = MaterialTheme.typography.bodySmall, color = cs.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
                screenOf(w, screens)?.let { MonoCaption(it, modifier = Modifier.padding(top = 4.dp)) }
            }
            if (w.fullscreen) {
                Spacer(Modifier.width(8.dp))
                MonoBadge("FULL SCREEN", color = cs.primaryContainer, content = cs.onPrimaryContainer)
            }
        }
        Spacer(Modifier.height(12.dp))
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (w.fullscreen) {
                FilledTonalButton(onClick = { run(Cmd.setWindowFullscreen(w.id, null, false)) }) {
                    Icon(Icons.Rounded.FullscreenExit, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("Exit full screen")
                }
                if (screens.size > 1) OutlinedButton(onClick = { pick = true }) { Text("Other screen") }
            } else {
                FilledTonalButton(onClick = goFull) {
                    Icon(Icons.Rounded.Fullscreen, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("Full screen")
                }
            }
            OutlinedButton(onClick = { run(Cmd.closeWindow(w.id)) }) { Text("Close") }
            TextButton(onClick = { askEnd = true }, colors = ButtonDefaults.textButtonColors(contentColor = cs.error)) { Text("End task") }
        }
    }
    if (pick) ScreenPicker("Full screen on", w.app, screens, here = w.screen, onDismiss = { pick = false }) { run(Cmd.setWindowFullscreen(w.id, it.id, true)) }
    if (askEnd) Confirm("End ${w.app}?", "It stops at once, with every window it has open. Anything not saved is lost.", { askEnd = false }) { run(Cmd.killApp(w.id)) }
}

/** Asks which screen; [here] is the one the thing is on now, when it is on one. */
@Composable
private fun ScreenPicker(title: String, what: String, screens: List<Screen>, here: String?, onDismiss: () -> Unit, onPick: (Screen) -> Unit) {
    val cs = MaterialTheme.colorScheme
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(what, style = MaterialTheme.typography.bodyMedium, color = cs.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Spacer(Modifier.height(4.dp))
                screens.forEach { s ->
                    Row(
                        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).clickable { onDismiss(); onPick(s) }.padding(vertical = 10.dp, horizontal = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(Icons.Rounded.Monitor, null, tint = if (s.id == here) cs.primary else cs.onSurfaceVariant)
                        Spacer(Modifier.width(14.dp))
                        Column {
                            Text(s.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            val detail = listOfNotNull(screenDetail(s).takeIf { it.isNotEmpty() }, "It is here now".takeIf { s.id == here }).joinToString(" · ")
                            if (detail.isNotEmpty()) Text(detail, style = MaterialTheme.typography.bodySmall, color = cs.onSurfaceVariant)
                        }
                    }
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
