package com.rrgmc.karaokemachine

import android.app.Activity
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Process
import java.io.File
import java.io.IOException

/**
 * The two ways the headset shows the machine: a screen hanging in the room, or a system window.
 *
 * **Each mode runs the machine in a process of its own.** SDL starts the machine once per process.
 * A second machine activity in the same process makes SDL's `onCreate` call `System.exit`, because
 * `SDL_HINT_ANDROID_ALLOW_RECREATE_ACTIVITY` is off. The immersive scene therefore keeps the app's
 * main process, and [FlatActivity] declares `:flat`. A switch starts the other mode and then ends
 * this process, so each machine starts fresh.
 */
internal enum class Mode {
    IMMERSIVE,
    FLAT;

    companion object {
        /**
         * The mode last started, which is the one the library tile opens.
         *
         * A file rather than preferences, because the two modes run in two processes and each
         * process caches its preferences. The first launch opens immersive.
         */
        fun last(context: Context): Mode =
            try {
                if (file(context).readText().trim() == FLAT.name) FLAT else IMMERSIVE
            } catch (_: IOException) {
                IMMERSIVE
            }

        /** Records [mode] as the one running, so the tile and an opened file both find it. */
        fun started(context: Context, mode: Mode) {
            try {
                file(context).writeText(mode.name)
            } catch (_: IOException) {
                // The next launch opens immersive, which is a default rather than a fault.
            }
        }

        private fun file(context: Context) = File(context.filesDir, FILE)

        private const val FILE = "headset_mode"
    }
}

/**
 * Moves the machine from one mode to the other, but only when nothing would be lost.
 *
 * The machine keeps its queue in memory, and a switch ends the process holding it. So a switch
 * waits for an idle machine: no song loaded and nobody waiting. Otherwise [refused] runs, on the
 * main thread, and nothing changes.
 */
internal object Switch {

    fun toFlat(activity: Activity, refused: () -> Unit) =
        whenIdle(activity, refused) {
            // Horizon OS's own route out of an immersive scene: the home environment, told which
            // panel to open once it is there. Starting the panel directly leaves the scene behind it.
            val panel = Intent(activity.applicationContext, FlatActivity::class.java)
                .setAction(Intent.ACTION_MAIN)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            val pending = PendingIntent.getActivity(
                activity.applicationContext,
                0,
                panel,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            activity.startActivity(
                Intent(Intent.ACTION_MAIN)
                    .addCategory(Intent.CATEGORY_HOME)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    .putExtra(HOME_PENDING_INTENT, pending),
            )
        }

    fun toImmersive(activity: Activity, refused: () -> Unit) =
        whenIdle(activity, refused) {
            activity.startActivity(
                Intent(activity, ImmersiveActivity::class.java)
                    .setAction(Intent.ACTION_MAIN)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
        }

    private fun whenIdle(activity: Activity, refused: () -> Unit, start: () -> Unit) {
        Thread {
            val idle = Machine.idle(activity)
            activity.runOnUiThread {
                if (!idle) {
                    refused()
                } else {
                    start()
                    activity.finishAndRemoveTask()
                    // The machine in this process is idle, and the other mode starts its own. Ending
                    // this process is what lets that one bind the port and open the catalog alone.
                    Process.killProcess(Process.myPid())
                }
            }
        }.start()
    }

    /** The extra Horizon OS's home reads for the panel to open, as Meta's `HybridSample` sends it. */
    private const val HOME_PENDING_INTENT = "extra_launch_in_home_pending_intent"
}
