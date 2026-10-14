package com.rrgmc.karaokemachine

import android.content.Context
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URI
import org.json.JSONException
import org.json.JSONObject

/**
 * The machine as the headset shell sees it: an HTTP API on loopback, in some process of this app.
 *
 * The shell never calls into Rust. What it needs to know, it asks the API the machine already
 * serves to every remote, so the machine stays unaware that a headset is involved.
 */
internal object Machine {

    /** The API's root on this device, such as `http://127.0.0.1:8177/`. See [Loopback]. */
    fun root(context: Context): String = Loopback.root(context)

    /** Whether the owner serves the singer's remote at `/`. It is on unless turned off. */
    fun servesRemote(context: Context): Boolean = Loopback.servesRemote(context)

    /**
     * Whether leaving now would lose nothing: no song loaded and nobody waiting.
     *
     * A machine that does not answer counts as idle, because a machine that is not running holds
     * nothing to lose. Blocks on the network, so it is never called on the main thread.
     */
    fun idle(context: Context): Boolean = status(context).idle

    /** What the machine is doing, as the headset shell needs to know it. */
    data class Status(val playing: Boolean, val idle: Boolean)

    /**
     * Whether a song is loaded, and whether leaving now would lose nothing, from one request.
     *
     * A machine that does not answer is neither playing nor holding anything. Blocks on the
     * network, so it is never called on the main thread.
     */
    fun status(context: Context): Status {
        val state = state(context) ?: return Status(playing = false, idle = true)
        val playing = !state.isNull(NOW_PLAYING)
        return Status(playing = playing, idle = !playing && state.optInt(QUEUE_LEN, 0) == 0)
    }

    /** Whether a machine answers on this device at all. Blocks on the network. */
    fun answering(context: Context): Boolean = state(context) != null

    private fun state(context: Context): JSONObject? {
        val url = URI(root(context) + STATE_PATH).toURL()
        val connection = url.openConnection() as HttpURLConnection
        connection.connectTimeout = TIMEOUT_MS
        connection.readTimeout = TIMEOUT_MS
        return try {
            if (connection.responseCode != HttpURLConnection.HTTP_OK) {
                null
            } else {
                JSONObject(connection.inputStream.bufferedReader().use { it.readText() })
            }
        } catch (_: IOException) {
            null
        } catch (_: JSONException) {
            null
        } finally {
            connection.disconnect()
        }
    }

    /** `GET /api/v1/state`, public, and the fields of its `StateDto` that say what would be lost. */
    private const val STATE_PATH = "api/v1/state"
    private const val NOW_PLAYING = "now_playing"
    private const val QUEUE_LEN = "queue_len"

    /** Loopback answers at once or not at all. */
    private const val TIMEOUT_MS = 1000
}
