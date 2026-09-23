package com.rrgmc.karaokemachine

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.view.View
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import java.io.File
import org.json.JSONException
import org.json.JSONObject

/**
 * The singer's remote, served by the machine in this same process and shown beside the screen.
 *
 * The machine already serves its remote at `/` to every phone in the room. On a headset the wearer
 * holds no phone, so the scene puts that page on a panel and reaches it over loopback. Nothing here
 * is a second remote; the page is the one a phone gets.
 */
internal object QueuePanel {

    /**
     * The remote's address on this device, or null when the owner has turned the remote off.
     *
     * Read from the machine's own `settings.json`, in the internal storage directory SDL reports to
     * the machine. A missing or unreadable file means the defaults, which are the port below and
     * the remote on. The machine writes the file on its first start, and that can come after this.
     */
    fun address(context: Context): String? {
        val api = readApi(File(context.filesDir, SETTINGS_FILE))
        if (api?.opt(SERVE_REMOTE) == false) return null
        val port = api?.optString(BIND)?.substringAfterLast(':')?.toIntOrNull() ?: DEFAULT_PORT
        return "http://127.0.0.1:$port/"
    }

    /**
     * The page, retrying until the machine answers.
     *
     * The scene is ready before the machine has bound its port. A first load that fails is
     * therefore the normal case at launch, so the view tries again rather than showing an error.
     */
    @SuppressLint("SetJavaScriptEnabled")
    fun view(context: Context, address: String): View {
        val web = WebView(context)
        web.setBackgroundColor(Color.parseColor("#101418"))
        // The remote is htmx: it swaps fragments with script and keeps its preferences in storage.
        web.settings.javaScriptEnabled = true
        web.settings.domStorageEnabled = true
        web.webViewClient = object : WebViewClient() {
            override fun onReceivedError(
                view: WebView,
                request: WebResourceRequest,
                error: WebResourceError,
            ) {
                if (request.isForMainFrame) {
                    view.postDelayed({ view.loadUrl(address) }, RETRY_MS)
                }
            }
        }
        web.loadUrl(address)
        return web
    }

    private fun readApi(file: File): JSONObject? =
        try {
            if (file.isFile) JSONObject(file.readText()).optJSONObject(API) else null
        } catch (_: JSONException) {
            null
        } catch (_: java.io.IOException) {
            null
        }

    /** The file and keys `crates/machine/karaokemachine/src/settings.rs` writes. */
    private const val SETTINGS_FILE = "settings.json"
    private const val API = "api"
    private const val BIND = "bind"
    private const val SERVE_REMOTE = "serve_remote"

    /** `km_api::DEFAULT_PORT`, which the machine binds when the file names none. */
    private const val DEFAULT_PORT = 8177

    private const val RETRY_MS = 2000L
}
