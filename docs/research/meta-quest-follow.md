# Research: a screen that follows the wearer on a Meta Quest

**Findings, and no decision.** The screen hangs in the room, and
[`What the machine *is*, on a headset`](../decisions/distribution.md#what-the-machine-is-on-a-headset)
is what puts it there. A screen that follows the wearer would change that entry. The platform itself
is measured in [`meta-quest.md`](meta-quest.md).

Read on 2026-09-26, from this repository and from the Spatial SDK 0.14.0 archive the `headset`
flavour builds against. **Nothing here ran on a headset.** Every behaviour claim below is a reading
of code, and the marker on each says how far it can be trusted.

**Summary.** The immersive build can make the screen follow the wearer, and passthrough changes
nothing about it. Spatial SDK carries a `Followable` component, and the activity already registers
the system that moves it. So the change is a component on the screen's entity, one target and a few
numbers. It needs no Rust, no new dependency and no manifest entry.

The flat APK cannot do it. Horizon OS owns that window, and the application has no say over where it
sits.

The open questions are about the product rather than the code. A singer turns towards a room, and a
screen that turns with them leaves the place a television would stand. The component's default dead
zone also makes the screen almost head-locked at this distance.

| Marker | Meaning |
|---|---|
| **[repo]** | Read from this repository. High confidence. |
| **[archive]** | Read with `javap` from the `meta-spatial-sdk-toolkit` 0.14.0 archive on Maven Central. High confidence for a signature or a constant. |
| **[bytecode]** | A behaviour read from the order of calls in that archive's bytecode. Medium confidence. |
| **[web]** | Public documentation. Second-hand. |
| **[inferred]** | Reasoning. A hypothesis to test. |

## 1. Where the screen sits

**[repo]** `ImmersiveActivity.onSceneReady` places two panels once. The machine's screen goes 2 m
out at a height of 1.4 m, and the Flat and Curved buttons go under it. Nothing moves either panel
after that.

**[repo]** The scene uses `ReferenceSpace.LOCAL_FLOOR`. The headset's own Reset View therefore
brings the screen round in front of the wearer. That button is the only way the screen moves.

**[repo]** Passthrough is on through `scene.enablePassthrough(true)` and
`scene.enableHolePunching(true)`, with `com.oculus.feature.PASSTHROUGH` in the manifest. The room
shows behind the screen wherever the screen is. A follow mode leaves all three alone.

**[repo]** The flat APK opens as a Horizon OS system panel. The wearer moves and resizes it with the
shell's own controls, and [`meta-quest.md`](meta-quest.md) §2 measured that. The application draws
into the panel and never places it.

## 2. Spatial SDK carries a follow component

**[archive]** `com.meta.spatial.toolkit.Followable` is an entity component with eight fields. A
bare `Followable()` takes these defaults:

| Field | Type | Default |
|---|---|---|
| `target` | `Entity` | `Entity.nullEntity()` |
| `offset` | `Pose` | 3 m ahead, no rotation |
| `minAngle` | `Float` | -90 |
| `maxAngle` | `Float` | 90 |
| `type` | `FollowableType` | `FACE` |
| `tolerance` | `Float` | 0.2 |
| `speed` | `Float` | 1.0 |
| `active` | `Boolean` | `true` |

**[archive]** `FollowableType` has two values, `FACE` and `PIVOT_Y`. `FollowableSystemKt` also
exports `resetFollowable(Entity, Boolean)` and `clampYAngle(Vector3, Float, Float)`.

**[archive]** `AppSystemActivity` registers `ToolkitFeature`, and that feature registers
`FollowableSystem`, `GrabbableSystem` and `GrabbableFollowableSystem`. `ImmersiveActivity` extends
`AppSystemActivity`. So a `Followable` on the screen's entity moves it, and nothing else needs
registering.

**[bytecode]** Each frame the system builds a goal pose from the target's pose and the offset. It
passes the goal through `clampYAngle` with `minAngle` and `maxAngle`. It moves the entity only when
the goal lies further than `tolerance` from where the entity is. It then eases towards the goal with
`Pose.lerp`, at a rate built from a 0.15 constant and `speed`.

**[inferred]** That reading gives each field a meaning. `tolerance` is the dead zone, in metres. The
two angles limit how far up or down the screen goes, and they give no sideways dead zone. `FACE`
turns the screen fully towards the wearer, and `PIVOT_Y` turns it about the vertical axis alone.

**[archive]** `AvatarSystem` names the tracked parts with the strings `head`, `body`,
`left_controller`, `right_controller`, `left_hand` and `right_hand`. An `AvatarAttachment` carries
one of them in its `type` field.

**[inferred]** The target is therefore the entity whose `AvatarAttachment` reads `head`. A query on
`AvatarAttachment` finds it. It may not exist yet when `onSceneReady` runs, and then the target
needs setting on a later frame.

**[archive]** `Grabbable` is the other answer to a screen in the wrong place. Its fields are
`enabled`, `type`, `isGrabbed`, `minHeight` and `maxHeight`, and its defaults are on, `FACE` and no
height limit. The wearer takes hold of the panel and puts it down somewhere else. The screen then
stays fixed, but where the wearer wants it.

## 3. What a follow does to this screen

**[inferred]** The dead zone is a distance, so its angle depends on how far out the screen sits. The
gap between two goal points is `2 × distance × sin(angle / 2)`. At 2 m the default 0.2 m is a head
turn of about 6 degrees.

**[inferred]** A screen that moves after a 6-degree turn is nearly head-locked. A glance at the
keypad, at a microphone or at another person moves it. A dead zone of about 30 degrees needs a
`tolerance` of about 1 m at this distance.

**[web]** Meta's comfort guidance advises against content locked to the head. Text that moves with
every small head movement is hard to read, and some wearers feel sick. The lyrics are the reason the
screen exists.

**[inferred]** The offset has to keep the screen at `SCREEN_DISTANCE`. The curved shape uses that
distance as its cylinder radius. Any other offset puts the wearer off the cylinder's axis, and the
curve then bends unevenly.

**[inferred]** The Flat and Curved buttons have to move with the screen. A `TransformParent` on the
button panel, naming the screen, does that. `FollowableSystem` reads `TransformParent` already, so
the two components combine. The other way is a second `Followable` with its own offset, and two
panels easing separately can drift apart for a moment.

**[inferred]** A moving panel still has to take a ray from a controller or a hand. `IsdkFeature`
probably casts against the panel's pose on each frame, but pointing while the screen eases is
untested.

**[inferred]** A follow costs no passthrough time. The cameras and the depth pipeline run either
way, and moving a compositor layer is a pose change for each frame.

## 4. What a follow does to the product

**[repo]** The decision calls a headset a practice device for one person. It also calls the screen
one that hangs in the room. A follow keeps the first and changes the second.

**[inferred]** A fixed screen stands where a television would stand. The wearer turns to the room
between lines and turns back to the words. A following screen goes with them, and the room is then
behind it.

**[inferred]** A follow suits somebody who moves while singing, such as pacing or dancing. A fixed
screen suits somebody who stands and faces it. So the choice belongs to the wearer, and a switch
that starts off keeps the decided behaviour.

**[repo]** Flat or curved is kept in the headset's own `SharedPreferences`, and `settings.json`
never learns it. The decision gives the reason: the shape is a property of where somebody stands. A
follow mode is the same kind of property, so it would live beside the shape.

**[inferred]** `Followable.active` can change while a song plays. `ImmersiveActivity.reshape` bends
the screen without rebuilding the scene, and a follow switch can work the same way. A third button
beside Flat and Curved would carry it.

## 5. Recommendation

**It is possible, and it is cheap.** One `Followable` on the screen, one `TransformParent` on the
buttons and one saved switch come to a few dozen lines of Kotlin. No Rust changes, and the flat APK
is untouched.

**The version worth trying is a lazy follow.** That means `PIVOT_Y`, the offset at
`SCREEN_DISTANCE`, a `tolerance` near 1 m and the switch off by default. The defaults give a screen
that is almost head-locked, and that works against reading.

**`Grabbable` answers the same complaint with less motion.** It lets the wearer move a fixed screen
by hand. It may be the better first step, because the lyrics never move while somebody reads them.

**A decision entry comes first.** Either mode changes `What the machine *is*, on a headset`, and a
changed decision gets its entry in `docs/decisions/` with the code.

Five measurements on a headset would settle it:

1. The `tolerance` that stops a glance at the keypad from moving the screen.
2. Whether the curved screen stays even while it eases.
3. Whether the lyrics stay sharp while the panel moves.
4. Whether a hand ray keeps hold of a moving panel.
5. When the `head` entity first exists after `onSceneReady`.

## Sources

- `ports/machine/android/app/src/headset/java/com/rrgmc/karaokemachine/ImmersiveActivity.kt` and
  `ports/machine/android/app/src/headset/AndroidManifest.xml` in this repository.
- `ports/machine/android/app/build.gradle`, for the Spatial SDK version the `headset` flavour pins.
- `meta-spatial-sdk-toolkit-0.14.0.aar` from Maven Central, read with `javap -c -p`. The classes
  read were `Followable`, `FollowableType`, `FollowableSystem`, `FollowableSystemKt`, `Grabbable`,
  `AvatarAttachment`, `AvatarSystem`, `ToolkitFeature` and `AppSystemActivity`.
- Meta's comfort and user-interface guidance for Horizon OS, on content locked to the head.
- [`meta-quest.md`](meta-quest.md) and
  [`What the machine *is*, on a headset`](../decisions/distribution.md#what-the-machine-is-on-a-headset).
