package com.rrgmc.karaokemachine

import android.content.SharedPreferences
import com.meta.spatial.core.Entity
import com.meta.spatial.core.Pose
import com.meta.spatial.core.Quaternion
import com.meta.spatial.core.Vector3
import com.meta.spatial.mruk.MRUKAnchor
import com.meta.spatial.mruk.MRUKPlane
import com.meta.spatial.mruk.MRUKRoom
import com.meta.spatial.toolkit.getAbsoluteTransform
import java.util.UUID

/** Where the screen hangs, and how wide it is. */
internal data class ScreenPlace(val pose: Pose, val width: Float)

/**
 * Where the screen hangs, remembered against the room the headset scanned.
 *
 * **A pose on its own means nothing on the next launch.** The floor-relative space recentres each
 * session, so a saved position would land wherever the wearer happened to face. A wall of the
 * scanned room stays put, so the screen is saved relative to the wall nearest it.
 *
 * Spatial SDK 0.14.0 offers no public persistent spatial anchor. The room's own anchors are the
 * stable frame it does offer.
 *
 * The record lives in the headset's own preferences and never in `settings.json`, for the reason
 * the screen's shape does.
 */
internal class Placement(private val prefs: SharedPreferences) {

    /** The saved place, rebuilt against [room], or null when nothing is saved for this room. */
    fun restore(room: MRUKRoom): ScreenPlace? {
        val id = prefs.getString(ANCHOR, null)?.let(::parseUuid) ?: return null
        val anchor = room.anchors.firstOrNull { it.anchorUuid() == id } ?: return null
        val relative = Pose(
            Vector3(prefs.getFloat(TX, 0f), prefs.getFloat(TY, 0f), prefs.getFloat(TZ, 0f)),
            Quaternion(
                prefs.getFloat(QW, 1f),
                prefs.getFloat(QX, 0f),
                prefs.getFloat(QY, 0f),
                prefs.getFloat(QZ, 0f),
            ),
        )
        val width = prefs.getFloat(WIDTH, 0f)
        if (width <= 0f) return null
        return ScreenPlace(getAbsoluteTransform(anchor).times(relative), width)
    }

    /** Remembers [place] against the wall of [room] nearest to it. */
    fun save(room: MRUKRoom, place: ScreenPlace) {
        val position = place.pose.t
        val wall = room.walls.minByOrNull { getAbsoluteTransform(it).t.distanceTo(position) } ?: return
        val id = wall.anchorUuid() ?: return
        val relative = getAbsoluteTransform(wall).inverse().times(place.pose)
        prefs.edit()
            .putString(ANCHOR, id.toString())
            .putFloat(TX, relative.t.x)
            .putFloat(TY, relative.t.y)
            .putFloat(TZ, relative.t.z)
            .putFloat(QW, relative.q.w)
            .putFloat(QX, relative.q.x)
            .putFloat(QY, relative.q.y)
            .putFloat(QZ, relative.q.z)
            .putFloat(WIDTH, place.width)
            .apply()
    }

    /**
     * The screen flat on the room's main wall, at eye height, facing whoever is looking.
     *
     * Null when the room has no usable wall, and the caller keeps the screen where it is. The
     * wall's normal is flipped towards [viewer] rather than trusted, because the convention for a
     * plane's facing is the one thing here that no measurement has confirmed.
     */
    fun onWall(room: MRUKRoom, viewer: Pose, widest: Float): ScreenPlace? {
        val wall = room.getKeyWall() ?: return null
        val plane = wall.tryGetComponent<MRUKPlane>() ?: return null
        val wallPose = getAbsoluteTransform(wall)

        val wallWidth = plane.max.x - plane.min.x
        val wallHeight = plane.max.y - plane.min.y
        val width = minOf(widest, wallWidth * WALL_SHARE, wallHeight * WALL_SHARE * 16f / 9f)
        if (width < NARROWEST) return null
        val height = width * 9f / 16f

        val centre = wallPose.times(
            Vector3((plane.min.x + plane.max.x) / 2f, (plane.min.y + plane.max.y) / 2f, 0f),
        )
        var normal = wallPose.forward()
        normal = Vector3(normal.x, 0f, normal.z).normalize()
        if (normal.dot(viewer.t - centre) < 0f) normal = -normal

        val bottom = centre.y - wallHeight / 2f + height / 2f
        val top = centre.y + wallHeight / 2f - height / 2f
        val y = EYE_HEIGHT.coerceIn(minOf(bottom, top), maxOf(bottom, top))
        val position = Vector3(centre.x, y, centre.z) + normal * WALL_GAP

        // A panel faces along its own negative Z, so its forward points into the wall.
        return ScreenPlace(Pose(position, Quaternion.lookRotationAroundY(-normal)), width)
    }

    private fun Entity.anchorUuid(): UUID? = tryGetComponent<MRUKAnchor>()?.uuid

    private fun parseUuid(text: String): UUID? =
        try {
            UUID.fromString(text)
        } catch (_: IllegalArgumentException) {
            null
        }

    companion object {
        /** Metres. Where a standing adult's eyes are, measured from the floor. */
        const val EYE_HEIGHT = 1.4f

        /** Metres between the screen and the wall, so the two never fight over a pixel. */
        private const val WALL_GAP = 0.05f

        /** How much of the wall the screen may cover, leaving it a margin. */
        private const val WALL_SHARE = 0.8f

        /** Metres. A wall too narrow for this is a wall the screen does not go on. */
        const val NARROWEST = 1.0f

        private const val ANCHOR = "place_anchor"
        private const val TX = "place_tx"
        private const val TY = "place_ty"
        private const val TZ = "place_tz"
        private const val QW = "place_qw"
        private const val QX = "place_qx"
        private const val QY = "place_qy"
        private const val QZ = "place_qz"
        private const val WIDTH = "place_width"
    }
}
