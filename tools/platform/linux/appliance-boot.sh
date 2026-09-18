#!/usr/bin/env bash
# Makes a deployed box boot like an appliance: no bootloader menu unless somebody presses a key, and
# the machine's own mark on the television instead of kernel text.
#
#   tools/platform/linux/appliance-boot.sh user@box                    # do it
#   tools/platform/linux/appliance-boot.sh user@box --revert           # put the box back
#   tools/platform/linux/appliance-boot.sh user@box --force            # anyway, on a dual-boot box
#   tools/platform/linux/appliance-boot.sh user@box --slim-initramfs   # and make the boot faster
#   tools/platform/linux/appliance-boot.sh user@box --port 2222 --identity ~/.ssh/karaoke
#
# **`--slim-initramfs` is opt-in because its cost is a property of your box, not of this product.**
# It builds the initramfs for the hardware present rather than for all of it. What was measured on
# the appliance is the problem, not the cure: a 72 MB initramfs and a 12 MB kernel read at about
# 12 MB/s, which is 6.76 s of the boot spent in the bootloader. This prints the size before and
# after so the saving is a number you have rather than one this comment claimed.
#
# The price is that such an initramfs will not boot hardware that changes: a new storage controller,
# or the disk moved to another machine, needs a rescue USB. Everything else this script does is
# reversible over ssh; that one is not.
#
# **Run it once per box, after the first deploy.** tools/platform/linux/deploy.sh installs the
# package -- which is where the Plymouth theme comes from -- and this selects it. Nothing here is
# repeated on every code push, and that is deliberate: this rewrites a bootloader's configuration
# and rebuilds an initramfs, and neither belongs in the script you run twenty times an afternoon.
#
# **It will ask for a password, and a deploy does not.** A box locked down to the specific commands
# a deploy runs -- the correct way to set one up -- does not have `update-grub`, `update-initramfs`
# or `plymouth-set-default-theme` on its NOPASSWD list, and it should not: those are not things an
# unattended script should be able to do. This is a once-per-box act with an operator in front of
# it, so it uses `ssh -t` and lets sudo ask.
#
# **Everything it changes, it can put back.** `/etc/default/grub` is copied aside before it is
# touched and the previous Plymouth theme is written down, both exactly once, so a second run cannot
# overwrite the record of what the box looked like before the first. `--revert` restores both.
#
# Requirements here: ssh. There: a box that already has the karaokemachine package on it, and
# **Debian**. Everything this touches except one thing is distribution-neutral -- it probes for the
# bootloader, its config generator, the theme tool and the initramfs lister rather than assuming
# their names, and skips cleanly where a box has no GRUB at all. The exception is installing
# plymouth, which is `apt-get`; on anything else it says so and stops rather than half-configuring a
# stranger's boot. `--slim-initramfs` needs initramfs-tools and says so too.
#
# What it deliberately does not do: choose the timeout. One second, hidden, is a product decision
# -- see `What the box shows before the machine does` in docs/decisions/distribution.md -- and a
# flag here would be a second place to argue it.
set -euo pipefail

cd "$(dirname "$0")/../../.."

HOST=""
PORT=""
IDENTITY=""
MODE=apply
FORCE=0
SLIM=0

while [ $# -gt 0 ]; do
  case "$1" in
    --revert) MODE=revert ;;
    --force) FORCE=1 ;;
    --slim-initramfs) SLIM=1 ;;
    --port) PORT="$2"; shift ;;
    --identity) IDENTITY="$2"; shift ;;
    -h|--help) sed -n '2,36p' "$0" | sed 's/^# \?//'; exit 0 ;;
    -*) echo "appliance-boot: unknown option $1" >&2; exit 2 ;;
    *)
      if [ -n "$HOST" ]; then echo "appliance-boot: more than one host given" >&2; exit 2; fi
      HOST="$1"
      ;;
  esac
  shift
done

if [ -z "$HOST" ]; then
  echo "appliance-boot: usage: tools/platform/linux/appliance-boot.sh [user@]host [--revert]" >&2
  echo "                    [--force] [--slim-initramfs] [--port N] [--identity PATH]" >&2
  exit 2
fi

