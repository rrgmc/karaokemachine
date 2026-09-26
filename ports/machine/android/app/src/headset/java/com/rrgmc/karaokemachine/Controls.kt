package com.rrgmc.karaokemachine

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.meta.spatial.uiset.button.PrimaryButton
import com.meta.spatial.uiset.button.SecondaryButton
import com.meta.spatial.uiset.theme.SpatialTheme
import com.meta.spatial.uiset.theme.darkSpatialColorScheme

/** What the controls show, which the scene changes as it learns about the room. */
internal class ControlsState(curved: Boolean, queueShown: Boolean) {
    var curved by mutableStateOf(curved)
    var queueShown by mutableStateOf(queueShown)

    /** Whether a scanned room is loaded, which is what a wall needs. */
    var roomKnown by mutableStateOf(false)

    /** Whether the machine serves a remote to put on the queue panel. */
    var queueAvailable by mutableStateOf(false)

    /** Whether the last switch to a window was refused, because a song or a queue would be lost. */
    var refused by mutableStateOf(false)
}

/** What the controls ask the scene to do. */
internal interface ControlsActions {
    fun shape(curved: Boolean)

    fun toWall()

    fun toWindow()

    fun toggleQueue()
}

/**
 * One row under the screen, in Horizon OS's own UI Set so it reads like the system around it.
 *
 * The choice in force is the primary button and the other is secondary. A button that could do
 * nothing is left out rather than greyed, so the row holds only what works in this room.
 */
@Composable
internal fun Controls(state: ControlsState, actions: ControlsActions) {
    SpatialTheme(darkSpatialColorScheme()) {
        Row(
            modifier = Modifier.fillMaxSize().background(PANEL).padding(12.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterHorizontally),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Choice(stringResource(R.string.headset_screen_flat), !state.curved) {
                actions.shape(false)
            }
            Choice(stringResource(R.string.headset_screen_curved), state.curved) {
                actions.shape(true)
            }
            if (state.roomKnown) {
                SecondaryButton(stringResource(R.string.headset_screen_wall), { actions.toWall() })
            }
            if (state.queueAvailable) {
                Choice(stringResource(R.string.headset_queue), state.queueShown) {
                    actions.toggleQueue()
                }
            }
            SecondaryButton(stringResource(R.string.headset_to_window), { actions.toWindow() })
            if (state.refused) {
                BasicText(
                    stringResource(R.string.headset_switch_refused),
                    style = TextStyle(color = Color.White, fontSize = 16.sp),
                )
            }
        }
    }
}

@Composable
private fun Choice(label: String, chosen: Boolean, onClick: () -> Unit) {
    if (chosen) PrimaryButton(label, onClick) else SecondaryButton(label, onClick)
}

private val PANEL = Color(0xFF101418)
