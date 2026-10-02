package com.rrgmc.karaokemachine

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.compose.ui.platform.ComposeView
import com.meta.spatial.compose.ComposeFeature
import com.meta.spatial.compose.composePanel
import com.meta.spatial.core.Entity
import com.meta.spatial.core.Pose
import com.meta.spatial.core.Quaternion
import com.meta.spatial.core.SpatialFeature
import com.meta.spatial.core.Vector2
import com.meta.spatial.core.Vector3
import com.meta.spatial.isdk.IsdkGrabMovementType
import com.meta.spatial.isdk.IsdkGrabState
import com.meta.spatial.isdk.IsdkGrabbable
import com.meta.spatial.isdk.IsdkPanelResize
import com.meta.spatial.isdk.ResizeCornerState
import com.meta.spatial.isdk.ResizeMode
import com.meta.spatial.mruk.MRUKFeature
import com.meta.spatial.mruk.MRUKLoadDeviceResult
import com.meta.spatial.mruk.MRUKRoom
import com.meta.spatial.runtime.BlendFactor
import com.meta.spatial.runtime.LayerAlphaBlend
import com.meta.spatial.runtime.LayerConfig
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
 * by hand, and comes back where it was left. The singer's remote hangs beside it. A pill of controls
 * rides beside it and steps aside while a song plays. One of its buttons takes the machine into a
 * system window, which is [FlatActivity].
 */
class ImmersiveActivity : AppSystemActivity() {

    private fun prefs() = getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private val placement by lazy { Placement(prefs()) }

    private val controls by lazy { ControlsState(queueShown = prefs().getBoolean(QUEUE_SHOWN, true)) }

    private val mruk by lazy { MRUKFeature(this, systemManager) }

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
     * `VRFeature` registers Meta's Interaction SDK itself, which draws both rays and grabs and
     * resizes a panel. **Do not register `IsdkFeature` beside it.** A second registration takes the
     * ray off both controllers, and nothing in the log says so. The manifest declares
     * `oculus.software.handtracking` beside it, which is also what lets Horizon OS start this at
     * all when no controller is awake.
     *
     * `MRUKFeature` reads the room the headset scanned, and `ComposeFeature` draws the controls.
     * [debugFeatures] adds the metrics overlay to a debug build and nothing to a release one.
     */
    override fun registerFeatures(): List<SpatialFeature> =
        listOf(
            VRFeature(this),
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
            // Turns about the vertical to face the wearer while it is carried, so a hand never
            // leaves it at an angle.
            IsdkGrabbable(
                true,
                IsdkGrabState.NotGrabbed,
                IsdkGrabMovementType.AxialBillboard,
                GRAB_RESPONSIVENESS,
            ),
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
        // Not grabbable: the controls belong to the screen and follow it, see [placeControls].
        controlsEntity = Entity.createPanelEntity(
            R.id.controls_panel,
            Transform(Pose(ahead)),
            Visible(true),
        )
        if (queueAddress != null) {
            queueEntity = Entity.createPanelEntity(
                R.id.queue_panel,
                Transform(Pose(ahead)),
                // Turns about the vertical to face the wearer while it is carried.
                IsdkGrabbable(
                    true,
                    IsdkGrabState.NotGrabbed,
                    IsdkGrabMovementType.AxialBillboard,
                    GRAB_RESPONSIVENESS,
                ),
                // `Simple` for the same reason as the screen: the WebView keeps its layout and the
                // page only grows.
                IsdkPanelResize(
                    true,
                    ResizeMode.Simple,
                    Vector2(QUEUE_WIDTH * QUEUE_SMALLEST, QUEUE_HEIGHT * QUEUE_SMALLEST),
                    Vector2(QUEUE_WIDTH * QUEUE_LARGEST, QUEUE_HEIGHT * QUEUE_LARGEST),
                    true,
                ),
                Visible(controls.queueShown),
            )
        }
        putAhead()

        sceneReady = true
        if (sceneAllowed()) loadRoom()
    }

    /**
     * Carries the controls with the screen while the wearer holds it, and remembers where it went.
     *
     * A grab and a resize both end here. The controls move only while the screen does, and saving
     * happens once at the release, so a still screen costs nothing per frame.
     */
    override fun onSceneTick() {
        super.onSceneTick()
        val entity = screenEntity ?: return
        val grabbed = entity.tryGetComponent<IsdkGrabbable>()?.grabState == IsdkGrabState.Grabbed
        val corner = entity.tryGetComponent<IsdkPanelResize>()?.activeResizeCorner
        val now = grabbed || (corner != null && corner != ResizeCornerState.NONE)
        if (now || handling) placeControls()
        if (handling && !now) remember()
        handling = now
    }

    override fun onResume() {
        super.onResume()
        watching = true
        main.post(watch)
    }

    override fun onPause() {
        watching = false
        main.removeCallbacks(watch)
        remember()
        super.onPause()
    }