# The same reasoning as tools/platform/linux/deploy.sh: MSYS rewrites anything that looks like a Unix
# path before handing it over, which would turn the remote `/etc/default/grub` into something under
# `C:/Program Files/Git`. Git for Windows' ssh is itself an MSYS program so in practice it does not
# fire, and saying so out loud costs nothing.
export MSYS2_ARG_CONV_EXCL='*'

SSH_OPTS=()
[ -n "$PORT" ] && SSH_OPTS+=(-p "$PORT")
[ -n "$IDENTITY" ] && SSH_OPTS+=(-i "$IDENTITY")

FILTER=tools/platform/linux/grub-appliance-edit.sh
if [ ! -f "$FILTER" ]; then
  echo "appliance-boot: $FILTER is missing from this checkout" >&2
  exit 1
fi

# Asked before anything is changed, so a wrong hostname costs a second. BatchMode makes it fail
# rather than prompt, which is what distinguishes "key installed" from "will prompt".
if ! ssh "${SSH_OPTS[@]+"${SSH_OPTS[@]}"}" -o BatchMode=yes -o ConnectTimeout=10 \
       "$HOST" true >/dev/null 2>&1; then
  echo "== note"
  echo "  No key-based login to $HOST, so ssh will ask for a password as well as sudo."
  echo "  One-time fix:  ssh-copy-id ${PORT:+-p $PORT }$HOST"
  echo
fi

# The filter travels as an argument rather than as a second file copied over, so the whole job is one
# ssh invocation and therefore one password prompt. It is the same base64 trick the payload itself
# uses and for the same reason: quoting a multi-line shell program through ssh *and* sudo is a
# well-known way to lose a dollar sign.
FILTER_B64=$(base64 < "$FILTER" | tr -d '\n')

REMOTE_SCRIPT=$(cat <<'REMOTE'
set -eu

filter_b64="$1"
mode="$2"
force="$3"
slim="$4"

theme=karaokemachine
theme_dir="/usr/share/plymouth/themes/$theme"
grub_defaults=/etc/default/grub
grub_backup=/etc/default/grub.karaokemachine-orig
theme_backup=/etc/default/plymouth.karaokemachine-orig
quit_dropin=/etc/systemd/system/plymouth-quit.service.d/karaokemachine.conf
quiet_grub=/etc/grub.d/09_karaokemachine
initramfs_conf=/etc/initramfs-tools/conf.d/karaokemachine.conf
wait_dropin=/etc/systemd/system/plymouth-quit-wait.service.d/karaokemachine.conf

# -- what this box is ----------------------------------------------------------------------------
#
# Read rather than assumed, and printed rather than only branched on: which of these is true decides
# what the operator can do at the boot prompt, and they are standing there when this runs.

if [ -d /sys/firmware/efi ]; then firmware=UEFI; else firmware=BIOS; fi

# **`command -v` is the wrong question here, and it cost a whole run.** Every tool this script needs
# lives in /usr/sbin, and a non-root Debian login has PATH=/usr/local/bin:/usr/bin:/bin:/usr/games --
# no sbin on it at all. `sudo` finds them anyway, because sudoers sets its own secure_path with the
# sbin directories in it. So `command -v update-grub` said no about a command `sudo update-grub`
# would have run perfectly: the probe and the invocation were asking two different shells.
#
# Looking for the file is the question that matches what happens next.
find_sbin () {
  for dir in /usr/sbin /sbin /usr/local/sbin /usr/bin /bin /usr/local/bin; do
    if [ -x "$dir/$1" ]; then printf '%s\n' "$dir/$1"; return 0; fi
  done
  return 1
}

# The generated config, wherever this distribution puts it, and whatever it calls the thing that
# regenerates it. `update-grub` is a Debian convenience script; everywhere else it is grub-mkconfig
# or grub2-mkconfig with the output named explicitly.
grub_cfg=""
for candidate in /boot/grub/grub.cfg /boot/grub2/grub.cfg /boot/efi/EFI/debian/grub.cfg; do
  if sudo test -f "$candidate"; then grub_cfg="$candidate"; break; fi
done

grub_regen=$(find_sbin update-grub || find_sbin grub-mkconfig || find_sbin grub2-mkconfig || true)
ply_set=$(find_sbin plymouth-set-default-theme || true)

have_grub=0
[ -f "$grub_defaults" ] && [ -n "$grub_cfg" ] && [ -n "$grub_regen" ] && have_grub=1

echo "== the box"
printf '  firmware:  %s\n' "$firmware"
if [ "$have_grub" -eq 1 ]; then
  printf '  grub:      %s (%s)\n' "$grub_cfg" "$grub_regen"
else
  printf '  grub:      unusable\n'
fi
printf '  theme:     %s\n' "$([ -d "$theme_dir" ] && echo "$theme_dir" || echo '(not installed)')"
echo


# -- revert --------------------------------------------------------------------------------------
#
# First, because it must work even when everything below would refuse. A box that cannot be put back
# is not configured, it is modified.

# `update-grub` takes no arguments and writes where it was built to; the mkconfig pair write to
# stdout unless told otherwise, so handing them the config that was actually found is the difference
# between regenerating it and printing it.
#
# Output is *not* piped anywhere. Indenting it through sed was what let the last failure pass for a
# success, and a legible transcript is not worth a guard that cannot fail.
regenerate () {
  case "$grub_regen" in
    */update-grub) sudo "$grub_regen" ;;
    *) sudo "$grub_regen" -o "$grub_cfg" ;;
  esac
}

