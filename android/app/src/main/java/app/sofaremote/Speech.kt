package app.sofaremote

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer

/** Speech to text for the web app's keyboard 🎤 (WebView has no Web Speech API).
 *  [emit] gets (type, text): "partial"/"final" with the words, "error" with a message, "end".
 *  Call everything on the main thread. */
class Speech(private val context: Context, private val emit: (String, String) -> Unit) {
    private var recognizer: SpeechRecognizer? = null

    fun start(language: String) {
        stop()
        if (!SpeechRecognizer.isRecognitionAvailable(context)) {
            emit("error", "No speech recognition on this phone (install or enable Google voice typing)")
            emit("end", "")
            return
        }
        val r = SpeechRecognizer.createSpeechRecognizer(context)
        recognizer = r
        r.setRecognitionListener(object : RecognitionListener {
            override fun onPartialResults(b: Bundle) { best(b)?.let { emit("partial", it) } }
            override fun onResults(b: Bundle) {
                best(b)?.let { emit("final", it) }
                finish(r)
            }
            override fun onError(error: Int) {
                // no match / silence just ends quietly
                if (error != SpeechRecognizer.ERROR_NO_MATCH && error != SpeechRecognizer.ERROR_SPEECH_TIMEOUT) {
                    emit("error", message(error))
                }
                finish(r)
            }
            override fun onReadyForSpeech(p: Bundle?) {}
            override fun onBeginningOfSpeech() {}
            override fun onRmsChanged(rms: Float) {}
            override fun onBufferReceived(buffer: ByteArray?) {}
            override fun onEndOfSpeech() {}
            override fun onEvent(type: Int, p: Bundle?) {}
        })
        r.startListening(Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
            putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
            if (language.isNotEmpty()) putExtra(RecognizerIntent.EXTRA_LANGUAGE, language)
        })
    }

    /** Stop listening; what was heard so far still arrives as the final result. */
    fun stop() {
        recognizer?.stopListening()
    }

    /** Cancel (the app went to the background). */
    fun destroy() {
        val r = recognizer ?: return
        r.destroy()
        recognizer = null
        emit("end", "")
    }

    private fun finish(r: SpeechRecognizer) {
        if (recognizer === r) {
            r.destroy()
            recognizer = null
        }
        emit("end", "")
    }

    private fun best(b: Bundle): String? =
        b.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()?.takeIf { it.isNotBlank() }

    private fun message(error: Int) = when (error) {
        SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> "Microphone permission denied"
        SpeechRecognizer.ERROR_NETWORK, SpeechRecognizer.ERROR_NETWORK_TIMEOUT -> "Voice input needs internet on this phone"
        SpeechRecognizer.ERROR_AUDIO -> "Microphone unavailable"
        SpeechRecognizer.ERROR_RECOGNIZER_BUSY -> "Voice input busy, try again"
        else -> "Voice input failed ($error)"
    }
}
