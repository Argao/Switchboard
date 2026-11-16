package com.aiyu.switchboard.ui.system

import androidx.compose.foundation.clickable
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.aiyu.switchboard.net.AppWindow
import com.aiyu.switchboard.net.Cmd
import com.aiyu.switchboard.net.HostConnection
import com.aiyu.switchboard.net.HostState
import com.aiyu.switchboard.net.Screen
import com.aiyu.switchboard.ui.kit.MonoBadge
import com.aiyu.switchboard.ui.kit.MonoCaption
import com.aiyu.switchboard.ui.kit.PodHeader
import com.aiyu.switchboard.ui.kit.SectionCard
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

/** Home card: how much is open, and the way in. */
@Composable
fun AppsPod(open: List<AppWindow>, onOpen: () -> Unit) {
    SectionCard {
        PodHeader(Icons.Rounded.Apps, "Apps", appsSummary(open), shape = MaterialShapes.Pill, action = "Manage", onAction = onOpen)
    }
}

/** The Apps screen: every open window, with what can be done to it. */
@Composable
fun AppsContent(s: HostState, conn: HostConnection) {
    val cs = MaterialTheme.colorScheme
    val windows = s.windows
    if (windows == null) {
        SectionCard { Text("This computer cannot list its open apps yet. For now that works on Windows.", color = cs.onSurfaceVariant) }
        return
    }
    val scope = rememberCoroutineScope()
    // The computer may refuse (a program running as administrator); say why.
    var note by remember { mutableStateOf<String?>(null) }
    val run: (kotlinx.serialization.json.JsonObject) -> Unit = { cmd ->
        scope.launch {
            val r = conn.send(cmd)
            note = if (r.ok) null else r.error ?: "That did not work"
        }
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
    if (pick) ScreenPicker(w, screens, onDismiss = { pick = false }) { run(Cmd.setWindowFullscreen(w.id, it.id, true)) }
    if (askEnd) Confirm("End ${w.app}?", "It stops at once, with every window it has open. Anything not saved is lost.", { askEnd = false }) { run(Cmd.killApp(w.id)) }
}

@Composable
private fun ScreenPicker(w: AppWindow, screens: List<Screen>, onDismiss: () -> Unit, onPick: (Screen) -> Unit) {
    val cs = MaterialTheme.colorScheme
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Full screen on") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(w.app, style = MaterialTheme.typography.bodyMedium, color = cs.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Spacer(Modifier.height(4.dp))
                screens.forEach { s ->
                    Row(
                        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).clickable { onDismiss(); onPick(s) }.padding(vertical = 10.dp, horizontal = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(Icons.Rounded.Monitor, null, tint = if (s.id == w.screen) cs.primary else cs.onSurfaceVariant)
                        Spacer(Modifier.width(14.dp))
                        Column {
                            Text(s.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            val detail = listOfNotNull(screenDetail(s).takeIf { it.isNotEmpty() }, "It is here now".takeIf { s.id == w.screen }).joinToString(" · ")
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