if [ "$mode" = revert ]; then
  echo "== revert"
  did=0

  # **Both bootloader undos are collected before a single regenerate**, and the flag is why rather
  # than tidiness: either one alone still needs grub.cfg rebuilt, and a revert that removed a file
  # and left the generated config quoting it would be the half-worked kind of undo this script is
  # most careful about everywhere else.
  regen_wanted=0

  if sudo test -f "$quiet_grub"; then
    sudo rm -f "$quiet_grub"
    echo "  $quiet_grub removed; the loader speaks up again"
    regen_wanted=1
    did=1
  fi

  if sudo test -f "$grub_backup"; then
    sudo cp "$grub_backup" "$grub_defaults"
    sudo rm -f "$grub_backup"
    echo "  $grub_defaults restored"
    regen_wanted=1
    did=1
  else
    echo "  no $grub_backup -- the bootloader was never changed from here"
  fi

  if [ "$regen_wanted" -eq 1 ]; then
    if [ -n "$grub_regen" ]; then
      regenerate
    else
      # Undoing the files is still worth doing where the config cannot be rebuilt: it leaves the box
      # correct for whoever runs the regenerator by hand. Saying so is the point.
      echo "  nothing here can rebuild the boot config -- the hidden menu stays hidden until you"
      echo "  run update-grub yourself"
    fi
  fi

  # Before the theme, whose `-R` rebuilds every initramfs -- so removing this first means the rebuild
  # that follows is also the one that puts MODULES back. If the theme was never changed from here
  # there is nothing to trigger that rebuild, which the branch below handles.
  if sudo test -f "$initramfs_conf"; then
    sudo rm -f "$initramfs_conf"
    echo "  $initramfs_conf removed; the initramfs goes back to carrying every module"
    if sudo test -f "$theme_backup" && [ -n "$ply_set" ]; then
      : # the theme revert below rebuilds it
    else
      update_initramfs=$(find_sbin update-initramfs || true)
      if [ -n "$update_initramfs" ]; then
        sudo "$update_initramfs" -u -k all
      else
        echo "  no update-initramfs here -- rebuild it by hand or the slim one stays in /boot" >&2
      fi
    fi
    did=1
  fi

  if sudo test -f "$theme_backup" && [ -n "$ply_set" ]; then
    was=$(sudo cat "$theme_backup")
    # `none` means Plymouth was not installed when this ran, so there is no previous theme to go
    # back to. `text` is the one that always exists -- it ships in the plymouth package itself
    # rather than in plymouth-themes -- and it is the closest thing to the way the box was.
    [ "$was" = none ] && was=text
    sudo "$ply_set" -R "$was"
    sudo rm -f "$theme_backup"
    echo "  boot splash back to '$was'"
    did=1
  else
    echo "  no $theme_backup -- the splash was never changed from here"
  fi

  # Both drop-ins, then one daemon-reload. Leaving either behind would leave plymouth ordered after
  # a service whose Type=notify this revert has not undone -- and the package's unit is the thing
  # that carries Type=notify, so a revert here plus an older package is exactly the combination that
  # would hold the splash on screen for TimeoutStartSec.
  dropped=0
  for f in "$quit_dropin" "$wait_dropin"; do
    if sudo test -f "$f"; then
      sudo rm -f "$f"
      sudo rmdir "$(dirname "$f")" 2>/dev/null || true
      dropped=1
    fi
  done
  if [ "$dropped" -eq 1 ]; then
    sudo systemctl daemon-reload
    echo "  plymouth-quit back to Debian's own ExecStart and ordering"
    did=1
  fi

  echo
  [ "$did" -eq 1 ] && echo "  Reboot to see it." || echo "  Nothing to undo."
  exit 0
