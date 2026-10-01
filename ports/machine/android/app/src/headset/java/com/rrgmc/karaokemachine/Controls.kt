package com.rrgmc.karaokemachine

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.meta.spatial.uiset.button.ButtonShelf
import com.meta.spatial.uiset.theme.SpatialTheme
import com.meta.spatial.uiset.theme.darkSpatialColorScheme
import com.meta.spatial.uiset.theme.icons.SpatialIcons
import com.meta.spatial.uiset.theme.icons.regular.BulletList
import com.meta.spatial.uiset.theme.icons.regular.Media180
import com.meta.spatial.uiset.theme.icons.regular.OpenPanel
import com.meta.spatial.uiset.theme.icons.regular.Reorient

/** What the controls show, which the scene changes as it learns about the room and the machine. */
internal class ControlsState(curved: Boolean, queueShown: Boolean) {
    var curved by mutableStateOf(curved)
    var queueShown by mutableStateOf(queueShown)

    /** Whether a scanned room is loaded, which is what a wall needs. */
    var roomKnown by mutableStateOf(false)

    /** Whether the machine serves a remote to put on the queue panel. */
    var queueAvailable by mutableStateOf(false)

    /** Whether no song is loaded and the queue is empty, which is when a switch loses nothing. */
    var idle by mutableStateOf(false)
}

/** What the controls ask the scene to do. */
internal interface ControlsActions {
    fun shape(curved: Boolean)

    fun toWall()

    fun toWindow()

    fun toggleQueue()
}

/**
 * A pill under the screen, in Horizon OS's own UI Set so it reads like the system around it.
 *
 * Each button is an icon over a word. A toggle shows its state by being selected. A button that
 * could do nothing is left out rather than greyed, so the pill holds only what works now.
 */
@Composable
internal fun Controls(state: ControlsState, actions: ControlsActions) {
    SpatialTheme(darkSpatialColorScheme()) {
        Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Row(
                modifier = Modifier
                    .background(PILL, RoundedCornerShape(percent = 50))
                    .padding(horizontal = 24.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Shelf(
                    SpatialIcons.Regular.Media180,
                    stringResource(R.string.headset_screen_curved),
                    state.curved,
                ) {
                    actions.shape(!state.curved)
                }
                if (state.roomKnown) {
                    Shelf(
                        SpatialIcons.Regular.Reorient,
                        stringResource(R.string.headset_screen_wall),
                        false,
                    ) {
                        actions.toWall()
                    }
                }
                if (state.queueAvailable) {
                    Shelf(
                        SpatialIcons.Regular.BulletList,
                        stringResource(R.string.headset_queue),
                        state.queueShown,
                    ) {
                        actions.toggleQueue()
                    }
                }
                if (state.idle) {
                    Shelf(
                        SpatialIcons.Regular.OpenPanel,
                        stringResource(R.string.headset_to_window),
                        false,
                    ) {
                        actions.toWindow()
                    }
                }
            }
        }
    }
}

@Composable
private fun Shelf(icon: ImageVector, label: String, selected: Boolean, onClick: () -> Unit) {
    ButtonShelf(
        icon = {
            Image(icon, contentDescription = null, colorFilter = ColorFilter.tint(Color.White))
        },
        label = label,
        selected = selected,
        onSelectionChange = { onClick() },
    )
}

/** Dark and slightly see-through, like the system's own control bar. */
private val PILL = Color(0xE6101418)
