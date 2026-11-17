package com.aiyu.switchboard.net

import android.Manifest
import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.widget.RemoteViews
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import com.aiyu.switchboard.App
import com.aiyu.switchboard.MainActivity
import com.aiyu.switchboard.R
import com.aiyu.switchboard.ui.system.clockText
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.time.ZoneId

/** How long before the shutdown the phone speaks up, in seconds. */
internal const val WARN_BEFORE = 300L

/** How much later "Postpone" makes it, in seconds. */
internal const val POSTPONE = 1800L

/** Furthest ahead the computer takes a shutdown: a day. */
private const val MAX_AHEAD = 86_400L

/** When to warn about a shutdown set for [at]; null when that moment has passed (it was set, or seen, with less time left). */
internal fun warnTime(at: Long, now: Long): Long? = (at - WARN_BEFORE).takeIf { it > now }

/**
 * The delay to ask the computer for so that a shutdown set for [at] comes
 * [POSTPONE] later. The computer counts from now, so the time left goes in too.
 */
internal fun postponeDelay(at: Long, now: Long): Int = ((at - now).coerceAtLeast(0) + POSTPONE).coerceAtMost(MAX_AHEAD).toInt()

internal fun timerTitle(hostName: String, at: Long, zone: ZoneId): String = "$hostName shuts down at ${clockText(at, zone)}"

internal fun warningTitle(hostName: String): String = "$hostName is about to shut down"

internal fun timerBody(at: Long, cutAfter: Int, zone: ZoneId): String = "Total shutdown. The plug cuts the power at ${clockText(at + cutAfter, zone)}."

/**
 * Keeps a scheduled total shutdown in the notification shade, counting down,
 * whether or not the app is open, and speaks up [WARN_BEFORE] seconds before it
 * with a way to postpone.
 *
 * The computer gives the time of the shutdown, not a countdown, so none of
 * this needs the app running: Android draws the countdown from that time, and
 * an alarm brings the warning. Only acting on it (postpone, cancel) reconnects.
 */
object ShutdownTimer {
    private const val TIMER = "shutdown_timer"
    private const val WARNING = "shutdown_warning"
    const val WARN = "com.aiyu.switchboard.SHUTDOWN_WARN"
    const val POSTPONE_IT = "com.aiyu.switchboard.SHUTDOWN_POSTPONE"
    const val CANCEL_IT = "com.aiyu.switchboard.SHUTDOWN_CANCEL"
    private const val HOST = "host"
    private const val NAME = "name"
    private const val AT = "at"
    private const val CUT = "cut_after"

    private fun timerId(hostId: String) = "shutdown:$hostId".hashCode()
    private fun warningId(hostId: String) = "shutdown-warning:$hostId".hashCode()

    private fun intent(context: Context, action: String, hostId: String, hostName: String, at: Long, cutAfter: Int) =
        Intent(context, ShutdownReceiver::class.java).setAction(action).putExtra(HOST, hostId).putExtra(NAME, hostName).putExtra(AT, at).putExtra(CUT, cutAfter)

    private fun broadcast(context: Context, intent: Intent): PendingIntent =
        // One per computer and action; the extras of a newer one replace the older's.
        PendingIntent.getBroadcast(context, (intent.getStringExtra(HOST) + intent.action).hashCode(), intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)

