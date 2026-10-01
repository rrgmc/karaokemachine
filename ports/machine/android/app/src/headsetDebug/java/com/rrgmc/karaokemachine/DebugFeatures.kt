package com.rrgmc.karaokemachine

import com.meta.spatial.core.SpatialFeature
import com.meta.spatial.ovrmetrics.OVRMetricsFeature
import com.meta.spatial.ovrmetrics.OVRMetricsScene
import com.meta.spatial.ovrmetrics.OVRMetricsTicks
import com.meta.spatial.toolkit.AppSystemActivity

/**
 * The scene's own numbers, on the OVR Metrics Tool overlay, in a debug build only.
 *
 * The tool draws the headset's frame rate, temperatures and clocks itself. This adds the scene's
 * tick rate, its slowest tick and its object count beside them. That pairing is what tells a warm
 * headset slowing the video decoder apart from a scene doing too much. The overlay appears only
 * while the tool is installed and turned on.
 */
internal fun debugFeatures(activity: AppSystemActivity): List<SpatialFeature> =
    listOf(
        OVRMetricsFeature(
            activity,
            OVRMetricsTicks(activity.systemManager),
            OVRMetricsScene({ activity.scene }),
        ),
    )
