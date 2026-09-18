#!/bin/sh
# Turns an ordinary /etc/default/grub into an appliance's: no menu unless somebody asks for one, and
# a boot that shows the machine's splash instead of kernel text.
#
#   tools/platform/linux/grub-appliance-edit.sh < /etc/default/grub > new
#
# **A filter, not an editor**, and that is the whole reason this is a separate file rather than ten
# lines inside tools/platform/linux/appliance-boot.sh. Reading stdin and writing stdout means it can
# be tested against a dozen starting files with no root, no bootloader and no box --
# crates/machine/karaokemachine/tests/grub_appliance_edit.rs does exactly that. It also means the
# caller owns the backup and the write, which is where that responsibility belongs: this is the part
# that has to be *correct*, and the part that has to be *careful* is somewhere else.
#
# It never edits in place and never touches /etc. Nothing here needs privilege.
#
# ## What it sets, and why each one
#
# * **GRUB_TIMEOUT_STYLE=hidden** with **GRUB_TIMEOUT** above zero. Hidden draws no menu at all and
#   waits the timeout for a keystroke; any key brings the menu up. That is the behaviour wanted --
#   an appliance that boots straight through, and a way back in for whoever has to fix it.
#
#   **Zero would be wrong, and the reason is the firmware.** GRUB's `keystatus` -- the test for a
#   held SHIFT that lets a BIOS box show the menu with no timeout at all -- is a BIOS facility and
#   does not exist under EFI. So on a UEFI box, a zero timeout is not "fast", it is "no way into the
#   menu short of a live USB".
#
#   **One second rather than three, and a television is what decided it.** The window is not the
#   only dead time before the splash: the panel drops the HDMI link for about two seconds when i915
#   takes the connector over from the firmware, and the window stacks straight on top of that. Three
#   seconds made five seconds of nothing. One still catches a key held down from the moment the box
#   starts, which is how anybody actually reaches a boot menu, and it is the difference between a
#   boot that looks deliberate and one that looks stuck.
#
# * **`splash` on the kernel command line**, which is what makes Plymouth draw anything. `quiet`
#   alone gives a black screen with no kernel text, which is quieter and is not a splash.
#
# * **`loglevel=3`** so that the handful of messages `quiet` still lets through -- warnings and
#   worse -- do not land on top of the splash.
#
# * **`vt.global_cursor_default=0`**, because otherwise a blinking underscore sits in the corner of
#   the splash from the moment the kernel takes the console. It is the single most "this is a
#   computer" pixel on the screen.
#
# * **`systemd.show_status=false`**, and it was not here at first. `quiet` silences the *kernel*;
#   systemd's own job status is a separate stream that lands on the console the moment plymouth
#   stops drawing. On a real television that was five seconds of
#   `[  OK  ] Finished plymouth-quit-wait.service - Hold until boot process finishes up` where the
#   mark should have been -- the machine takes that long to load its SoundFont and catalog, and the
#   console is what fills the gap. This and `plymouth quit --retain-splash` are the two halves of
#   closing it; neither is enough alone, because the retained frame is still a framebuffer that
#   console text draws straight over.
#
# * **GRUB_GFXPAYLOAD_LINUX=keep**, so the kernel inherits the mode GRUB set instead of the firmware
#   dropping back and the driver picking one again. One fewer mode change between power and picture,
#   and Plymouth starts in the mode it will keep.
#
# ## What it deliberately leaves alone
#
# GRUB_DEFAULT and GRUB_DISTRIBUTOR say which entry and whose name, neither of which is this
# script's business. Debian's `recordfail` handling is untouched on purpose: a boot that failed
# brings the menu back by itself, which is the safety net that makes hiding it defensible in the
# first place.
#
# Every comment, every blank line, every key it does not recognise and the order of all of them
# survive unchanged -- somebody else's `GRUB_CMDLINE_LINUX` or `GRUB_BADRAM` is not ours to reflow.
# A commented-out `#GRUB_TIMEOUT=5` stays commented: it is documentation in every distribution's
# shipped file, and uncommenting one would be this script answering a question nobody asked it.
#
# **Idempotent.** Run twice and the second run is byte-for-byte the first. That matters because the
# caller cannot know whether a box has been through this before, and a filter that appends a little
# more each time is how a kernel command line ends up with `quiet quiet quiet`.
set -eu

TIMEOUT_STYLE=hidden

# Three, and the comment block above argues it. Named here so the test and the decision entry can
# both point at one number rather than at a literal buried in an awk program.
TIMEOUT_SECONDS=1

