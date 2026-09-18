#!/bin/sh
#
# Waits for a display SDL could actually draw on, then gets out of the way.
#
# Run as ExecStartPre by karaokemachine.service. See the long comment there for the measurements that
# produced it; the short version is that on a cold boot this machine used to start before the graphics
# stack was ready, SDL's kmsdrm backend found no video device, and the appliance came up headless
# after every power cut while reporting itself healthy.
#
# **This is the second version, and the first one's own comment predicted why.** It waited for
# /dev/dri/card0 to *exist* and then slept a flat tenth of a second for udev's ACL, and it said: "A
# tenth of a second is not a fix for a race -- it is the settle the loop above cannot observe, and if
# it is ever not enough the symptom is the identical log line, which is the thing to grep for." On
# 2026-09-01 that is exactly what happened. The node was there on time (`card0 appeared after 300ms`,
# the same 300 ms as the boot that verified the first fix), the logind session on seat0 was active,
# and SDL still reported `kmsdrm not available` -- 69 ms before the kernel logged `fbcon: i915drmfb
# (fb0) is primary device`. The node was never the whole predicate.
#
# **What SDL actually requires**, from `get_driindex` in SDL_kmsdrmvideo.c: it opens each
# /dev/dri/card*, needs non-zero CRTC, connector and encoder counts, and then needs at least one
# connector that is DRM_MODE_CONNECTED *and* has at least one mode. Polling
# /sys/class/drm/card*-*/status and .../modes is the same test read from the same kernel state, from
# a shell. So this waits for the thing itself rather than for a proxy for it.
#
# **The other way `get_driindex` fails looks identical in the log**: open(2) returning EACCES, i.e.
# udev's uaccess ACL not yet applied to the account the service runs as. That is what the old flat
# sleep was guessing at. ExecStartPre runs as User=, so this can simply *ask* -- and it reports which
# of the two preconditions it waited on, so the next occurrence is a measurement rather than another
# round of inference.
#
# **A script rather than an inline `ExecStartPre=/bin/sh -c '...'`.** Two boots were spent on systemd's
# Exec-line quoting: it expands `$WORD` as one of its own environment variables before any shell sees
# the line, so `$i` became empty and the loop's bound collapsed to `[ -lt 100 ]`; and `$$`, the
# documented escape, then has to survive into a shell where `$$` means something else entirely.
# `systemctl show` prints the *stored* argv with the dollars still in it, so the line reads as correct
# in precisely the place you would go to check it. A file has none of these problems and can say why
# it exists, which an Exec line cannot.
#
# Exits 0 whichever way it goes. A machine with no graphics device at all still has an API, a queue
# and a catalog, and waiting forever for a screen that is never coming would be a worse failure than
# the one this fixes -- the same rule the application already applies to every other part of startup.
#
# **And it is now only half of the fix.** No ExecStartPre can help the case this appliance actually
# lives in -- a television switched off at the wall, which drops HDMI hotplug detect, so at boot there
# is genuinely no connected connector and the right thing to do is time out and start. The machine
# itself retries the display for as long as it runs; see `display::run` and the loop around it in
# crates/machine/karaokemachine/src/lib.rs. What this script buys is that the *common* cold boot wins
# on the first attempt, with no black flash and nothing in the journal about a missing screen.

DEVICE="${KM_DRM_DEVICE:-}"                    # optional: restrict to one card's node
DRM_SYSFS="${KM_DRM_SYSFS:-/sys/class/drm}"    # overridable so a test can point at a fake tree
DRI_DIR="${KM_DRM_DRI_DIR:-/dev/dri}"
TIMEOUT_DS="${KM_DRM_TIMEOUT_DS:-100}"         # tenths of a second; 100 = ten seconds

FOUND_CONNECTOR=""
FOUND_CARD=""
FOUND_MODE=""

# The first connector that is connected, has a mode, and whose card node exists.
#
# Not a subshell: the three answers are set as globals because `$(...)` cannot hand back three values
# and the card is needed again afterwards.
find_connector() {
    for status in "$DRM_SYSFS"/card*-*/status; do
        # An unmatched glob stays literal in sh, so this is also what handles a box with no DRM at
        # all -- the `-f` test simply fails on the pattern itself.
        [ -f "$status" ] || continue
        read -r state < "$status" || continue
        [ "$state" = "connected" ] || continue

        # **`[ -s ]` cannot be used here**, and this is the trap worth knowing: every sysfs attribute
        # stats as one page whatever it holds, so `-s` is true for an *empty* modes file -- which is
        # precisely the state this script exists to wait past. Reading it is the only way to tell.
        modes="${status%/status}/modes"
        read -r mode < "$modes" 2>/dev/null || continue
        [ -n "$mode" ] || continue

        connector="${status%/status}"
        connector="${connector##*/}"           # card0-HDMI-A-1
        card="${connector%%-*}"                # card0
        [ -e "$DRI_DIR/$card" ] || continue
        [ -z "$DEVICE" ] || [ "$DRI_DIR/$card" = "$DEVICE" ] || continue

        FOUND_CONNECTOR="$connector"
        FOUND_CARD="$card"
        FOUND_MODE="$mode"
        return 0
    done
    return 1
}

i=0
while ! find_connector; do
    if [ "$i" -ge "$TIMEOUT_DS" ]; then
        echo "wait-for-drm: no connected display in $((TIMEOUT_DS / 10))s; starting anyway" >&2
        exit 0
    fi
    sleep 0.1
    i=$((i + 1))
done

# The second precondition, asked rather than slept for. This does not gate -- a node that is not yet
# openable gets the old flat settle and a line saying so, which is the thing to grep for if the ACL
# turns out to be the real race after all.
if [ -r "$DRI_DIR/$FOUND_CARD" ] && [ -w "$DRI_DIR/$FOUND_CARD" ]; then
    echo "wait-for-drm: $FOUND_CONNECTOR $FOUND_MODE, $DRI_DIR/$FOUND_CARD openable, after $((i * 100))ms" >&2
else
    echo "wait-for-drm: $FOUND_CONNECTOR $FOUND_MODE after $((i * 100))ms, but $DRI_DIR/$FOUND_CARD is not readable and writable by $(id -un); settling" >&2
    sleep 0.1
fi
exit 0
