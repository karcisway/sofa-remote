package app.sofaremote

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.graphics.Color
import android.graphics.Typeface
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.text.InputType
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.inputmethod.InputMethodManager
import android.webkit.JavascriptInterface
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import com.journeyapps.barcodescanner.ScanIntentResult
import com.journeyapps.barcodescanner.ScanOptions
import com.google.zxing.client.android.Intents
import org.json.JSONObject

/** The web app from the PC in a WebView, plus what a web page can't do: the volume buttons
 *  (also with the screen locked, see ButtonService), opening the keyboard by itself, and a
 *  Wi-Fi lock that keeps the phone's Wi-Fi from dozing between touches (less mouse jitter). */
class MainActivity : Activity() {
    private var web: WebView? = null
    private var fullscreenView: View? = null
    private var fullscreenCallback: WebChromeClient.CustomViewCallback? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private val held = mutableSetOf<Int>()
    private val speech by lazy {
        Speech(this) { type, text ->
            val ev = JSONObject().put("type", type).put("text", text)
            web?.evaluateJavascript("window.sofaSpeech && window.sofaSpeech($ev)", null)
        }
    }
    private var exiting = false
    private var backPressedAt = 0L
    /** Language of a voice request waiting for the microphone permission. */
    private var pendingSpeech: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Buttons.cfg = Prefs.buttons(this)
        if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        ButtonService.onExit = { exitApp() }
        if (intent?.action == ACTION_EXIT) return exitApp()
        if (!handleShare(intent)) Prefs.link(this)?.let { load(it) } ?: showSetup(null)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        if (intent.action == ACTION_EXIT) exitApp() else handleShare(intent)
    }

    private fun handleShare(intent: Intent?): Boolean {
        if (intent?.action != Intent.ACTION_SEND) return false
        val text = intent.getStringExtra(Intent.EXTRA_TEXT) ?: return false
        val link = Regex("""https?://\S+""").find(text)?.value ?: text
        if (!useLink(link)) showSetup(link)
        return true
    }

    override fun onStart() {
        super.onStart()
        ButtonService.update(this, appVisible = true)
    }

    override fun onStop() {
        super.onStop()
        if (exiting) stopService(Intent(this, ButtonService::class.java))
        else ButtonService.update(this, appVisible = false)
    }

    override fun onDestroy() {
        ButtonService.onExit = null
        super.onDestroy()
    }

    override fun onResume() {
        super.onResume()
        web?.onResume()
        // Low-latency Wi-Fi while the app is on screen (Android 10+; the system drops it when hidden)
        @Suppress("DEPRECATION")
        val mode = if (Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_LOW_LATENCY else WifiManager.WIFI_MODE_FULL_HIGH_PERF
        wifiLock = getSystemService(WifiManager::class.java).createWifiLock(mode, "SofaRemote:app").apply { acquire() }
    }

    override fun onPause() {
        speech.destroy()
        wifiLock?.release()
        wifiLock = null
        super.onPause()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        if (requestCode != MIC_REQUEST) return
        val language = pendingSpeech ?: return
        pendingSpeech = null
        if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) speech.start(language)
        else web?.evaluateJavascript(
            "window.sofaSpeech && (sofaSpeech({type:'error',text:'Microphone permission denied'}), sofaSpeech({type:'end'}))", null)
    }

    // ---- volume buttons while the app is on screen
    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        val button = when (event.keyCode) {
            KeyEvent.KEYCODE_VOLUME_UP -> Buttons.UP
            KeyEvent.KEYCODE_VOLUME_DOWN -> Buttons.DOWN
            else -> return super.dispatchKeyEvent(event)
        }
        if (!Buttons.cfg.enabled || Prefs.link(this) == null || web == null) return super.dispatchKeyEvent(event)
        when (event.action) {
            KeyEvent.ACTION_DOWN -> {
                if (event.repeatCount == 0) held += button
                // holding both: one "both" action, no repeats
                if (event.repeatCount == 0 || held.size < 2) Buttons.press(button, repeat = event.repeatCount > 0)
            }
            KeyEvent.ACTION_UP -> held -= button
        }
        return true
    }

    @Deprecated("Deprecated in Java")
    override fun onBackPressed() {
        if (fullscreenView != null) fullscreenCallback?.onCustomViewHidden() ?: hideFullscreen()
        else if (System.currentTimeMillis() - backPressedAt < 2000) exitApp()
        else {
            backPressedAt = System.currentTimeMillis()
            Toast.makeText(this, "Press back again to exit", Toast.LENGTH_SHORT).show()
        }
    }

    /** Close the app for real: also stops the volume buttons service and its notification. */
    private fun exitApp() {
        exiting = true
        speech.destroy()
        stopService(Intent(this, ButtonService::class.java))
        finishAndRemoveTask()
    }

    // ---- web app
    private fun load(link: String) {
        val view = web ?: WebView(this).also { w ->
            w.setBackgroundColor(Color.parseColor("#0E0F13"))
            w.settings.javaScriptEnabled = true
            w.settings.domStorageEnabled = true
            w.settings.mediaPlaybackRequiresUserGesture = false
            w.addJavascriptInterface(Bridge(), "SofaAndroid")
            w.webViewClient = object : WebViewClient() {
                // The PC is off or unreachable: show what to do instead of a blank page
                override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                    if (request.isForMainFrame) showOffline(request.url.toString(), error.description.toString())
                }
            }
            w.webChromeClient = object : WebChromeClient() {
                override fun onShowCustomView(view: View, callback: CustomViewCallback) {
                    fullscreenView = view
                    fullscreenCallback = callback
                    (window.decorView as FrameLayout).addView(view, FrameLayout.LayoutParams(-1, -1))
                    systemBars(false)
                }
                override fun onHideCustomView() = hideFullscreen()
            }
            web = w
        }
        setContentView(view)
        view.loadUrl(link)
    }

    private fun showOffline(link: String, reason: String) {
        val pad = (20 * resources.displayMetrics.density).toInt()
        val host = Uri.parse(link).host ?: "the PC"
        val button = { label: String, onClick: () -> Unit ->
            Button(this).apply { text = label; setOnClickListener { onClick() } }
        }
        val box = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad * 3, pad, pad)
            setBackgroundColor(Color.parseColor("#0E0F13"))
            addView(TextView(context).apply {
                text = "Can't reach the PC"
                textSize = 26f
                setTypeface(typeface, Typeface.BOLD)
                setTextColor(Color.parseColor("#E8E9EE"))
            })
            addView(TextView(context).apply {
                text = "Is sofa-remote running on $host, and is this phone on the same Wi-Fi?\n\n($reason)"
                textSize = 16f
                setTextColor(Color.parseColor("#8A8D99"))
                setPadding(0, pad / 2, 0, pad)
            })
            addView(button("Try again") { load(link) }, LinearLayout.LayoutParams(-1, -2))
            addView(button("Scan QR code") { scanQr() }, LinearLayout.LayoutParams(-1, -2))
            addView(button("Connect to another PC") { showSetup(null) }, LinearLayout.LayoutParams(-1, -2))
            addView(button("Exit") { exitApp() }, LinearLayout.LayoutParams(-1, -2))
        }
        setContentView(box, ViewGroup.LayoutParams(-1, -1))
    }

    private fun hideFullscreen() {
        fullscreenView?.let { (window.decorView as FrameLayout).removeView(it) }
        fullscreenView = null
        fullscreenCallback = null
        systemBars(true)
    }

    private fun systemBars(show: Boolean) {
        if (Build.VERSION.SDK_INT >= 30) {
            val c = window.insetsController ?: return
            if (show) c.show(WindowInsets.Type.systemBars())
            else {
                c.hide(WindowInsets.Type.systemBars())
                c.systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            }
        } else {
            @Suppress("DEPRECATION")
            window.decorView.systemUiVisibility = if (show) 0 else
                (View.SYSTEM_UI_FLAG_FULLSCREEN or View.SYSTEM_UI_FLAG_HIDE_NAVIGATION or View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY)
        }
    }

    /** Saves and opens a link like http://192.168.1.20:8765/?k=KEY. */
    private fun useLink(link: String): Boolean {
        val clean = link.trim()
        if (Remote.socketUrl(clean) == null) return false
        Prefs.setLink(this, clean)
        load(clean)
        ButtonService.restart(this, appVisible = true)
        return true
    }

    @Suppress("DEPRECATION")
    private fun scanQr() {
        val options = ScanOptions()
            .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
            .setPrompt("Scan the QR code shown on your PC")
            .setBeepEnabled(false)
            .addExtra(Intents.Scan.SHOW_MISSING_CAMERA_PERMISSION_DIALOG, false)
            .setOrientationLocked(false)
            .setCaptureActivity(QrScannerActivity::class.java)
        startActivityForResult(options.createScanIntent(this), QR_REQUEST)
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != QR_REQUEST) return
        if (data?.getBooleanExtra(Intents.Scan.MISSING_CAMERA_PERMISSION, false) == true) {
            toast("Allow camera permission in app settings to scan QR codes, or paste the PC link")
        }
        if (resultCode != RESULT_OK || data == null) return
        val link = ScanIntentResult.parseActivityResult(resultCode, data).contents ?: return
        if (!useLink(link)) toast("That QR code is not a Sofa Remote connection link")
    }

    // ---- first start / change PC
    private fun showSetup(prefill: String?) {
        val pad = (20 * resources.displayMetrics.density).toInt()
        val fg = Color.parseColor("#E8E9EE")
        val dim = Color.parseColor("#8A8D99")
        val input = EditText(this).apply {
            hint = "http://192.168.1.20:8765/?k=…"
            setText(prefill ?: Prefs.link(this@MainActivity) ?: "")
            setTextColor(fg)
            setHintTextColor(dim)
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
            isSingleLine = true
        }
        val connect = {
            if (!useLink(input.text.toString())) toast("That link has no ?k= key: copy the whole link the PC prints")
        }
        val scan = Button(this).apply {
            text = "Scan the QR code"
            setOnClickListener { scanQr() }
        }
        val ok = Button(this).apply { text = "Connect"; setOnClickListener { connect() } }
        val box = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad * 2, pad, pad)
            gravity = Gravity.CENTER_HORIZONTAL
            addView(TextView(context).apply {
                text = "Sofa Remote"
                textSize = 26f
                setTypeface(typeface, Typeface.BOLD)
                setTextColor(fg)
            })
            addView(TextView(context).apply {
                text = "Run sofa-remote on the PC, then scan the QR code it shows, or paste the link it prints (it contains the access key)."
                textSize = 16f
                setTextColor(dim)
                setPadding(0, pad / 2, 0, pad)
            })
            addView(scan, LinearLayout.LayoutParams(-1, -2))
            addView(input, LinearLayout.LayoutParams(-1, -2).apply { topMargin = pad })
            addView(ok, LinearLayout.LayoutParams(-1, -2))
        }
        setContentView(box, ViewGroup.LayoutParams(-1, -1))
    }

    private fun toast(msg: String) = Toast.makeText(this, msg, Toast.LENGTH_LONG).show()

    /** window.SofaAndroid in the web app. Called on a WebView thread. */
    inner class Bridge {
        @JavascriptInterface
        fun getButtons(): String = Prefs.buttons(this@MainActivity).toJson().toString()

        @JavascriptInterface
        fun setButtons(json: String) = runOnUiThread {
            val old = Prefs.buttons(this@MainActivity)
            val cfg = ButtonConfig.parse(json)
            Prefs.setButtons(this@MainActivity, cfg)
            if (cfg.silent != old.silent) ButtonService.restart(this@MainActivity, appVisible = true)
            else ButtonService.update(this@MainActivity, appVisible = true)
        }

        @JavascriptInterface
        fun startSpeech(language: String) = runOnUiThread {
            if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
                speech.start(language)
            } else {
                pendingSpeech = language
                requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), MIC_REQUEST)
            }
        }

        @JavascriptInterface
        fun stopSpeech() = runOnUiThread { speech.stop() }

        @JavascriptInterface
        fun exit() = runOnUiThread { exitApp() }

        @JavascriptInterface
        fun changePc() = runOnUiThread { showSetup(null) }

        @JavascriptInterface
        fun scanQr() = runOnUiThread { this@MainActivity.scanQr() }

        @JavascriptInterface
        fun showKeyboard() = runOnUiThread {
            val w = web ?: return@runOnUiThread
            w.requestFocus()
            w.postDelayed({
                getSystemService(InputMethodManager::class.java).showSoftInput(w, InputMethodManager.SHOW_IMPLICIT)
            }, 50)
        }
    }

    companion object {
        /** Notification "Exit" button: close the app and stop the volume buttons. */
        const val ACTION_EXIT = "app.sofaremote.EXIT"
        private const val MIC_REQUEST = 2
        private const val QR_REQUEST = 3
    }
}
