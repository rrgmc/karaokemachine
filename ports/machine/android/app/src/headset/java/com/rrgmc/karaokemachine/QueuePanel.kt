package com.rrgmc.karaokemachine

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.view.View
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient

/**
 * The singer's remote, served by the machine in this same process and shown beside the screen.
 *
 * The machine already serves its remote at `/` to every phone in the room. On a headset the wearer
 * holds no phone, so the scene puts that page on a panel and reaches it over loopback. Nothing here
 * is a second remote; the page is the one a phone gets.
 */
internal object QueuePanel {

    /** The remote's address on this device, or null when the owner has turned the remote off. */
    fun address(context: Context): String? =
        if (Machine.servesRemote(context)) Machine.root(context) else null

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

    private const val RETRY_MS = 2000L
}