fi

# -- refuse before changing anything ---------------------------------------------------------------

# **A box that has GRUB and cannot regenerate its config stops here, before anything is written.**
# Editing /etc/default/grub without regenerating changes nothing at all: grub.cfg is what boots, and
# the defaults file is only ever an input to producing it.
#
# The first version of this script did exactly that -- wrote the file, could not find update-grub,
# printed one line about it, and went on to report a healthy state block. Two faults in series, and
# both are the house pattern: the probe was `command -v update-grub`, which a non-root PATH answers
# no to about a command sudo would have run; and the report was `regenerate 2>&1 | sed`, whose
# status is sed's, so the failure had nothing for `set -e` to act on. **A step that could fail
# without failing the script**, twice over, in the one place that decides whether a reboot shows
# anything.
#
# After the revert block on purpose: a box that cannot rebuild its config is exactly the box whose
# owner most needs to be able to put it back.
if [ -f "$grub_defaults" ] && [ "$have_grub" -eq 0 ]; then
  echo "appliance-boot: this box has $grub_defaults but the menu cannot be hidden." >&2
  [ -z "$grub_cfg" ] && echo "                No generated config in /boot/grub or /boot/grub2." >&2
  [ -z "$grub_regen" ] && echo "                No update-grub, grub-mkconfig or grub2-mkconfig." >&2
  echo "                Writing the defaults file without regenerating changes nothing that boots," >&2
  echo "                so nothing has been changed at all. Fix the above and run this again." >&2
  exit 1
fi

if [ ! -d "$theme_dir" ]; then
  echo "appliance-boot: $theme_dir is not there, so the karaokemachine package is not installed." >&2
  echo "                Run tools/platform/linux/deploy.sh against this box first -- the theme" >&2
  echo "                ships in the package, and selecting one that is absent gives a boot with" >&2
  echo "                no splash at all." >&2
  exit 1
fi

# **Hiding the menu on a dual-boot box locks the other operating system away.** One second with
# no prompt is a way back for somebody who knows to press a key; it is not a way back for somebody
# who reboots expecting to choose Windows. grub.cfg is root-readable, so nothing outside this script
# can check it on the operator's behalf.
#
# `osprober` is the precise test rather than counting menuentries: os-prober tags every entry it
# generates with an id beginning `osprober-`, where a plain single-OS Debian has a top-level entry
# for itself and often one for the firmware setup, which would make a count say two and refuse for
# no reason.
if [ "$have_grub" -eq 1 ]; then
  # `x=$(grep -c ...) || x=0`, and not `x=$(grep -c ... || echo 0)`. grep -c prints its count and
  # *then* exits 1 when the count is zero, so the inline form yields the two lines "0" and "0" -- and
  # `[ "0\n0" -gt 0 ]` is not false, it is an error, which an `if` swallows. The guard would have
  # passed on every single-OS box for the wrong reason, and nobody would ever have seen it do so.
  others=$(sudo grep -c "osprober" "$grub_cfg" 2>/dev/null) || others=0
  if [ "$others" -gt 0 ] && [ "$force" -ne 1 ]; then
    echo "appliance-boot: $grub_cfg has $others entries for other operating systems on it." >&2
    echo "                Hiding the menu takes them away from anybody who does not know to press" >&2
    echo "                a key as the box starts. Pass --force if that is what you" >&2
    echo "                want." >&2
    exit 1
  fi
  [ "$others" -gt 0 ] && echo "  (--force: hiding a menu that lists $others other operating systems)"
