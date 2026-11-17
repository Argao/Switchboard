package com.aiyu.switchboard.net

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Mirrors core/src/protocol.rs. Keep the two in step.

const val PROTOCOL_VERSION = 2

/** Bytes per file chunk before base64; core/src/protocol.rs FILE_CHUNK. */
const val FILE_CHUNK = 192 * 1024

val WireJson = Json {
    ignoreUnknownKeys = true
    classDiscriminator = "type"
    explicitNulls = false
    encodeDefaults = true
}

@Serializable
data class ClientHello(
    val v: Int = PROTOCOL_VERSION,
    val mode: String,
    @SerialName("device_id") val deviceId: String,
    @SerialName("device_name") val deviceName: String,
    @SerialName("static_pk") val staticPk: String,
    @SerialName("eph_pk") val ephPk: String,
)

@Serializable
sealed interface ServerHello {
    @Serializable @SerialName("accept")
    data class Accept(val v: Int, @SerialName("host_id") val hostId: String, @SerialName("host_name") val hostName: String, @SerialName("eph_pk") val ephPk: String) : ServerHello

    @Serializable @SerialName("reject")
    data class Reject(val reason: String) : ServerHello
}

@Serializable
sealed interface Event {
    @Serializable @SerialName("welcome")
    data class Welcome(@SerialName("host_id") val hostId: String, @SerialName("host_name") val hostName: String, val os: String, val macs: List<String> = emptyList()) : Event

    @Serializable @SerialName("result")
    data class Result(val id: Long, val ok: Boolean, val error: String? = null, val data: String? = null) : Event

    /** The desktop wants to send this phone a file. */
    @Serializable @SerialName("file_offer")
    data class FileOffer(val xfer: String, val name: String, val size: Long) : Event

    @Serializable @SerialName("state")
    data class State(
        val volume: Volume? = null,
        val outputs: List<AudioOutput> = emptyList(),
        val apps: List<AudioApp> = emptyList(),
        val displays: List<Display> = emptyList(),
        /** Null when the computer has no RGB lighting it can drive. */
        val rgb: RgbState? = null,
        val media: Media? = null,
        val players: List<MediaPlayer> = emptyList(),
        val toggles: List<ToggleState> = emptyList(),
        /** True when the person at the computer has allowed phones to use its keyboard and mouse. */
        @SerialName("remote_input") val remoteInput: Boolean = false,
        /** The open windows, front to back. Null when the computer cannot manage them. */
        val windows: List<AppWindow>? = null,
        /** The screens a window can be sent to. */
        val screens: List<Screen> = emptyList(),
        /** The streaming sites the computer can open in full screen, by id ("netflix", "prime_video"); empty when it cannot. */
        val streams: List<String> = emptyList(),
        /** Null when the computer has no smart plug set up. */
        @SerialName("total_shutdown") val totalShutdown: TotalShutdown? = null,
        val stats: Stats = Stats(),
    ) : Event

    @Serializable @SerialName("revoked")
    data object Revoked : Event
}

typealias HostState = Event.State

@Serializable data class Volume(val level: Int, val muted: Boolean)

@Serializable data class AudioOutput(val id: String, val name: String, val default: Boolean)

@Serializable data class AudioApp(val id: String, val name: String, val detail: String = "", val level: Int, val muted: Boolean)

@Serializable
data class Display(val id: String, val name: String, val kind: String, val brightness: Int? = null, val contrast: Int? = null, val power: Boolean? = null) {
    val external get() = kind == "external"
}

/** [effect] is "static", "breathing", "rainbow", "rainbow_wave" or "neon_shift"; [color] is `#rrggbb`. */
@Serializable data class RgbState(val on: Boolean, val brightness: Int, val effect: String, val color: String, val speed: Int)

/** One open window; [screen] is the [Screen.id] it is on. */
@Serializable data class AppWindow(val id: String, val app: String, val title: String, val screen: String? = null, val fullscreen: Boolean = false)

/** A screen as the computer lays windows out on it; not the same list as [Display]. */
@Serializable data class Screen(val id: String, val name: String, val primary: Boolean = false, val width: Int = 0, val height: Int = 0)

/**
 * Shutting down and then cutting the power at the computer's smart plug.
 * [at] is when the shutdown starts and [cutAt] when the plug cuts the power,
 * both in Unix seconds; [cutAfter] is the plug's wait in seconds.
 */
@Serializable
data class TotalShutdown(
    val at: Long? = null,
    @SerialName("cut_after") val cutAfter: Int = 300,
    @SerialName("cut_at") val cutAt: Long? = null,
    /** An update is waiting to install; a total shutdown is refused until it is done. */
    @SerialName("pending_update") val pendingUpdate: Boolean = false,
    /** Why the last one did not happen. */
    val error: String? = null,
)

/** `dark_mode`, `do_not_disturb`, `wifi` or `bluetooth`; only the ones the computer supports are sent. */
@Serializable data class ToggleState(val toggle: String, val on: Boolean)

@Serializable data class MediaPlayer(val id: String, val name: String, val playing: Boolean)

@Serializable
data class Media(
    val id: String = "",
    val player: String,
    val title: String,
    val artist: String,
    val playing: Boolean,
    /** Milliseconds into the track and its length, when the player says. */
    val position: Long? = null,
    val length: Long? = null,
    @SerialName("can_seek") val canSeek: Boolean = false,
    val shuffle: Boolean? = null,
    /** "off", "track" or "all". */
    val repeat: String? = null,
    /** Changes with the cover; fetch the picture with [Cmd.mediaArt]. */
    val art: String? = null,
)

@Serializable
data class Stats(
    val cpu: Float = 0f,
    @SerialName("mem_used") val memUsed: Long = 0,
    @SerialName("mem_total") val memTotal: Long = 0,
    val uptime: Long = 0,
    val battery: Battery? = null,
)

