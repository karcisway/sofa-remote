package app.sofaremote

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.support.v4.media.MediaMetadataCompat
import android.support.v4.media.session.MediaSessionCompat
import android.support.v4.media.session.PlaybackStateCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.media.VolumeProviderCompat
import androidx.media.session.MediaButtonReceiver
import kotlin.concurrent.thread

/**
 * Keeps the phone's buttons working with the screen locked.
 *
 * A media session set to "remote playback" gets the volume keys instead of the phone's own
 * volume (the same way a Chromecast app does), plus headset buttons and the play/pause/next/
 * previous buttons of its notification and lock screen. Runs as a foreground service, with a
 * wake lock and a Wi-Fi lock so the connection is still up when a button is pressed.
 */
class ButtonService : Service() {
    private lateinit var session: MediaSessionCompat
    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null
    @Volatile private var silence = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        Buttons.cfg = Prefs.buttons(this)
        session = MediaSessionCompat(this, "SofaRemote").apply {
            setCallback(object : MediaSessionCompat.Callback() {
                override fun onPlay() = Buttons.fire(Buttons.cfg.play)
                override fun onPause() = Buttons.fire(Buttons.cfg.play)
                override fun onSkipToNext() = Buttons.fire(Buttons.cfg.next)
                override fun onSkipToPrevious() = Buttons.fire(Buttons.cfg.prev)
            })
            setPlaybackToRemote(object : VolumeProviderCompat(VOLUME_CONTROL_RELATIVE, 100, 50) {
                override fun onAdjustVolume(direction: Int) {
                    if (direction > 0) Buttons.press(Buttons.UP) else if (direction < 0) Buttons.press(Buttons.DOWN)
                }
            })
            // Always "playing", so the system keeps sending the buttons here
            setPlaybackState(PlaybackStateCompat.Builder()
                .setActions(PlaybackStateCompat.ACTION_PLAY or PlaybackStateCompat.ACTION_PAUSE or
                    PlaybackStateCompat.ACTION_PLAY_PAUSE or PlaybackStateCompat.ACTION_SKIP_TO_NEXT or
                    PlaybackStateCompat.ACTION_SKIP_TO_PREVIOUS)
                .setState(PlaybackStateCompat.STATE_PLAYING, 0, 1f)
                .build())
            isActive = true
        }
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "Phone buttons", NotificationManager.IMPORTANCE_LOW).apply {
            setShowBadge(false)
        })
        ServiceCompat.startForeground(this, NOTIFICATION, notification(),
            if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK else 0)

        wakeLock = getSystemService(PowerManager::class.java)
            .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "SofaRemote:buttons").apply { acquire() }
        @Suppress("DEPRECATION")
        wifiLock = getSystemService(WifiManager::class.java)
            .createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "SofaRemote:buttons").apply { acquire() }

        Remote.onState = { refresh() }
        Prefs.link(this)?.let { Remote.connect(it) }
        if (Buttons.cfg.silent) startSilence()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        MediaButtonReceiver.handleIntent(session, intent)
        return START_STICKY
    }

    override fun onDestroy() {
        silence = false
        Remote.onState = null
        Remote.disconnect()
        session.release()
        wakeLock?.release()
        wifiLock?.release()
        super.onDestroy()
    }

    private fun refresh() {
        session.setMetadata(MediaMetadataCompat.Builder()
            .putString(MediaMetadataCompat.METADATA_KEY_TITLE, "Sofa Remote")
            .putString(MediaMetadataCompat.METADATA_KEY_ARTIST, Remote.pcName.ifEmpty { "PC" })
            .build())
        runCatching { getSystemService(NotificationManager::class.java).notify(NOTIFICATION, notification()) }
    }

    private fun notification(): Notification {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val button = { action: Long -> MediaButtonReceiver.buildMediaButtonPendingIntent(this, action) }
        val pc = Remote.pcName.ifEmpty { "the PC" }
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle("Sofa Remote")
            .setContentText(if (Remote.online) "Volume buttons control $pc" else "Connecting to $pc…")
            .setContentIntent(open)
            .setOngoing(true)
            .setSilent(true)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .addAction(R.drawable.ic_prev, "Previous", button(PlaybackStateCompat.ACTION_SKIP_TO_PREVIOUS))
            .addAction(R.drawable.ic_play_pause, "Play/Pause", button(PlaybackStateCompat.ACTION_PLAY_PAUSE))
            .addAction(R.drawable.ic_next, "Next", button(PlaybackStateCompat.ACTION_SKIP_TO_NEXT))
            .setStyle(androidx.media.app.NotificationCompat.MediaStyle()
                .setMediaSession(session.sessionToken)
                .setShowActionsInCompactView(0, 1, 2))
            .build()
    }

    /** Some phones only route the buttons to an app that is really playing sound: play silence. */
    private fun startSilence() {
        silence = true
        thread(name = "silence", isDaemon = true) {
            val rate = 8000
            val chunk = ShortArray(rate / 10)
            val track = AudioTrack.Builder()
                .setAudioAttributes(AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
                .setAudioFormat(AudioFormat.Builder()
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setSampleRate(rate)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build())
                .setBufferSizeInBytes(rate)
                .setTransferMode(AudioTrack.MODE_STREAM)
                .build()
            track.play()
            try {
                while (silence) track.write(chunk, 0, chunk.size)
            } finally {
                track.stop()
                track.release()
            }
        }
    }

    companion object {
        private const val CHANNEL = "buttons"
        private const val NOTIFICATION = 1

        /** Start or stop the service to match the settings and whether the app is on screen. */
        fun update(c: Context, appVisible: Boolean) {
            val cfg = Prefs.buttons(c)
            Buttons.cfg = cfg
            val run = cfg.enabled && Prefs.link(c) != null && (appVisible || cfg.locked)
            val intent = Intent(c, ButtonService::class.java)
            if (run) ContextCompat.startForegroundService(c, intent) else c.stopService(intent)
        }

        fun restart(c: Context, appVisible: Boolean) {
            c.stopService(Intent(c, ButtonService::class.java))
            update(c, appVisible)
        }
    }
}