    /**
     * Makes the shade and the alarm match what the computer [hostId] says:
     * [t] with a time set shows the countdown, anything else takes it away.
     * Called on every change while connected, and again on each reconnect.
     */
    @Synchronized
    fun sync(context: Context, hostId: String, hostName: String, t: TotalShutdown?, now: Long = System.currentTimeMillis() / 1000) {
        val at = t?.at
        if (at == null || at <= now) return clear(context, hostId)
        if (Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val nm = context.getSystemService(NotificationManager::class.java)
        val am = context.getSystemService(AlarmManager::class.java)
        val alarm = broadcast(context, intent(context, WARN, hostId, hostName, at, t.cutAfter))
        val warn = warnTime(at, now)
        if (warn == null) {
            am.cancel(alarm)
            // Already warned about this very one: leave the warning where it is.
            if (nm.activeNotifications.any { it.id == warningId(hostId) && it.notification.extras.getLong(AT) == at }) return
        } else if (Build.VERSION.SDK_INT < 31 || am.canScheduleExactAlarms()) {
            am.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, warn * 1000, alarm)
        } else {
            // Exact alarms were turned off for the app: the warning may come a little late.
            am.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, warn * 1000, alarm)
        }
        nm.cancel(warningId(hostId))
        nm.notify(timerId(hostId), build(context, hostId, hostName, at, t.cutAfter, now, warning = false))
    }

    /** Nothing is scheduled on [hostId] any more. */
    fun clear(context: Context, hostId: String) {
        val nm = context.getSystemService(NotificationManager::class.java)
        nm.cancel(timerId(hostId))
        nm.cancel(warningId(hostId))
        context.getSystemService(AlarmManager::class.java).cancel(broadcast(context, intent(context, WARN, hostId, "", 0, 0)))
    }

    private fun build(context: Context, hostId: String, hostName: String, at: Long, cutAfter: Int, now: Long, warning: Boolean): android.app.Notification {
        val nm = context.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(TIMER, "Shutdown timer", NotificationManager.IMPORTANCE_LOW))
        nm.createNotificationChannel(NotificationChannel(WARNING, "Shutdown warning", NotificationManager.IMPORTANCE_HIGH))
        val zone = ZoneId.systemDefault()
        val flags = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        fun action(what: String) = broadcast(context, intent(context, what, hostId, hostName, at, cutAfter))
        val title = if (warning) warningTitle(hostName) else timerTitle(hostName, at, zone)
        val body = timerBody(at, cutAfter, zone)
        // The time left is what this is for, so it is drawn large, ahead of the words. A
        // chronometer runs on the time since boot, and Android keeps it counting with the app gone.
        val zero = SystemClock.elapsedRealtime() + (at * 1000 - System.currentTimeMillis())
        fun views(layout: Int, withBody: Boolean) = RemoteViews(context.packageName, layout).apply {
            setChronometerCountDown(R.id.shutdown_chronometer, true)
            setChronometer(R.id.shutdown_chronometer, zero, null, true)
            setTextViewText(R.id.shutdown_title, title)
            if (withBody) setTextViewText(R.id.shutdown_text, body)
        }
        return NotificationCompat.Builder(context, if (warning) WARNING else TIMER)
            .setSmallIcon(android.R.drawable.ic_lock_power_off)
            // For what does not draw the layouts below: a watch, a screen reader.
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(NotificationCompat.DecoratedCustomViewStyle())
            .setCustomContentView(views(R.layout.notification_shutdown_timer, withBody = false))
            .setCustomBigContentView(views(R.layout.notification_shutdown_timer_big, withBody = true))
            .setCustomHeadsUpContentView(views(R.layout.notification_shutdown_timer, withBody = false))
            .setWhen(at * 1000)
            // The large one says it; a second, small one in the header would only repeat it.
            .setShowWhen(false)
            .setOngoing(true)
            .setOnlyAlertOnce(!warning)
            .setPriority(if (warning) NotificationCompat.PRIORITY_HIGH else NotificationCompat.PRIORITY_LOW)
            .setCategory(if (warning) NotificationCompat.CATEGORY_ALARM else NotificationCompat.CATEGORY_STOPWATCH)
            // The computer is off by then; nothing is left to tell this to go.
            .setTimeoutAfter(((at - now).coerceAtLeast(1)) * 1000)
            .addExtras(Bundle().apply { putLong(AT, at) })
            .setContentIntent(PendingIntent.getActivity(context, 0, Intent(context, MainActivity::class.java), flags))
            .addAction(0, "Postpone 30 min", action(POSTPONE_IT))
            .addAction(0, "Cancel shutdown", action(CANCEL_IT))
            .build()
    }

    /** What a tap on the notification, or the alarm, asked for. Returns what to tell the user, if anything. */
    internal suspend fun handle(context: Context, intent: Intent): String? {
        val hostId = intent.getStringExtra(HOST) ?: return null
        val name = intent.getStringExtra(NAME).orEmpty()
        val at = intent.getLongExtra(AT, 0)
        val connections = (context.applicationContext as App).connections
        connections.start()
        val conn = withTimeoutOrNull(4_000) {
            val c = connections.live.first { hostId in it }.getValue(hostId)
            val fresh = c.link.value is Link.Online
            c.link.first { it is Link.Online }
            // What it held from before the app went away may be old; the computer sends its state right behind the welcome.
            if (!fresh) withTimeoutOrNull(2_000) { c.state.drop(1).first() }
            c
        }
        // Connecting brought the shade up to date by itself (Connections follows every computer's schedule).
        val current = conn?.state?.value?.totalShutdown?.at
        val now = System.currentTimeMillis() / 1000
        return when (intent.action) {
            WARN -> {
                // Moved or called off from somewhere else while the phone was away: no longer this one's warning.
                if (conn != null && current != at) return null
                if (at <= now) return null
                // Not in between the steps of a sync, which would leave both up.
                synchronized(this) {
                    val nm = context.getSystemService(NotificationManager::class.java)
                    nm.cancel(timerId(hostId))
                    nm.notify(warningId(hostId), build(context, hostId, name, at, intent.getIntExtra(CUT, 300), now, warning = true))
                }
                null
            }
            POSTPONE_IT -> {
                if (conn == null) return "Couldn't reach $name. The shutdown was not postponed."
                if (current == null) return "$name has no shutdown scheduled any more."
                val r = conn.send(Cmd.totalShutdown(postponeDelay(current, now)))
                if (!r.ok) return r.error ?: "The shutdown was not postponed."
                // The new time comes back with the next state, and the shade follows it.
                val moved = withTimeoutOrNull(2_000) { conn.state.first { it?.totalShutdown?.at != current } }?.totalShutdown
                    ?: conn.state.value?.totalShutdown?.copy(at = current + POSTPONE)?.also { sync(context, hostId, name, it) }
                "$name shuts down at ${clockText(moved?.at ?: (current + POSTPONE), ZoneId.systemDefault())}."
            }
            CANCEL_IT -> {
                if (conn == null) return "Couldn't reach $name. The shutdown is still on."
                val r = conn.send(Cmd.cancelTotalShutdown())
                if (!r.ok) return r.error ?: "The shutdown is still on."
                clear(context, hostId)
                "Shutdown of $name cancelled."
            }
            else -> null
        }
    }
}

class ShutdownReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val pending = goAsync()
        CoroutineScope(Dispatchers.Main).launch {
            val foreground = ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
            val connections = (context.applicationContext as App).connections
            try {
                ShutdownTimer.handle(context, intent)?.let { Toast.makeText(context, it, Toast.LENGTH_LONG).show() }
            } finally {
                pending.finish()
                // Let the connection go again unless the app is open or files are still moving.
                if (!foreground) {
                    delay(1_500)
                    if (!ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED) && !connections.transfers.busy.value) connections.stop()
                }
            }
        }
    }
}
