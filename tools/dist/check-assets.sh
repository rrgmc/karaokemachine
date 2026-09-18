#!/usr/bin/env bash
#
# Checks what is about to be staged out of `assets/`, before anything copies it.
#
#   tools/dist/check-assets.sh          # refuse the impossible, report the expensive
#
# `assets/` is the shipped tree: `tools/dist/common.sh`'s `dist_stage_assets` copies every file in it
# into the Windows folder, the macOS bundle and the Linux tarball;
# `tools/port/machine/android/assets.sh` copies the same set into the APK;
# `tools/platform/windows/installer.iss` takes it recursively; and cargo-deb takes a whitelist out of
# `crates/machine/karaokemachine/Cargo.toml` -- the `*.sf2` glob, the wallpaper zip and CREDITS.md by
# name. Nothing downstream of any of those asks what it is carrying.
#
# So this asks, once, in the one place all of them can call.
#
# ** It refuses exactly one thing and reports the rest. ** The refusal is two SoundFonts, because
# there is no arrangement in which a release wants both: the .deb's `*.sf2` glob would ship them all,
# and km-app plays whichever of `SOUNDFONT_SUBPATHS` it finds first, so the other is pure weight. An
# override bank belongs in the asset cache, outside the repository -- `tools/setup/fetch-assets.sh
# --bank <name>` puts it there and `task soundfont BANK=<name>` plays it -- where nothing here can see
# it and no carrier can carry it.
#
# Everything else is *reported*, never dropped and never silently allowed. A wallpaper pack in
# `assets/wallpapers/` that is not the shipped set is named as a question rather than approved,
# because its license is something this script cannot check. Same idiom as the `.iss` reconciliation
# in tools/platform/windows/installer.sh and the `.DS_Store` report in
# tools/platform/macos/installer.sh.
#
# See the `Local assets in a checkout` decision in docs/decisions/.

set -euo pipefail

cd "$(dirname "$0")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

# The names km-app will actually load, in preference order. **Duplicated from `SOUNDFONT_SUBPATHS` in
# crates/machine/karaokemachine/src/settings.rs**, which is the source of truth -- a shell script cannot read a Rust
# constant. Renaming one there makes the warning below fire spuriously, which is a visible failure
# rather than a silent one, and is the reason this is a warning and not a refusal.
READABLE_BANKS="gm.sf2 GeneralUser-GS.sf2 FluidR3_GM.sf2"

mib() { # <file> -> its size, rounded, as `12 MiB`
  awk -v b="$(wc -c <"$1" | tr -d ' ')" 'BEGIN { printf "%.0f MiB", b / 1048576 }'
}

banks=()
while IFS= read -r f; do
  [ -n "$f" ] && banks+=("$f")
done < <(find assets/soundfont -maxdepth 1 -type f -name '*.sf2' 2>/dev/null | sort)

if [ "${#banks[@]}" -gt 1 ]; then
  echo "check-assets: ${#banks[@]} SoundFonts in assets/soundfont, and a release may carry only one." >&2
  for f in "${banks[@]}"; do
    echo "  $f   ($(mib "$f"))" >&2
  done
  echo >&2
  echo "  crates/machine/karaokemachine/Cargo.toml globs assets/soundfont/*.sf2 into the .deb, and every other" >&2
  echo "  carrier copies the tree wholesale -- so all of these would ship, and km-app would play" >&2
  echo "  whichever it found first. An override bank is never installed here at all:" >&2
  echo >&2
  echo "      task soundfont:list                            # the eleven it knows about" >&2
  echo "      task soundfont BANK=<name>                     # fetch it and play it" >&2
  echo >&2
  echo "  That caches the bank outside the repository and names it in settings.json, which no" >&2
  echo "  carrier carries -- and unlike this folder it reaches a staged or installed machine too." >&2
  exit 1
fi

if [ "${#banks[@]}" -eq 1 ]; then
  bank="$(basename "${banks[0]}")"
  echo "assets: SoundFont $bank ($(mib "${banks[0]}"))"
  # A bank under any other name ships and is never loaded, which is the quietest way to add tens of
  # megabytes to every carrier for nothing.
  case " $READABLE_BANKS " in
    *" $bank "*) ;;
    *)
      echo "assets: WARNING -- km-app will not load $bank; it looks for $READABLE_BANKS."
      echo "assets:          It would ship in every carrier and never be read."
      ;;
  esac
else
  echo "assets: WARNING -- no SoundFont in assets/soundfont; the machine falls back to a test tone."
  echo "assets:          Run tools/setup/fetch-assets.sh."
fi

# Two kinds of zip live here now and they must not be reported as one thing, because one is the
# machine's own wallpapers and the other is somebody else's photographs.
#
# `default-wallpapers.zip` is the shipped set: seven CC0 photographs, committed, built by
# `km-wallpaper-pack local` and credited in CREDITS.md beside it. It is *meant* to be here.
#
# Anything else is a built pack, here only because somebody passed `--zip-dest ./assets/wallpapers`
# -- the default destination is local/assets/wallpapers. Greeting all of it with "staged
# deliberately" would read as approval for something this script has never checked: a pack built
# from Pixabay or Pexels may not be passed on at all, whoever typed the flag. So it is named as a
# question rather than as a decision. Still a report and not a refusal, because a pack built from
# CC0 or CC BY sources is a legitimate thing to ship, and only its manifest can tell them apart.
while IFS= read -r f; do
  [ -n "$f" ] || continue
  name="$(basename "$f")"
  if [ "$name" = "default-wallpapers.zip" ]; then
    echo "assets: wallpapers $name ($(mib "$f")) -- the shipped CC0 set; see CREDITS.md"
  else
    echo "assets: WARNING -- wallpaper pack $name ($(mib "$f")) is staged into every carrier."
    echo "assets:          Check its manifest says the images may be redistributed: a Pixabay or"
    echo "assets:          Pexels pack is for the machine that built it. Move it to"
    echo "assets:          local/assets/wallpapers/ to keep it out of releases."
  fi
done < <(find assets/wallpapers -maxdepth 1 -type f -name '*.zip' 2>/dev/null | sort)
