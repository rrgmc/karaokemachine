package com.rrgmc.karaokemachine

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import androidx.compose.ui.platform.ComposeView
import com.meta.spatial.compose.ComposeFeature
import com.meta.spatial.compose.composePanel
import com.meta.spatial.core.Entity
import com.meta.spatial.core.Pose
import com.meta.spatial.core.SpatialFeature
import com.meta.spatial.core.Vector2
import com.meta.spatial.core.Vector3
import com.meta.spatial.isdk.IsdkFeature
import com.meta.spatial.isdk.IsdkGrabState
import com.meta.spatial.isdk.IsdkGrabbable
import com.meta.spatial.isdk.IsdkPanelResize
import com.meta.spatial.isdk.ResizeCornerState
import com.meta.spatial.isdk.ResizeMode
import com.meta.spatial.mruk.MRUKFeature
import com.meta.spatial.mruk.MRUKLoadDeviceResult
import com.meta.spatial.mruk.MRUKRoom
import com.meta.spatial.runtime.LayerConfig
import com.meta.spatial.runtime.PanelSceneObject
import com.meta.spatial.runtime.PanelShapeType
import com.meta.spatial.runtime.ReferenceSpace
import com.meta.spatial.toolkit.AppSystemActivity
import com.meta.spatial.toolkit.PanelRegistration
import com.meta.spatial.toolkit.Scale
import com.meta.spatial.toolkit.Transform
import com.meta.spatial.toolkit.Visible
import com.meta.spatial.toolkit.createPanelEntity
import com.meta.spatial.vr.VRFeature

/**
 * The immersive shell: one screen hanging in the room, with the machine drawing on it.
 *
 * The machine itself is [MainActivity], unchanged and shared with the phone and the television.
 * Spatial SDK hosts it as a panel, so the renderer stays OpenGL ES and no Rust knows a headset is
 * involved. See `What the machine *is*, on a headset` in docs/decisions/distribution.md.
 *
 * The wearer places the screen. It starts on the main wall of the scanned room, moves and resizes
 * by hand, and comes back where it was left. The singer's remote hangs beside it. A button under it
 * takes the machine into a system window, which is [FlatActivity].
 */
class ImmersiveActivity : AppSystemActivity() {

    private fun prefs() = getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private val placement by lazy { Placement(prefs()) }

    private val controls by lazy {
        ControlsState(
            curved = prefs().getBoolean(CURVED, false),
            queueShown = prefs().getBoolean(QUEUE_SHOWN, true),
        )
    }

    private val mruk by lazy { MRUKFeature(this, systemManager) }

    /** The machine's panel, once the scene has built it, for bending it in place. */
    private var screen: PanelSceneObject? = null

    private var screenEntity: Entity? = null
    private var controlsEntity: Entity? = null
    private var queueEntity: Entity? = null

    /** The scanned room, once the headset has handed it over. Null without the permission. */
    private var room: MRUKRoom? = null

    private var sceneReady = false

    /** Whether the wearer held the screen on the last frame, so letting go is seen once. */
    private var handling = false

    /** The remote's address, or null when the owner has turned the remote off. */
    private val queueAddress by lazy { QueuePanel.address(this) }

    /**
     * Controllers and hands both, because a karaoke machine is pointed at rather than typed on.
     *
     * `VRFeature` alone gives a ray from a controller and nothing from a hand, so a headset whose
     * controllers are flat has no way to reach the keypad. `IsdkFeature` is what draws the hand's
     * own ray, and it is also what grabs and resizes a panel. The manifest declares
     * `oculus.software.handtracking` beside it, which is also what lets Horizon OS start this at
     * all when no controller is awake.
     *
     * `MRUKFeature` reads the room the headset scanned, and `ComposeFeature` draws the controls.
     * [debugFeatures] adds the metrics overlay to a debug build and nothing to a release one.
     */
    override fun registerFeatures(): List<SpatialFeature> =
        listOf(
            VRFeature(this),
            IsdkFeature(this, spatial, systemManager),
            ComposeFeature(),
            mruk,
        ) + debugFeatures(this)

