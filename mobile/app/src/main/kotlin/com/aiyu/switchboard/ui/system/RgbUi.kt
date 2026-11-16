package com.aiyu.switchboard.ui.system

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.border
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AutoAwesome
import androidx.compose.material.icons.rounded.Lightbulb
import androidx.compose.material.icons.rounded.Palette
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.WbSunny
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.aiyu.switchboard.net.Cmd
import com.aiyu.switchboard.net.HostConnection
import com.aiyu.switchboard.net.RgbState
import com.aiyu.switchboard.ui.kit.LevelRow
import com.aiyu.switchboard.ui.kit.MonoBadge
import com.aiyu.switchboard.ui.kit.PodHeader
import com.aiyu.switchboard.ui.kit.SectionCard

/** Effects the computer can draw, in the order they are listed. */
private val effects = listOf(
    Triple("static", "Solid color", "One steady color"),
    Triple("breathing", "Breathing", "The color fading in and out"),
    Triple("rainbow", "Rainbow", "The whole color wheel, in a loop"),
    Triple("rainbow_wave", "Rainbow wave", "The color wheel running along each strip"),
    Triple("neon_shift", "Neon Shift", "A loop of pink, purple and blue"),
)

private val swatches = listOf(
    "#ffffff" to "White",
    "#ff0000" to "Red",
    "#ff6a00" to "Orange",
    "#00ff40" to "Green",
    "#00d0ff" to "Cyan",
    "#0040ff" to "Blue",
    "#8000ff" to "Purple",
    "#ff0080" to "Pink",
)

fun rgbEffectName(effect: String) = effects.find { it.first == effect }?.second ?: effect

/** `#rrggbb` as a color; white when the computer sent something else. */
internal fun rgbColor(hex: String): Color = hex.removePrefix("#").takeIf { it.length == 6 }?.toLongOrNull(16)?.let { Color(0xFF000000 or it) } ?: Color.White

/** The dashboard's lighting pod: on/off on the icon, brightness on the slider. */
@Composable
fun RgbPod(rgb: RgbState, conn: HostConnection, onOpen: () -> Unit) {
    val cs = MaterialTheme.colorScheme
    SectionCard {
        PodHeader(Icons.Rounded.Lightbulb, "RGB lighting", if (rgb.on) rgbEffectName(rgb.effect) else "Off", shape = MaterialShapes.Sunny, action = "Effects", onAction = onOpen)
        Spacer(Modifier.height(14.dp))
        LevelRow(
            value = rgb.brightness,
            label = "RGB brightness",
            onSend = { conn.post(Cmd.setRgbBrightness(it)) },
            icon = Icons.Rounded.Lightbulb,
            onIcon = { conn.post(Cmd.setRgbPower(!rgb.on)) },
            iconContainer = if (rgb.on) cs.primaryContainer else cs.surfaceContainerHighest,
            iconTint = if (rgb.on) cs.onPrimaryContainer else cs.onSurfaceVariant,
            badge = if (rgb.on) null else "OFF",
            enabled = rgb.on,
        )
    }
}

/** The lighting screen: power and brightness, the effect, then what that effect can change. */
@Composable
fun RgbContent(rgb: RgbState, conn: HostConnection) {
    val cs = MaterialTheme.colorScheme
    SectionCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("Lighting", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                Text(if (rgb.on) "On · ${rgbEffectName(rgb.effect)}" else "Off", style = MaterialTheme.typography.bodySmall, color = cs.onSurfaceVariant)
            }
            Switch(checked = rgb.on, onCheckedChange = { conn.post(Cmd.setRgbPower(it)) }, modifier = Modifier.semantics { contentDescription = "RGB lighting" })
        }
        Spacer(Modifier.height(14.dp))
        LevelRow(rgb.brightness, "RGB brightness", { conn.post(Cmd.setRgbBrightness(it)) }, icon = Icons.Rounded.WbSunny, enabled = rgb.on)
        Spacer(Modifier.height(12.dp))
        Presets(rgb.brightness) { conn.post(Cmd.setRgbBrightness(it)) }
    }

    SectionCard {
        PodHeader(Icons.Rounded.AutoAwesome, "Effect", "Every light shows the same one", shape = MaterialShapes.Clover4Leaf)
        Spacer(Modifier.height(12.dp))
        Column(Modifier.selectableGroup(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            effects.forEach { (id, name, about) ->
                val picked = rgb.effect == id
                Row(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).selectable(selected = picked, role = Role.RadioButton) { if (!picked) conn.post(Cmd.setRgbEffect(id)) }.padding(vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RadioButton(selected = picked, onClick = null)
                    Spacer(Modifier.width(12.dp))
                    Column {
                        Text(name, style = MaterialTheme.typography.bodyLarge)
                        Text(about, style = MaterialTheme.typography.bodySmall, color = cs.onSurfaceVariant)
                    }
                }
            }
        }
    }

    // Rainbow and Neon Shift bring their own colors.
    if (rgb.effect == "static" || rgb.effect == "breathing") SectionCard {
        PodHeader(Icons.Rounded.Palette, "Color", shape = MaterialShapes.Cookie6Sided, trailing = { MonoBadge(rgb.color.uppercase(), color = cs.surfaceContainerHighest, content = cs.onSurfaceVariant) })
        Spacer(Modifier.height(14.dp))
        Row(Modifier.fillMaxWidth().selectableGroup(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            swatches.forEach { (hex, name) ->
                val picked = rgb.color.equals(hex, ignoreCase = true)
                Box(
                    Modifier
                        .weight(1f)
                        .aspectRatio(1f)
                        .clip(CircleShape)
                        .background(rgbColor(hex))
                        .border(BorderStroke(if (picked) 3.dp else 1.dp, if (picked) cs.primary else cs.outlineVariant), CircleShape)
                        .selectable(selected = picked, role = Role.RadioButton) { conn.post(Cmd.setRgbColor(hex)) }
                        .semantics { contentDescription = name },
                )
            }
        }
    }

    // A steady color has nothing to speed up.
    if (rgb.effect != "static") SectionCard {
        PodHeader(Icons.Rounded.Speed, "Speed", "How fast the effect runs", shape = MaterialShapes.Flower)
        Spacer(Modifier.height(12.dp))
        LevelRow(rgb.speed, "RGB effect speed", { conn.post(Cmd.setRgbSpeed(it)) }, icon = Icons.Rounded.Speed)
    }
}
