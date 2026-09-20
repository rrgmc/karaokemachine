package com.rrgmc.karaokemachine

import android.content.Context
import android.graphics.Color
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import com.meta.spatial.core.Entity
import com.meta.spatial.core.Pose
import com.meta.spatial.core.SpatialFeature
import com.meta.spatial.core.Vector3
import com.meta.spatial.isdk.IsdkFeature
import com.meta.spatial.runtime.LayerConfig
import com.meta.spatial.runtime.PanelSceneObject
import com.meta.spatial.runtime.PanelShapeType
import com.meta.spatial.runtime.ReferenceSpace
import com.meta.spatial.toolkit.AppSystemActivity
import com.meta.spatial.toolkit.PanelRegistration
import com.meta.spatial.toolkit.Transform
import com.meta.spatial.toolkit.createPanelEntity
import com.meta.spatial.vr.VRFeature

/**
 * The immersive shell: one screen hanging in the room, with the machine drawing on it.
 *
 * The machine itself is [MainActivity], unchanged and shared with the phone and the television.
 * Spatial SDK hosts it as a panel, so the renderer stays OpenGL ES and no Rust knows a headset is
 * involved. See `What the machine *is*, on a headset` in docs/decisions/distribution.md.
 */
class ImmersiveActivity : AppSystemActivity() {

    /**
     * The screen's shape, remembered in the headset rather than in `settings.json`.
     *
     * Flat or curved is a property of where somebody stands, the way a window's position is a
     * property of a desktop. A machine that drives a television has one screen shape and no use for
     * a second one in its settings file.
     */
    private enum class Shape {
        FLAT,
        CURVED,
    }

    private fun shape(): Shape =
        if (prefs().getBoolean(CURVED, false)) Shape.CURVED else Shape.FLAT

    private fun prefs() = getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /**
     * Controllers and hands both, because a karaoke machine is pointed at rather than typed on.
     *
     * `VRFeature` alone gives a ray from a controller and nothing from a hand, so a headset whose
     * controllers are flat has no way to reach the keypad. `IsdkFeature` is what draws the hand's
     * own ray. The manifest declares `oculus.software.handtracking` beside it, which is also what
     * lets Horizon OS start this at all when no controller is awake.
     */
    override fun registerFeatures(): List<SpatialFeature> =
        listOf(VRFeature(this), IsdkFeature(this, spatial, systemManager))

    override fun onSceneReady() {
        super.onSceneReady()

        // The floor-relative recentred space, so the headset's own Reset View brings the screen
        // round. Pinning the view origin takes that away, and a screen the wearer cannot bring back
        // in front of them is a screen in the wrong place for good.
        scene.setReferenceSpace(ReferenceSpace.LOCAL_FLOOR)

        // The room behind the screen. A scene that never asks draws a black void, and the manifest's
        // `com.oculus.feature.PASSTHROUGH` is what lets Horizon OS grant the layer. Both halves are
        // needed and neither reports its absence.
        scene.enablePassthrough(true)
        scene.enableHolePunching(true)

        Entity.createPanelEntity(
            R.id.machine_panel,
            Transform(Pose(Vector3(0.0f, EYE_HEIGHT, SCREEN_DISTANCE))),
        )
        Entity.createPanelEntity(
            R.id.shape_panel,
            Transform(Pose(Vector3(0.0f, EYE_HEIGHT - 0.75f, SCREEN_DISTANCE - 0.4f))),
        )
    }

    override fun registerPanels(): List<PanelRegistration> =
        listOf(machinePanel(), shapePanel())

    /** The machine, on the screen the wearer chose. */
    private fun machinePanel() =
        PanelRegistration(R.id.machine_panel) {
            activityClass = MainActivity::class.java
            config {
                width = SCREEN_WIDTH
                height = SCREEN_WIDTH * 9.0f / 16.0f
                layoutWidthInDp = 1600f
                layoutHeightInDp = 900f
                // A compositor layer rather than a textured mesh. Mesh rendering is documented as
                // unsuitable for text, and lyrics are the whole point of the screen.
                layerConfig = LayerConfig()
                enableTransparent = false
                includeGlass = false
                if (shape() == Shape.CURVED) {
                    panelShapeType = PanelShapeType.CYLINDER
                    radiusForCylinderOrSphere = SCREEN_DISTANCE
                }
            }
            // Kept so the shape can change without restarting anything. A song is playing on this
            // panel, and rebuilding the scene to bend a screen would stop it.
            panel { screen = this }
        }

    /** Bends or flattens the screen in place, leaving the song playing on it alone. */
    private fun reshape(curved: Boolean) {
        val panel = screen ?: return
        val config = panel.panelShapeConfig ?: return
        config.panelShapeType = if (curved) PanelShapeType.CYLINDER else PanelShapeType.QUAD
        config.radiusForCylinderOrSphere = SCREEN_DISTANCE
        panel.reshape(config)
    }

    /** Two buttons under the screen, because the shape is the wearer's to pick. */
    private fun shapePanel() =
        PanelRegistration(R.id.shape_panel) {
            view { context -> shapeControls(context) }
            config {
                width = 0.6f
                height = 0.16f
                layoutWidthInDp = 600f
                layoutHeightInDp = 160f
                layerConfig = LayerConfig()
                enableTransparent = false
                includeGlass = false
            }
        }

    private fun shapeControls(context: Context): View {
        val row = LinearLayout(context)
        row.orientation = LinearLayout.HORIZONTAL
        row.gravity = Gravity.CENTER
        row.setBackgroundColor(Color.parseColor("#101418"))

        val label = TextView(context)
        label.text = getString(R.string.headset_screen_shape)
        label.setTextColor(Color.WHITE)
        label.textSize = 18f
        label.setPadding(24, 0, 24, 0)
        row.addView(label)

        row.addView(shapeButton(context, R.string.headset_screen_flat, false))
        row.addView(shapeButton(context, R.string.headset_screen_curved, true))
        return row
    }

    private fun shapeButton(context: Context, label: Int, curved: Boolean): Button {
        val button = Button(context)
        button.text = getString(label)
        button.textSize = 18f
        button.setOnClickListener {
            prefs().edit().putBoolean(CURVED, curved).apply()
            reshape(curved)
        }
        return button
    }

    /** The machine's own panel, once the scene has built it. */
    private var screen: PanelSceneObject? = null

    private companion object {
        const val PREFS = "headset"
        const val CURVED = "screen_curved"

        /** Metres. A television's distance, at a television's size. */
        const val SCREEN_DISTANCE = 2.0f
        const val SCREEN_WIDTH = 2.0f
        const val EYE_HEIGHT = 1.4f
    }
}