    override fun onCreate(savedInstanceState: Bundle?) {
        Mode.started(this, Mode.IMMERSIVE)
        super.onCreate(savedInstanceState)
        controls.queueAvailable = queueAddress != null
        if (!sceneAllowed()) {
            requestPermissions(arrayOf(USE_SCENE), SCENE_REQUEST)
        }
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == SCENE_REQUEST && sceneAllowed() && sceneReady) loadRoom()
    }

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

        // Straight ahead until the room says otherwise. Without the permission, or in a room that
        // was never scanned, this is where the screen stays until the wearer moves it.
        val ahead = Vector3(0.0f, Placement.EYE_HEIGHT, SCREEN_DISTANCE)
        screenEntity = Entity.createPanelEntity(
            R.id.machine_panel,
            Transform(Pose(ahead)),
            IsdkGrabbable(),
            // `Simple` scales the quad and leaves the 1600x900 dp layout alone. SDL is therefore
            // never resized under a playing song, and the lyrics only grow.
            IsdkPanelResize(
                true,
                ResizeMode.Simple,
                Vector2(Placement.NARROWEST, Placement.NARROWEST * 9.0f / 16.0f),
                Vector2(WIDEST, WIDEST * 9.0f / 16.0f),
                true,
            ),
        )
        controlsEntity = Entity.createPanelEntity(
            R.id.controls_panel,
            Transform(Pose(ahead)),
            IsdkGrabbable(),
        )
        if (queueAddress != null) {
            queueEntity = Entity.createPanelEntity(
                R.id.queue_panel,
                Transform(Pose(ahead)),
                IsdkGrabbable(),
                Visible(controls.queueShown),
            )
        }
        put(ScreenPlace(Pose(ahead), SCREEN_WIDTH))

        sceneReady = true
        if (sceneAllowed()) loadRoom()
    }

    /**
     * Notices the wearer letting go of the screen, and remembers where it went.
     *
     * A grab and a resize both end here. Saving once at the release, rather than every frame, keeps
     * the preferences file out of the frame loop.
     */
    override fun onSceneTick() {
        super.onSceneTick()
        val entity = screenEntity ?: return
        val grabbed = entity.tryGetComponent<IsdkGrabbable>()?.grabState == IsdkGrabState.Grabbed
        val corner = entity.tryGetComponent<IsdkPanelResize>()?.activeResizeCorner
        val now = grabbed || (corner != null && corner != ResizeCornerState.NONE)
        if (handling && !now) remember()
        handling = now
    }

    override fun onPause() {
        remember()
        super.onPause()
    }

    override fun registerPanels(): List<PanelRegistration> =
        listOfNotNull(machinePanel(), controlsPanel(), queueAddress?.let(::queuePanel))

    private fun sceneAllowed() =
        checkSelfPermission(USE_SCENE) == PackageManager.PERMISSION_GRANTED

    /**
     * Asks the headset for the scanned room, then puts the screen back or on the wall.
     *
     * Every failure leaves the screen where it is, and none of them is shown. A room never scanned
     * and a refused permission both mean the wearer places the screen by hand, which always works.
     */
    private fun loadRoom() {
        mruk.loadSceneFromDevice().thenAccept { result ->
            runOnUiThread {
                if (result != MRUKLoadDeviceResult.SUCCESS) return@runOnUiThread
                val found = mruk.getCurrentRoom() ?: mruk.rooms.firstOrNull() ?: return@runOnUiThread
                room = found
                controls.roomKnown = true
                val place = placement.restore(found) ?: placement.onWall(found, scene.getViewerPose(), SCREEN_WIDTH)
                if (place != null) put(place)
            }
        }
    }

    /**
     * The screen at [place] and at its width, with the controls under it and the queue beside it.
     *
     * The two small panels come out towards the wearer by [NEARER], so a screen on a wall never
     * swallows them. Each stays grabbable, and neither follows the screen once the wearer moves it.
     */
    private fun put(place: ScreenPlace) {
        screenEntity?.setComponent(Transform(place.pose))
        screenEntity?.setComponent(Scale(Vector3(place.width / SCREEN_WIDTH)))
        val height = place.width * 9.0f / 16.0f
        // A panel's local negative Z points out of its face, towards whoever is looking at it.
        val under = Vector3(0.0f, -(height / 2.0f + CONTROLS_GAP), -NEARER)
        val beside = Vector3(-(place.width / 2.0f + QUEUE_GAP), 0.0f, -NEARER)
        controlsEntity?.setComponent(Transform(place.pose.times(Pose(under))))
        queueEntity?.setComponent(Transform(place.pose.times(Pose(beside))))
    }

    /** Saves where the screen is, against the room. Without a room there is nothing to save to. */
    private fun remember() {
        val found = room ?: return
        val entity = screenEntity ?: return
        val pose = entity.tryGetComponent<Transform>()?.transform ?: return
        val scale = entity.tryGetComponent<Scale>()?.scale?.x ?: 1.0f
        placement.save(found, ScreenPlace(pose, SCREEN_WIDTH * scale))
    }

    /**
     * The machine, on the screen the wearer chose.
     *
     * A `.kmpkg` that started the scene rides on to the machine in the panel's intent, so the
     * machine opens it as it would on a phone. Spatial SDK takes either an activity class or an
     * intent, and never both.
     */
    private fun machinePanel() =
        PanelRegistration(R.id.machine_panel) {
            val opened = intent
            if (opened?.action == Intent.ACTION_VIEW) {
                panelIntent = Intent(opened).setClass(this@ImmersiveActivity, MainActivity::class.java)
            } else {
                activityClass = MainActivity::class.java
            }
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
                if (controls.curved) {
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

    private val actions = object : ControlsActions {
        override fun shape(curved: Boolean) {
            prefs().edit().putBoolean(CURVED, curved).apply()
            controls.curved = curved
            reshape(curved)
        }

        override fun toWall() {
            val found = room ?: return
            val place = placement.onWall(found, scene.getViewerPose(), SCREEN_WIDTH) ?: return
            put(place)
            placement.save(found, place)
        }

        override fun toWindow() {
            controls.refused = false
            Switch.toFlat(this@ImmersiveActivity) { controls.refused = true }
        }

        override fun toggleQueue() {
            val shown = !controls.queueShown
            prefs().edit().putBoolean(QUEUE_SHOWN, shown).apply()
            controls.queueShown = shown
            queueEntity?.setComponent(Visible(shown))
        }
    }

    /** One row of buttons under the screen: its shape, the wall, and the queue. */
    private fun controlsPanel(): PanelRegistration {
        val content: (ComposeView) -> Unit = { view ->
            view.setContent { Controls(controls, actions) }
        }
        return PanelRegistration(R.id.controls_panel) {
            composePanel(content)
            config {
                width = 1.6f
                height = 0.16f
                layoutWidthInDp = 1600f
                layoutHeightInDp = 160f
                layerConfig = LayerConfig()
                enableTransparent = false
                includeGlass = false
            }
        }
    }

    /** The singer's remote, beside the screen, so a wearer needs no phone to queue a song. */
    private fun queuePanel(address: String) =
        PanelRegistration(R.id.queue_panel) {
            view { context -> QueuePanel.view(context, address) }
            config {
                width = 0.9f
                height = 1.2f
                layoutWidthInDp = 540f
                layoutHeightInDp = 720f
                layerConfig = LayerConfig()
                enableTransparent = false
                includeGlass = false
            }
        }

    private companion object {
        const val PREFS = "headset"
        const val CURVED = "screen_curved"
        const val QUEUE_SHOWN = "queue_shown"

        /** Horizon OS's permission for the room the headset scanned. */
        const val USE_SCENE = "com.oculus.permission.USE_SCENE"
        const val SCENE_REQUEST = 1

        /** Metres. A television's distance, at a television's size. */
        const val SCREEN_DISTANCE = 2.0f
        const val SCREEN_WIDTH = 2.0f

        /** Metres. The widest a hand may stretch the screen. */
        const val WIDEST = 4.0f

        /** Metres. How far the controls and the queue stand out in front of the screen. */
        const val NEARER = 0.4f

        /** Metres between the screen's lower edge and the controls. */
        const val CONTROLS_GAP = 0.2f

        /** Metres from the screen's left edge to the queue's centre, for a queue 0.9 m wide. */
        const val QUEUE_GAP = 0.55f
    }
}