@Serializable data class Battery(val percent: Int, val charging: Boolean)

/** Phone-to-desktop commands, built as the flat objects serde expects. */
object Cmd {
    fun hello() = obj("hello")
    fun setVolume(level: Int) = obj("set_volume") { put("level", level.coerceIn(0, 100)) }
    fun setMute(muted: Boolean) = obj("set_mute") { put("muted", muted) }
    fun setOutput(output: String) = obj("set_output") { put("output", output) }
    fun setAppVolume(app: String, level: Int) = obj("set_app_volume") { put("app", app); put("level", level.coerceIn(0, 100)) }
    fun setAppMute(app: String, muted: Boolean) = obj("set_app_mute") { put("app", app); put("muted", muted) }
    fun setBrightness(display: String, level: Int) = obj("set_brightness") { put("display", display); put("level", level.coerceIn(0, 100)) }
    fun setContrast(display: String, level: Int) = obj("set_contrast") { put("display", display); put("level", level.coerceIn(0, 100)) }
    fun setDisplayPower(display: String, on: Boolean) = obj("set_display_power") { put("display", display); put("on", on) }
    fun setRgbPower(on: Boolean) = obj("set_rgb_power") { put("on", on) }
    fun setRgbBrightness(level: Int) = obj("set_rgb_brightness") { put("level", level.coerceIn(0, 100)) }
    fun setRgbEffect(effect: String) = obj("set_rgb_effect") { put("effect", effect) }
    fun setRgbColor(color: String) = obj("set_rgb_color") { put("color", color) }
    fun setRgbSpeed(level: Int) = obj("set_rgb_speed") { put("level", level.coerceIn(0, 100)) }
    fun media(action: String) = obj("media") { put("action", action) }
    fun seek(positionMs: Long) = obj("seek") { put("position", positionMs.coerceAtLeast(0)) }
    fun setShuffle(on: Boolean) = obj("set_shuffle") { put("on", on) }
    fun setRepeat(mode: String) = obj("set_repeat") { put("mode", mode) }
    fun selectPlayer(id: String) = obj("select_player") { put("player", id) }
    fun mediaArt() = obj("media_art")
    fun mouseMove(dx: Int, dy: Int) = obj("mouse_move") { put("dx", dx); put("dy", dy) }
    /** [button] is "left", "right" or "middle"; [action] is "click", "press" or "release". */
    fun mouseButton(button: String, action: String) = obj("mouse_button") { put("button", button); put("action", action) }
    fun scroll(dx: Int, dy: Int) = obj("scroll") { put("dx", dx); put("dy", dy) }
    fun key(key: String, ctrl: Boolean = false, alt: Boolean = false, shift: Boolean = false, meta: Boolean = false) =
        obj("key") { put("key", key); put("ctrl", ctrl); put("alt", alt); put("shift", shift); put("meta", meta) }
    fun chord(ch: Char, ctrl: Boolean = false, alt: Boolean = false, shift: Boolean = false, meta: Boolean = false) =
        obj("chord") { put("ch", ch.toString()); put("ctrl", ctrl); put("alt", alt); put("shift", shift); put("meta", meta) }
    fun typeText(text: String) = obj("type_text") { put("text", text) }
    fun setToggle(toggle: String, on: Boolean) = obj("set_toggle") { put("toggle", toggle); put("on", on) }
    fun clipboardGet() = obj("clipboard_get")
    fun clipboardSet(text: String) = obj("clipboard_set") { put("text", text) }
    fun power(action: String) = obj("power") { put("action", action) }
    /** Shuts down [delay] seconds from now (0 is at once); the plug cuts the power [cutAfter] seconds later, or after its last wait when null. */
    fun totalShutdown(delay: Int, cutAfter: Int? = null) = obj("total_shutdown") { put("delay", delay.coerceIn(0, 86_400)); if (cutAfter != null) put("cut_after", cutAfter.coerceIn(60, 1800)) }
    fun cancelTotalShutdown() = obj("cancel_total_shutdown")
    /** Asks the window to close, as its own close button does. */
    fun closeWindow(window: String) = obj("close_window") { put("window", window) }
    /** Ends the window's program at once; unsaved work is lost. */
    fun killApp(window: String) = obj("kill_app") { put("window", window) }
    /** Covers [screen] with the window (the screen it is on when null), or puts it back. */
    fun setWindowFullscreen(window: String, screen: String?, on: Boolean) = obj("set_window_fullscreen") { put("window", window); if (screen != null) put("screen", screen); put("on", on) }
    /** Opens a streaming site (one of [Event.State.streams]) in the computer's browser, full screen on [screen], or wherever the browser opens when null. */
    fun openStream(stream: String, screen: String?) = obj("open_stream") { put("stream", stream); if (screen != null) put("screen", screen) }
    fun unpair() = obj("unpair")
    fun fileSend(xfer: String, name: String, size: Long) = obj("file_send") { put("xfer", xfer); put("name", name); put("size", size) }
    fun fileChunk(xfer: String, offset: Long, data: String) = obj("file_chunk") { put("xfer", xfer); put("offset", offset); put("data", data) }
    fun fileDone(xfer: String) = obj("file_done") { put("xfer", xfer) }
    fun fileCancel(xfer: String) = obj("file_cancel") { put("xfer", xfer) }
    fun fileRead(xfer: String, offset: Long, len: Int) = obj("file_read") { put("xfer", xfer); put("offset", offset); put("len", len) }

    private fun obj(type: String, body: kotlinx.serialization.json.JsonObjectBuilder.() -> Unit = {}): JsonObject =
        buildJsonObject { put("type", type); body() }
}
