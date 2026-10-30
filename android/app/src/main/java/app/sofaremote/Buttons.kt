package app.sofaremote

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import org.json.JSONObject

/** What the phone's buttons do. Stored as JSON, edited from the web app's settings. */
data class ButtonConfig(
    val enabled: Boolean = true,
    val locked: Boolean = true,
    val up: String = "volup",
    val down: String = "voldown",
    val both: String = "playpause",
    val window: Int = 150,
    val play: String = "playpause",
    val next: String = "next",
    val prev: String = "prev",
    val silent: Boolean = false,
) {
    fun toJson(): JSONObject = JSONObject()
        .put("enabled", enabled).put("locked", locked).put("up", up).put("down", down).put("both", both)
        .put("window", window).put("play", play).put("next", next).put("prev", prev).put("silent", silent)

    companion object {
        fun parse(text: String?): ButtonConfig {
            val d = ButtonConfig()
            val j = runCatching { JSONObject(text ?: "{}") }.getOrElse { JSONObject() }
            return ButtonConfig(
                enabled = j.optBoolean("enabled", d.enabled),
                locked = j.optBoolean("locked", d.locked),
                up = j.optString("up", d.up),
                down = j.optString("down", d.down),
                both = j.optString("both", d.both),
                window = j.optInt("window", d.window).coerceIn(0, 600),
                play = j.optString("play", d.play),
                next = j.optString("next", d.next),
                prev = j.optString("prev", d.prev),
                silent = j.optBoolean("silent", d.silent),
            )
        }
    }
}

object Prefs {
    private fun sp(c: Context) = c.getSharedPreferences("sofa", Context.MODE_PRIVATE)
    fun link(c: Context): String? = sp(c).getString("link", null)
    fun setLink(c: Context, link: String) {
        val edit = sp(c).edit().putString("link", link)
        if (link != link(c)) edit.remove("wake") // another PC: its addresses are not the saved ones
        edit.apply()
    }
    fun clearLink(c: Context) = sp(c).edit().remove("link").remove("wake").apply()
    /** Wake-on-LAN addresses of the PC (JSON, see Wake), saved while it was awake. */
    fun wake(c: Context): String? = sp(c).getString("wake", null)
    fun setWake(c: Context, json: String) = sp(c).edit().putString("wake", json).apply()
    fun buttons(c: Context) = ButtonConfig.parse(sp(c).getString("buttons", null))
    fun setButtons(c: Context, cfg: ButtonConfig) = sp(c).edit().putString("buttons", cfg.toJson().toString()).apply()
}

/** Volume up / down presses, with "both at once" detection. */
object Buttons {
    const val UP = 0
    const val DOWN = 1

    var cfg = ButtonConfig()
    private val main = Handler(Looper.getMainLooper())
    private var pending: Runnable? = null
    private var pendingButton = -1
    private var pendingAt = 0L
    private var quietUntil = 0L

    fun fire(action: String) {
        Remote.send(Remote.message(action))
    }

    /** A press of UP or DOWN. `repeat`: the button is being held (no "both" check). */
    fun press(button: Int, repeat: Boolean = false) {
        val now = SystemClock.uptimeMillis()
        if (now < quietUntil) return
        val single = if (button == UP) cfg.up else cfg.down
        if (repeat || cfg.both.isBlank() || cfg.window <= 0) return fire(single)

        // With volup/voldown on the two buttons, each press goes out at once: if the other one
        // follows within the window, its own step cancels the first and "both" fires as well.
        // Other actions wait for the window, since they can't be undone.
        val instant = setOf(cfg.up.trim().lowercase(), cfg.down.trim().lowercase()) == setOf("volup", "voldown")
        if (pendingButton == 1 - button && now - pendingAt <= cfg.window) {
            pending?.let { main.removeCallbacks(it) }
            pending = null
            pendingButton = -1
            if (instant) fire(single)
            fire(cfg.both)
            quietUntil = now + 350   // ignore the rest of a both-buttons press
            return
        }
        pending?.let { main.removeCallbacks(it); it.run() }
        if (instant) fire(single)
        val run = Runnable {
            pending = null
            pendingButton = -1
            if (!instant) fire(single)
        }
        pending = run
        pendingButton = button
        pendingAt = now
        main.postDelayed(run, cfg.window.toLong())
    }
}