    private val main = Handler(Looper.getMainLooper())
    private var watching = false

    /**
     * Asks the machine what it is doing, every [WATCH_MS] while the scene is in front.
     *
     * The controls step aside while a song plays, so nothing but the lyrics holds the eye. The
     * window button shows only when a switch would lose nothing.
     */
    private val watch = object : Runnable {
        override fun run() {
            if (!watching) return
            Thread {
                val status = Machine.status(this@ImmersiveActivity)
                runOnUiThread {
                    controls.idle = status.idle
                    controlsEntity?.setComponent(Visible(!status.playing))
                }
            }.start()
            main.postDelayed(this, WATCH_MS)
        }
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
                // Nothing is shown to the wearer, so the log is the only place a failure is seen.
                Log.i(TAG, "room scan: $result, rooms=${mruk.rooms.size}")
                // By now the headset is tracking, so in front of the wearer means where they are.
                if (result != MRUKLoadDeviceResult.SUCCESS) return@runOnUiThread putAhead()
                // Only the room the wearer stands in. A headset holds every room it has scanned,
                // and taking one of the others hangs the screen on a wall in another part of the
                // house, out of reach. Outside every scanned room there is no wall, and the screen
                // stays in front of the wearer.
                val viewer = scene.getViewerPose().t
                val found = mruk.rooms.firstOrNull { it.isPositionInRoom(viewer, true) }
                Log.i(TAG, "room scan: wearer is ${if (found == null) "outside every" else "inside a"} room")
                if (found == null) return@runOnUiThread putAhead()
                room = found
                controls.roomKnown = true
                val place = placement.restore(found) ?: placement.onWall(found, scene.getViewerPose(), SCREEN_WIDTH)
                if (place != null) put(place)
            }
        }
    }

    /**
     * The screen at [place] and at its width, with the controls to its right and the queue to its
     * left.
     *
     * The queue comes out towards the wearer by [NEARER], so a screen on a wall never swallows it.
     * It stays grabbable and stays where the wearer leaves it. The controls belong to the screen,
     * see [placeControls].
     */
    private fun put(place: ScreenPlace) {
        val scale = place.width / SCREEN_WIDTH
        screenEntity?.setComponent(Scale(Vector3(scale)))
        screenEntity?.setComponent(Transform(place.pose))
        // A panel's local negative Z points out of its face, towards whoever is looking at it.
        val queueScale = queueEntity?.tryGetComponent<Scale>()?.scale?.x ?: 1.0f
        val queueHalf = QUEUE_WIDTH * queueScale / 2.0f
        val beside = Vector3(-(place.width / 2.0f + QUEUE_GAP + queueHalf), 0.0f, -NEARER)
        queueEntity?.setComponent(Transform(facingViewer(place.pose.times(Pose(beside)).t)))
        placeControls()
    }

    /**
     * The controls to the right of the screen as it is now, in the screen's own plane.
     *
     * They sit level with the screen rather than in front of it, so they read as part of it.
     */
    private fun placeControls() {
        val pose = screenEntity?.tryGetComponent<Transform>()?.transform ?: return
        val along = SCREEN_WIDTH * screenScale() / 2.0f + CONTROLS_GAP + CONTROLS_WIDTH / 2.0f
        controlsEntity?.setComponent(Transform(pose.times(Pose(Vector3(along, 0.0f, 0.0f)))))
    }

    /**
     * The screen [SCREEN_DISTANCE] in front of the wearer, facing them.
     *
     * Measured from where the wearer is and where they look. A fixed point in the floor space sits
     * wherever the headset last recentred, which can be beside the wearer or behind them.
     */
    private fun putAhead() {
        val viewer = scene.getViewerPose()
        val look = viewer.forward()
        var level = Vector3(look.x, 0.0f, look.z)
        level = if (level.length() < 0.01f) Vector3(0.0f, 0.0f, 1.0f) else level.normalize()
        // Hung like a television: never lower than a standing eye, so a seated wearer looks
        // slightly up at it rather than down. Before tracking starts the viewer is at the origin,
        // on the floor, and this rule covers that too.
        val height = maxOf(viewer.t.y, Placement.EYE_HEIGHT)
        val at = Vector3(viewer.t.x, height, viewer.t.z) + level * SCREEN_DISTANCE
        Log.i(TAG, "screen ahead: viewer=${viewer.t} look=$look screen=$at")
        put(ScreenPlace(facingViewer(at), SCREEN_WIDTH))
    }

    /**
     * A panel at [position], turned about the vertical to face the wearer.
     *
     * The queue stands off to one side of the screen, so the screen's own facing points it past the
     * wearer rather than at them.
     */
    private fun facingViewer(position: Vector3): Pose {
        val toViewer = scene.getViewerPose().t - position
        val level = Vector3(toViewer.x, 0.0f, toViewer.z)
        if (level.length() < 0.01f) return Pose(position)
        // A panel faces along its own negative Z, so its forward points away from the wearer.
        return Pose(position, Quaternion.lookRotationAroundY(-level.normalize()))
    }

    /** Saves where the screen is, against the room. Without a room there is nothing to save to. */
    private fun remember() {
        val found = room ?: return
        val pose = screenEntity?.tryGetComponent<Transform>()?.transform ?: return
        placement.save(found, ScreenPlace(pose, SCREEN_WIDTH * screenScale()))
    }

    private fun screenScale() = screenEntity?.tryGetComponent<Scale>()?.scale?.x ?: 1.0f

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
            }
        }

    private val actions = object : ControlsActions {
        override fun toWall() {
            val found = room ?: return
            val place = placement.onWall(found, scene.getViewerPose(), SCREEN_WIDTH) ?: return
            put(place)
            placement.save(found, place)
        }

        // The button shows only while the machine is idle. A song queued in the moment between
        // the last check and the press refuses the switch, and the next check hides the button.
        override fun toWindow() = Switch.toFlat(this@ImmersiveActivity) {}

        override fun toggleQueue() {
            val shown = !controls.queueShown
            prefs().edit().putBoolean(QUEUE_SHOWN, shown).apply()
            controls.queueShown = shown
            queueEntity?.setComponent(Visible(shown))
        }
    }

    /**
     * A vertical pill of buttons beside the screen: the wall, the queue and the window.
     *
     * The panel is transparent, so only the pill drawn on it shows. That takes three things, and
     * any one missing paints the rest of the panel white: a window theme with no background, a view
     * with no background, and a layer that blends. Android hands the compositor premultiplied alpha,
     * so the blend takes the source as it is.
     */
    private fun controlsPanel(): PanelRegistration {
        val content: (ComposeView) -> Unit = { view ->
            view.setBackgroundColor(android.graphics.Color.TRANSPARENT)
            view.setContent { Controls(controls, actions) }
        }
        return PanelRegistration(R.id.controls_panel) {
            composePanel(content)
            config {
                themeResourceId = R.style.TransparentPanel
                width = CONTROLS_WIDTH
                height = CONTROLS_HEIGHT
                layoutWidthInDp = 120f
                layoutHeightInDp = 480f
                layerConfig = LayerConfig(
                    alphaBlend = LayerAlphaBlend(
                        BlendFactor.ONE,
                        BlendFactor.ONE_MINUS_SOURCE_ALPHA,
                        BlendFactor.ONE,
                        BlendFactor.ONE_MINUS_SOURCE_ALPHA,
                    ),
                )
                enableTransparent = true
                includeGlass = false
            }
        }
    }

    /** The singer's remote, beside the screen, so a wearer needs no phone to queue a song. */
    private fun queuePanel(address: String) =
        PanelRegistration(R.id.queue_panel) {
            view { context -> QueuePanel.view(context, address) }
            config {
                width = QUEUE_WIDTH
                height = QUEUE_HEIGHT
                layoutWidthInDp = 540f
                layoutHeightInDp = 720f
                layerConfig = LayerConfig()
                enableTransparent = false
                includeGlass = false
            }
        }

    private companion object {
        const val PREFS = "headset"
        const val TAG = "KaraokeHeadset"
        const val QUEUE_SHOWN = "queue_shown"

        /** Horizon OS's permission for the room the headset scanned. */
        const val USE_SCENE = "com.oculus.permission.USE_SCENE"
        const val SCENE_REQUEST = 1

        /** Metres. A television's distance, at a television's size. */
        const val SCREEN_DISTANCE = 2.0f
        const val SCREEN_WIDTH = 2.0f

        /** Metres. The widest a hand may stretch the screen. */
        const val WIDEST = 4.0f

        /** Metres. How far the queue stands out in front of the screen. */
        const val NEARER = 0.4f

        /** Metres between the screen's right edge and the controls' panel. */
        const val CONTROLS_GAP = 0.05f

        /** Metres. The controls' panel, at the same 1:4 as its 120x480 dp layout. */
        const val CONTROLS_WIDTH = 0.15f
        const val CONTROLS_HEIGHT = 0.6f

        /** How often the scene asks the machine whether a song is playing. Loopback is cheap. */
        const val WATCH_MS = 2000L

        /**
         * How closely a carried panel follows the hand, where 1 is rigidly. The SDK's 0.15 smooths
         * so much that a panel trails the hand and keeps sliding after the pinch opens.
         */
        const val GRAB_RESPONSIVENESS = 0.5f

        /** Metres between the screen's left edge and the queue's right edge. */
        const val QUEUE_GAP = 0.1f

        /** Metres. The queue's size before a hand stretches it. */
        const val QUEUE_WIDTH = 0.9f
        const val QUEUE_HEIGHT = 1.2f

        /** The smallest and largest a hand may make the queue, as a share of its own size. */
        const val QUEUE_SMALLEST = 0.5f
        const val QUEUE_LARGEST = 2.0f
    }
}