fi

# Not fatal and not ours to fix. `nomodeset` turns kernel mode setting off, which takes away the
# splash *and* the machine's own screen -- SDL's kmsdrm backend has nothing to talk to. Removing a
# word somebody put on their own kernel command line is a bigger liberty than anything this takes,
# so it is said out loud instead.
if grep -q "nomodeset" "$grub_defaults" 2>/dev/null; then
  echo "== WARNING"
  echo "  nomodeset is on this box's kernel command line. With it there is no kernel mode setting,"
  echo "  so there is no splash and no picture from the machine either. Take it out of"
  echo "  $grub_defaults by hand if the television is meant to show anything."
  echo
fi

# -- plymouth ---------------------------------------------------------------------------------------

echo "== plymouth"
# **The one place this script is Debian rather than Linux**, so it says so rather than failing with
# `apt-get: command not found` three lines into changing a stranger's boot. Everything else here
# probes for what it needs -- the bootloader, its regenerator, the theme tool -- because those differ
# between distributions in name only. Installing packages differs in more than the name, and the
# appliance is a Debian deployment by decision (see DEPLOYING.md), so this refuses rather than
# guesses.
if ! find_sbin apt-get >/dev/null; then
  echo "appliance-boot: no apt-get on this box, so plymouth cannot be installed from here." >&2
  echo "                The supported appliance is Debian 13 or newer -- see DEPLOYING.md. On" >&2
  echo "                another distribution, install plymouth and its label plugin by hand and" >&2
  echo "                run this again; everything else it does is distribution-neutral." >&2
  exit 1
fi
# plymouth-label carries the label plugin, which is what the theme's Image.Text needs to draw a
# password prompt. A box with a plain root filesystem never asks for one -- and a box with an
# encrypted root, given a splash that cannot ask, is a black screen that never finishes booting and
# never says why. It costs a few hundred kilobytes to not be that.
sudo apt-get update -qq
sudo apt-get install -y plymouth plymouth-label
echo

# -- write down what was here first ------------------------------------------------------------------
#
# **Exactly once, each of them.** A second run must not overwrite the record of what the box looked
# like before the first -- that is the difference between a backup and a copy of the current state,
# and getting it wrong makes --revert restore the thing it was supposed to undo.

if [ "$have_grub" -eq 1 ] && ! sudo test -f "$grub_backup"; then
  sudo cp "$grub_defaults" "$grub_backup"
  echo "  kept $grub_defaults as $grub_backup"
fi

if ! sudo test -f "$theme_backup"; then
  was=none
  [ -n "$ply_set" ] && was=$("$ply_set" 2>/dev/null || echo none)
  [ -z "$was" ] && was=none
  printf '%s\n' "$was" | sudo tee "$theme_backup" >/dev/null
  echo "  previous splash was '$was'"
fi
echo

# Resolved a second time, and **after** the backup above rather than after the apt install that
# created it. The first resolution is what tells the backup that there was no previous theme,
# because on a box where plymouth has just been installed the tool exists and its answer is
# Debian's post-install default -- which is not what was here before and is not what --revert
# should go back to. So: resolve early to record the past, resolve again to act on the present.
ply_set=$(find_sbin plymouth-set-default-theme)

# -- the bootloader -----------------------------------------------------------------------------

