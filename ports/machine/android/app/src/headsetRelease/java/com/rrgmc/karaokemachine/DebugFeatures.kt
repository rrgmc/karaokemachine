package com.rrgmc.karaokemachine

import com.meta.spatial.core.SpatialFeature
import com.meta.spatial.toolkit.AppSystemActivity

/**
 * Nothing, so a release APK carries no metrics code.
 *
 * The debug build's copy of this file adds the OVR Metrics Tool overlay. Both copies keep one
 * signature, so `ImmersiveActivity` reads the same in either build.
 */
@Suppress("UNUSED_PARAMETER")
internal fun debugFeatures(activity: AppSystemActivity): List<SpatialFeature> = emptyList()
