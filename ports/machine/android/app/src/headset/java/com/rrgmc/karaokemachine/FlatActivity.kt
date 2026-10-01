package com.rrgmc.karaokemachine

import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.FrameLayout
import android.widget.Toast

/**
 * The machine in a system window on the headset, with one button that takes it into the room.
 *
 * It is [MainActivity] and nothing more, run in the `:flat` process. See [Mode] for why that process
 * is its own. The button is Android's rather than the machine's, so no Rust learns about headsets.
 *
 * Horizon OS's own window menu takes no item from an application, so the button is the only way
 * back. It shows only while the machine is idle, which is when [Switch] allows the move. A song on
 * the screen is therefore never covered by it.
 */
class FlatActivity : MainActivity() {

    private val main = Handler(Looper.getMainLooper())
    private lateinit var button: Button
    private var watching = false

    private val check = object : Runnable {
        override fun run() {
            if (!watching) return
            Thread {
                val idle = Machine.idle(this@FlatActivity)
                runOnUiThread { button.visibility = if (idle) View.VISIBLE else View.GONE }
            }.start()
            main.postDelayed(this, CHECK_MS)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        Mode.started(this, Mode.FLAT)
        super.onCreate(savedInstanceState)
        button = immersiveButton()
        addContentView(button, corner())
    }

    override fun onResume() {
        super.onResume()
        watching = true
        main.post(check)
    }

    override fun onPause() {
        watching = false
        main.removeCallbacks(check)
        super.onPause()
    }

    private fun immersiveButton(): Button {
        val button = Button(this)
        button.text = getString(R.string.headset_to_immersive)
        button.isAllCaps = false
        button.setTextSize(TypedValue.COMPLEX_UNIT_SP, TEXT_SP)
        button.setTextColor(Color.WHITE)
        button.setPadding(dp(PAD_X_DP), dp(PAD_Y_DP), dp(PAD_X_DP), dp(PAD_Y_DP))
        button.background = GradientDrawable().apply {
            cornerRadius = dp(RADIUS_DP).toFloat()
            setColor(Color.parseColor("#1F6FEB"))
        }
        // Hidden until the first check says the machine is idle.
        button.visibility = View.GONE
        button.setOnClickListener {
            Switch.toImmersive(this) {
                Toast.makeText(this, R.string.headset_switch_refused, Toast.LENGTH_LONG).show()
            }
        }
        return button
    }

    /** The bottom right corner of the window, beside Horizon OS's own window bar. */
    private fun corner() =
        FrameLayout.LayoutParams(
            FrameLayout.LayoutParams.WRAP_CONTENT,
            FrameLayout.LayoutParams.WRAP_CONTENT,
            Gravity.BOTTOM or Gravity.END,
        ).apply { setMargins(0, 0, dp(MARGIN_DP), dp(MARGIN_DP)) }

    private fun dp(value: Int) =
        TypedValue.applyDimension(
            TypedValue.COMPLEX_UNIT_DIP,
            value.toFloat(),
            resources.displayMetrics,
        ).toInt()

    private companion object {
        /** How often the window asks the machine whether it is idle. Loopback, so it is cheap. */
        const val CHECK_MS = 2000L

        const val TEXT_SP = 22f
        const val PAD_X_DP = 28
        const val PAD_Y_DP = 14
        const val RADIUS_DP = 28
        const val MARGIN_DP = 24
    }
}