if [ "$have_grub" -eq 1 ]; then
  echo "== bootloader"
  printf '%s' "$filter_b64" | base64 -d > /tmp/km-grub-filter.sh
  # A pipeline through sudo tee rather than a redirect, because the redirect happens in *this*
  # shell, which is not root. Written to a temporary file first so a filter that fails cannot
  # truncate /etc/default/grub on its way out -- `set -e` does not help a `>` that has already
  # opened the file.
  sh /tmp/km-grub-filter.sh < "$grub_defaults" > /tmp/km-grub-new
  rm -f /tmp/km-grub-filter.sh
  if diff -q "$grub_defaults" /tmp/km-grub-new >/dev/null 2>&1; then
    echo "  $grub_defaults already says what it should"
  else
    diff -u "$grub_defaults" /tmp/km-grub-new | sed 's/^/  /' || true
    sudo tee "$grub_defaults" < /tmp/km-grub-new >/dev/null
  fi
  rm -f /tmp/km-grub-new

  # **The loader's own text, which `quiet` does not cover.** GRUB's 10_linux prints "Loading Linux
  # ... " and "Loading initial ramdisk ...", and on a real appliance that text owns the screen for
  # the best part of ten seconds: measured here, the loader spends 6.7s reading a 12 MB kernel and a
  # 72 MB initrd off a SATA disk, and the kernel another 5.9s before i915 modesets over it. `quiet`
  # silences the kernel and has nothing to say about the bootloader, so those two lines are the
  # longest-lived thing on the television during a boot.
  #
  # 10_linux does guard the echoes -- `if [ x"$quiet_boot" = x0 ]` -- and Debian hardcodes
  # `quiet_boot="0"` four lines from the top of the file. **Setting it there is the obvious fix and
  # the wrong one**: 10_linux is a dpkg conffile, so an edited copy earns a conffile prompt at every
  # grub-common upgrade, on a box with no keyboard.
  #
  # /etc/grub.d is a documented extension point instead -- its own README says the number namespace
  # between 10 and 20 is the administrator's -- so this drops a file in that makes the terminal's
  # own colour black on black. The text is still printed and simply cannot be seen.
  #
  # **What that costs, said out loud:** GRUB's interactive command line (`c`) and the one-line hint
  # under the menu both use `color_normal` and go invisible with it. The menu *entries* do not --
  # they use `menu_color_normal`, which 05_debian_theme sets separately -- so the realistic recovery,
  # pressing a key and choosing an older kernel, still works and still looks like something. Anybody
  # who needs the command line can type `set color_normal=white/black` blind, and the file says so.
  printf '%s\n' \
    '#!/bin/sh' \
    '# Written by tools/platform/linux/appliance-boot.sh. Removed by its --revert.' \
    '#' \
    '# Hides the "Loading Linux ..." lines by drawing them black on black. They are printed by' \
    '# 10_linux, which guards them with a quiet_boot flag Debian hardcodes off -- and 10_linux is a' \
    '# conffile, so editing it earns an upgrade prompt on a box with no keyboard.' \
    '#' \
    '# The menu is unaffected: its entries use menu_color_normal. The command line (c) is not, and' \
    '# comes back with:  set color_normal=white/black' \
    'set -e' \
    'echo "set color_normal=black/black"' \
    | sudo tee "$quiet_grub" >/dev/null
  sudo chmod 755 "$quiet_grub"
  echo "  $quiet_grub hides the loader's own text"

  regenerate
  echo
else
  echo "== bootloader"
  echo "  skipped: this box does not boot through GRUB, so there is no menu here to hide."
  echo "  The splash below still applies -- it is the kernel's and the initramfs's, not GRUB's."
  echo "  Whatever does boot this box will need 'splash' on the kernel command line by hand."
  echo
fi

# -- the initramfs, if asked -----------------------------------------------------------------------
#
# **The largest single saving available, and the one with a real edge on it.** Measured on the
# appliance: the loader spends 6.76 s reading a 12 MB kernel and a 72 MB initramfs, which is about
# 12 MB/s -- GRUB's own disk path, not the disk's fault. Debian's `MODULES=most` builds an initramfs
# for hardware the box has not got; `MODULES=dep` builds one for the hardware it has, which here is
# roughly a fifth of the size and takes most of that 6.76 s with it.
#
# **What it costs is written on the tin**: an initramfs built for the hardware present will not boot
# hardware that changes. Move the disk to another box, swap a storage controller, and the next boot
# needs a rescue USB. That is a property of somebody's box and not of this product, which is why it
# is a flag and not the default -- everything else this script does is the appliance decision, and
# this one is the owner's.
#
# A file in conf.d rather than an edit to initramfs.conf, for the reason the GRUB text uses a file in
# /etc/grub.d: initramfs.conf is a dpkg conffile and an edited copy earns a prompt at every upgrade.
# conf.d is read after it and is the documented place for a local override.
#
# No rebuild here. The theme selection below runs `update-initramfs` for every kernel as part of
# `-R`, so writing the file first means one rebuild rather than two.
if [ "$slim" -eq 1 ]; then
  echo "== initramfs"
  if [ ! -d /etc/initramfs-tools ]; then
    echo "  skipped: this box does not build its initramfs with initramfs-tools." >&2
    echo "  dracut and the others have their own way of saying the same thing, and guessing at" >&2
    echo "  one from here would be changing how a stranger's box boots on a hunch." >&2
  else
    was_size=$(sudo stat -c %s "/boot/initrd.img-$(uname -r)" 2>/dev/null || echo 0)
    printf '%s\n' \
      "# Written by tools/platform/linux/appliance-boot.sh --slim-initramfs." \
      "# Removed by its --revert." \
      "#" \
      "# Builds an initramfs for the hardware this box has rather than for all of it, which is" \
      "# most of the time the bootloader spends reading one. It will NOT boot hardware that" \
      "# changes -- a new storage controller, or this disk moved to another machine -- and the way" \
      "# back from that is a rescue USB." \
      "MODULES=dep" \
      | sudo tee "$initramfs_conf" >/dev/null
    echo "  MODULES=dep written to $initramfs_conf"
    printf '  was:       %s bytes (rebuilt below)\n' "$was_size"
  fi
  echo
