package app.sofaremote

import android.net.Uri
import android.os.Handler
import android.os.Looper
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject
import java.util.concurrent.TimeUnit
import kotlin.math.roundToInt

/** The app's own connection to the PC, used by the hardware buttons. It keeps working while
 *  the screen is locked, when the web page inside the app is paused. */
object Remote {
    private val client = OkHttpClient.Builder().pingInterval(15, TimeUnit.SECONDS).build()
    private val main = Handler(Looper.getMainLooper())
    private var ws: WebSocket? = null
    private var url: String? = null
    private var wanted = false
    private var retry = 500L

    @Volatile var online = false
        private set
    @Volatile var pcName = ""
        private set
    var onState: (() -> Unit)? = null

    /** `link` is the one the PC prints: http://host:port/?k=KEY */
    fun connect(link: String) {
        val u = socketUrl(link) ?: return
        if (wanted && u == url) return
        disconnect()
        url = u
        wanted = true
        open()
    }

    fun disconnect() {
        wanted = false
        main.removeCallbacksAndMessages(null)
        ws?.close(1000, null)
        ws = null
        setOnline(false)
    }

    fun send(msg: JSONObject?): Boolean {
        msg ?: return false
        val sent = ws?.send(msg.toString()) == true
        if (!sent && wanted && ws == null) open()
        return sent
    }

    private fun open() {
        val u = url ?: return
        ws = client.newWebSocket(Request.Builder().url(u).build(), object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                if (webSocket != ws) return
                retry = 500
                setOnline(true)
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                val m = runCatching { JSONObject(text) }.getOrNull() ?: return
                if (m.optString("t") == "hello") {
                    pcName = m.optString("name")
                    main.post { onState?.invoke() }
                }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) = lost(webSocket)
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) = lost(webSocket)
        })
    }

    private fun lost(webSocket: WebSocket) {
        main.post {
            if (webSocket != ws) return@post
            ws = null
            setOnline(false)
            if (wanted) main.postDelayed({ if (wanted && ws == null) open() }, retry)
            retry = (retry * 2).coerceAtMost(5000)
        }
    }

    private fun setOnline(on: Boolean) {
        if (online == on) return
        online = on
        main.post { onState?.invoke() }
    }

    fun socketUrl(link: String): String? {
        val uri = runCatching { Uri.parse(link.trim()) }.getOrNull() ?: return null
        val key = uri.getQueryParameter("k") ?: return null
        val scheme = when (uri.scheme) { "http" -> "ws"; "https" -> "wss"; else -> return null }
        val host = uri.host ?: return null
        val port = if (uri.port > 0) ":${uri.port}" else ""
        return "$scheme://$host$port/ws?k=${Uri.encode(key)}"
    }

    /** One action, as typed in the app's settings: a key ("volup"), a shortcut ("ctrl+right"),
     *  click / rclick / dblclick / mclick, scroll:N, text:..., open:... */
    fun message(action: String): JSONObject? {
        val a = action.trim()
        if (a.isEmpty()) return null
        val lower = a.lowercase()
        val m = JSONObject()
        when {
            lower in setOf("click", "rclick", "dblclick", "mclick") -> m.put("t", "mouse").put("kind", lower)
            lower.startsWith("scroll:") -> {
                val notches = lower.removePrefix("scroll:").trim().toDoubleOrNull() ?: return null
                m.put("t", "mouse").put("kind", "scroll").put("dy", -(notches * 120).roundToInt())
            }
            lower.startsWith("text:") -> m.put("t", "type").put("text", a.substring(5))
            lower.startsWith("open:") -> m.put("t", "open").put("url", a.substring(5).trim())
            a.length > 1 && '+' in a -> m.put("t", "combo").put("keys", org.json.JSONArray(lower.split('+').map { it.trim() }.filter { it.isNotEmpty() }))
            else -> m.put("t", "key").put("k", lower)
        }
        return m
    }
}
