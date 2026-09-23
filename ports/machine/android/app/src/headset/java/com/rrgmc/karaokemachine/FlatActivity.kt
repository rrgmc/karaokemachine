package com.rrgmc.karaokemachine

import android.graphics.Color
import android.os.Bundle
import android.view.Gravity
import android.widget.Button
import android.widget.FrameLayout
import android.widget.Toast

/**
 * The machine in a system window on the headset, with one button that takes it into the room.
 *
 * It is [MainActivity] and nothing more, run in the `:flat` process. See [Mode] for why that process
 * is its own. The button is Android's rather than the machine's, so no Rust learns about headsets.
 */
class FlatActivity : MainActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        Mode.started(this, Mode.FLAT)
        super.onCreate(savedInstanceState)
        addContentView(immersiveButton(), corner())
    }

    private fun immersiveButton(): Button {
        val button = Button(this)
        button.text = getString(R.string.headset_to_immersive)
        button.alpha = BUTTON_ALPHA
        button.setTextColor(Color.WHITE)
        button.setBackgroundColor(Color.parseColor("#101418"))
        button.setOnClickListener {
            Switch.toImmersive(this) {
                Toast.makeText(this, R.string.headset_switch_refused, Toast.LENGTH_LONG).show()
            }
        }
        return button
    }

    /** The top right corner of the window. */
    private fun corner() =
        FrameLayout.LayoutParams(
            FrameLayout.LayoutParams.WRAP_CONTENT,
            FrameLayout.LayoutParams.WRAP_CONTENT,
            Gravity.TOP or Gravity.END,
        ).apply { setMargins(0, MARGIN, MARGIN, 0) }

    private companion object {
        /** Faint until needed, because it sits over the words. */
        const val BUTTON_ALPHA = 0.7f
        const val MARGIN = 16
    }
}