fi

# -- the splash -----------------------------------------------------------------------------------

echo "== splash"

# **The mark has to outlive plymouth, and outliving it takes two changes rather than one.**
#
# The first attempt was `--retain-splash` alone, which is supposed to leave the last frame on the
# framebuffer when the daemon exits. **On this box it retains nothing**: the screen goes black the
# instant plymouthd exits, whatever the flag says, because the kernel's own console takes the
# framebuffer back and repaints it. Measured twice, once across a four-second gap and once across a
# gap of 1.45 s -- the length made no difference, which is what rules out the tempting explanation
# that something reclaims it after a while.
#
# So the flag is kept and is not what fixes anything. What fixes it is that the gap is now short.
#
# So the ordering is reversed instead. `plymouth-quit.service` is ordered **after** the machine,
# whose unit is `Type=notify` and which sends READY once its audio, catalog and API are up and one
# line before it starts the display. The splash therefore covers the entire load and goes at the
# last possible moment, with `--retain-splash` still there to cover the second and a half that SDL
# takes to put up a first frame.
#
# **Both halves are needed and neither is sufficient**, though each looks like the whole fix on its
# own.
#
# A drop-in under /etc rather than a file in the package, because this edits *Debian's* unit. The
# package ships things and this script decides them, which is the same line the disabled service and
# the unselected theme are drawn on. `ExecStart=` on its own line first is how a drop-in replaces a
# command rather than adding a second one.
ply=$(find_sbin plymouth)
sudo mkdir -p "$(dirname "$quit_dropin")"
printf '%s\n' \
  "# Written by tools/platform/linux/appliance-boot.sh. Removed by its --revert." \
  "[Unit]" \
  "After=karaokemachine.service" \
  "[Service]" \
  "ExecStart=" \
  "ExecStart=-$ply quit --retain-splash" \
  | sudo tee "$quit_dropin" >/dev/null
# The waiter gets the same ordering. Left alone it would sit at multi-user.target holding the boot
# up until plymouth quits -- which is now after the machine has loaded, so it would be waiting on
# something waiting on it to stop waiting. Ordering it behind the machine too makes it wait for the
# thing it was always really waiting for.
sudo mkdir -p "$(dirname "$wait_dropin")"
printf '%s\n' \
  "# Written by tools/platform/linux/appliance-boot.sh. Removed by its --revert." \
  "[Unit]" \
  "After=karaokemachine.service" \
  | sudo tee "$wait_dropin" >/dev/null
sudo systemctl daemon-reload
echo "  plymouth now quits after the machine has loaded, keeping the last frame up"

# -R is the whole point of this line: it rebuilds the initramfs, which is where Plymouth and the
# theme actually live at boot. Without it the theme is selected in /etc and absent from the only
# place that gets read before the root filesystem is mounted.
#
# Not piped through sed either, for the reason regenerate is not: `plymouth-set-default-theme` is a
# shell script that can fail in the middle and keep going, and an indenting pipeline would take its
# exit status away as well as its shape.
sudo "$ply_set" -R "$theme"
echo

