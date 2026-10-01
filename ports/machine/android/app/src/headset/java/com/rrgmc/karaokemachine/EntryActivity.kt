package com.rrgmc.karaokemachine

import android.app.Activity
import android.content.Intent
import android.os.Bundle

/**
 * The headset's one way in: the library tile, and a `.kmpkg` opened from a file manager.
 *
 * It shows nothing and hands on at once. The tile opens the mode last used. A file goes to the
 * machine already running, in whichever mode that is, so it never starts a second machine beside
 * the first. With no machine running, the file starts the last mode and that machine opens it.
 */
class EntryActivity : Activity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val opened = intent
        if (opened.action != Intent.ACTION_VIEW) {
            startActivity(start(Mode.last(this)).setAction(Intent.ACTION_MAIN))
            finish()
            return
        }
        // Asking whether a machine answers is network, which the main thread may not do.
        Thread {
            val running = Machine.answering(this)
            runOnUiThread {
                startActivity(forward(opened, Mode.last(this), running))
                finish()
            }
        }.start()
    }

    private fun start(mode: Mode): Intent =
        Intent(this, if (mode == Mode.FLAT) FlatActivity::class.java else ImmersiveActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    /**
     * The opened file, readdressed to the activity that holds, or will hold, the machine.
     *
     * A running immersive machine is [MainActivity] on the scene's panel. It is `singleInstance`, so
     * the file reaches it through `onNewIntent`. Otherwise the mode's own activity takes the file,
     * and a cold [ImmersiveActivity] hands it on to the panel it creates.
     */
    private fun forward(opened: Intent, mode: Mode, running: Boolean): Intent {
        val target = when {
            mode == Mode.FLAT -> FlatActivity::class.java
            running -> MainActivity::class.java
            else -> ImmersiveActivity::class.java
        }
        return Intent(opened).setClass(this, target).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }
}