# Order matters only in that it is the order they are appended in when absent; a word already
# present keeps its position, because reordering somebody's kernel command line is a change they did
# not ask for and cannot see the reason for.
CMDLINE_WORDS="quiet splash loglevel=3 vt.global_cursor_default=0 systemd.show_status=false"

case "${1:-}" in
    -h | --help)
        sed -n '2,/^set -eu$/p' "$0" | sed 's/^# \{0,1\}//; $d'
        exit 0
        ;;
    "") ;;
    *)
        echo "grub-appliance-edit.sh: unexpected argument '$1'; it reads stdin and writes stdout" >&2
        exit 2
        ;;
esac

awk \
    -v timeout_style="$TIMEOUT_STYLE" \
    -v timeout_seconds="$TIMEOUT_SECONDS" \
    -v cmdline_words="$CMDLINE_WORDS" \
    '
    # Merge the wanted words into an existing value.
    #
    # Two kinds of word and they behave differently. A bare flag -- `quiet`, `splash` -- is present
    # or absent. A `key=value` word is a *setting*: if the file already says `loglevel=7`, appending
    # `loglevel=3` would leave the kernel taking the last one, which happens to work and reads as a
    # bug, so the existing token is replaced in place and keeps its position. Anything the file
    # already had that we know nothing about is carried through untouched and in order.
    function merge(existing,   n, i, j, m, tokens, wanted, key, done, out) {
        n = split(existing, tokens, /[ \t]+/)
        m = split(cmdline_words, wanted, /[ \t]+/)

        for (i = 1; i <= n; i++) {
            if (tokens[i] == "") continue
            for (j = 1; j <= m; j++) {
                if (done[j]) continue
                key = wanted[j]
                sub(/=.*$/, "=", key)
                if (tokens[i] == wanted[j] || (key ~ /=$/ && index(tokens[i], key) == 1)) {
                    tokens[i] = wanted[j]
                    done[j] = 1
                    break
                }
            }
            out = (out == "") ? tokens[i] : out " " tokens[i]
        }

        for (j = 1; j <= m; j++)
            if (!done[j]) out = (out == "") ? wanted[j] : out " " wanted[j]

        return out
    }

    # Strip one layer of matching quotes, if there is one. cargo-deb, Debian and everybody else
    # write this value double-quoted; single quotes and no quotes are both legal and both appear in
    # the wild, and the output is always double-quoted because that is what the file it came from
    # will be re-read by.
    function unquote(value) {
        if (value ~ /^".*"$/ || value ~ /^'"'"'.*'"'"'$/)
            return substr(value, 2, length(value) - 2)
        return value
    }

    # A commented line is prose. Nothing below this point sees one.
    /^[ \t]*#/ { print; next }

    /^[ \t]*GRUB_TIMEOUT_STYLE[ \t]*=/ {
        print "GRUB_TIMEOUT_STYLE=" timeout_style
        seen_style = 1
        next
    }

    /^[ \t]*GRUB_TIMEOUT[ \t]*=/ {
        print "GRUB_TIMEOUT=" timeout_seconds
        seen_timeout = 1
        next
    }

    /^[ \t]*GRUB_GFXPAYLOAD_LINUX[ \t]*=/ {
        print "GRUB_GFXPAYLOAD_LINUX=keep"
        seen_payload = 1
        next
    }

    /^[ \t]*GRUB_CMDLINE_LINUX_DEFAULT[ \t]*=/ {
        value = $0
        sub(/^[ \t]*GRUB_CMDLINE_LINUX_DEFAULT[ \t]*=[ \t]*/, "", value)
        print "GRUB_CMDLINE_LINUX_DEFAULT=\"" merge(unquote(value)) "\""
        seen_cmdline = 1
        next
    }

    { print }

    # Whatever the file did not already have. Under a header, so that somebody reading
    # /etc/default/grub in six months can see at a glance which lines were not theirs -- and so that
    # a second run recognises them as ordinary assignments and rewrites them in place rather than
    # adding a second block.
    END {
        if (seen_style && seen_timeout && seen_payload && seen_cmdline) exit
        print ""
        print "# Added by karaokemachine: no menu unless a key is pressed, and the splash instead of"
        print "# kernel text. tools/platform/linux/appliance-boot.sh --revert puts this file back."
        if (!seen_style)   print "GRUB_TIMEOUT_STYLE=" timeout_style
        if (!seen_timeout) print "GRUB_TIMEOUT=" timeout_seconds
        if (!seen_cmdline) print "GRUB_CMDLINE_LINUX_DEFAULT=\"" merge("") "\""
        if (!seen_payload) print "GRUB_GFXPAYLOAD_LINUX=keep"
    }
    '