# -- what is now true -------------------------------------------------------------------------------
#
# Not what was attempted. Every silent failure this family of scripts has had was a step that could
# fail without failing the script, so each of these is read back off the box rather than echoed from
# what was sent.

echo "== state"
printf '  splash:    %s\n' "$("$ply_set" 2>/dev/null || echo '?')"
# Asked of systemd rather than read out of the file just written, because what matters is that the
# drop-in was parsed and won -- a typo in the section header leaves a perfectly good-looking file
# that changes nothing.
printf '  quit:      %s\n' \
  "$(systemctl show plymouth-quit.service -p ExecStart --value --no-pager | sed -n 's/.*argv\[\]=\([^;]*\).*/\1/p' | head -1)"

# **The check that `-R` really did something.** Selecting a theme writes a line in /etc; drawing one
# needs the theme, its logo and the script plugin inside the initramfs, and those get there by a
# hook that can silently copy less than everything. A theme selected and not present is a black
# screen at the next boot, on a box whose bootloader menu this script has just hidden -- so it is
# worth the two seconds to look inside rather than trust the exit status of the line above.
# `find_sbin` rather than `command -v`, for the reason at the top of this script: this one happens to
# live in /usr/bin and so happened to work, which is exactly how the other two went unnoticed.
lsinitrd=$(find_sbin lsinitramfs || true)
if [ -n "$lsinitrd" ]; then
  initrd="/boot/initrd.img-$(uname -r)"
  # The other half of the promise `--slim-initramfs` makes: the size afterwards, so the saving is a
  # figure off this box rather than an estimate from a comment.
  now_size=$(sudo stat -c %s "$initrd" 2>/dev/null || echo 0)
  printf '  initramfs: %s bytes\n' "$now_size"
  if sudo test -f "$initrd"; then
    inside=$(sudo "$lsinitrd" "$initrd" 2>/dev/null || true)
    for want in themes/karaokemachine/karaokemachine.script themes/karaokemachine/logo.png plymouth/script.so; do
      if printf '%s\n' "$inside" | grep -q "$want"; then
        printf '  in initrd: %s\n' "$want"
      else
        printf '  MISSING from %s: %s\n' "$initrd" "$want"
      fi
    done
  fi
fi
if [ "$have_grub" -eq 1 ]; then
  for key in GRUB_TIMEOUT_STYLE GRUB_TIMEOUT GRUB_CMDLINE_LINUX_DEFAULT GRUB_GFXPAYLOAD_LINUX; do
    # The pipeline's exit status is cut's, which is 0 on no input, so `|| echo '(absent)'` would
    # never fire and a missing key would print as an empty value that reads like a set one.
    found=$(grep "^$key=" "$grub_defaults" | tail -1 | cut -d= -f2-)
    [ -z "$found" ] && found='(absent -- this should not happen)'
    printf '  %-26s %s\n' "$key:" "$found"
  done
fi
printf '  cmdline:   %s\n' "$(cat /proc/cmdline)"
echo "             ^ this is the running kernel's, so it says 'splash' only after a reboot"
REMOTE
)

echo "== remote"
ssh "${SSH_OPTS[@]+"${SSH_OPTS[@]}"}" -t "$HOST" \
  "echo $(printf '%s' "$REMOTE_SCRIPT" | base64 | tr -d '\n') | base64 -d | bash -s -- $FILTER_B64 $MODE $FORCE $SLIM"
echo

if [ "$MODE" = revert ]; then
  echo "== done"
  echo "  reboot:  ssh $HOST 'sudo reboot'"
  exit 0
fi

echo "== done"
echo "  Reboot and watch the television. That is the test nothing here substitutes for."
echo
echo "  reboot:   ssh $HOST 'sudo reboot'"
echo "  undo:     $0 $HOST --revert"
echo
echo "  Nothing is drawn for the first second, on purpose. Hold a key down as the box starts and the"
echo "  bootloader menu comes up -- that is the only way back into it on a UEFI box, so it is worth"
echo "  doing once now to see that it works."
echo
echo "  If the boot is silent and the television stays black past the splash, the splash is holding"
echo "  DRM master and the machine cannot take it:"
echo "    ssh $HOST 'systemd-analyze critical-chain karaokemachine.service'"
echo "  should show plymouth-quit-wait.service ahead of it."
