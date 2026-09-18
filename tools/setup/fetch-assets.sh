#!/usr/bin/env bash
#
# Fetches the bundled assets that are deliberately not committed, and caches the override banks.
#
#   tools/setup/fetch-assets.sh                    # fetch what is missing
#   tools/setup/fetch-assets.sh --force            # re-fetch even if present
#   tools/setup/fetch-assets.sh --list             # what banks this knows about
#   tools/setup/fetch-assets.sh --bank musescore   # cache an override bank
#   tools/setup/fetch-assets.sh --bank musescore --cache-only   # ...and print only its path
#
# Today the bundled asset means one thing: the General MIDI SoundFont. Without it the machine still
# runs -- the engine falls back to a sine test tone -- but instruments sound wrong, so a fresh clone
# wants this before the app is worth listening to.
#
# Why not commit the file: it is 31 MiB of binary that never changes, which is exactly what git is
# worst at. See the risk note in docs/ARCHITECTURE.md.
#
# Why a machine-local cache: GeneralUser GS's license asks that its download files not be linked
# directly, and suggests keeping your own local copy instead. So this downloads once per machine into
# a cache outside the repo, and every later run -- and every sibling checkout -- copies from there.
# The author's host is touched once, not once per build.
#
# The version is pinned by commit or by checksum, so this is reproducible and a man-in-the-middle or a
# silently re-uploaded file fails loudly rather than landing in the build. **The bank table itself is
# tools/setup/soundfont-banks.sh**, sourced rather than spelled here, because tools/dev/soundfont.sh
# reads the same eleven rows.
#
# ** One bank is installed and the other ten are only cached, and that asymmetry is the whole
# design. ** Everything under `assets/` is shipped: `crates/machine/karaokemachine/Cargo.toml` globs
# `assets/soundfont/*.sf2` into the .deb, and the Windows folder, the macOS bundle, the Linux tarball
# and the APK all copy the tree wholesale. A 206 MiB override left there would silently be added to
# every release.
#
# So the bundled bank is installed into `assets/soundfont/`, and an override is *cached* here and
# then installed into the machine's own SoundFont folder by `--set-soundfont`, which
# `task soundfont BANK=<name>` runs for you. `audio.soundfont` then names it **by bank id**: the
# folder is what says which banks exist, so a bank that stayed only in this cache would be chosen
# and then not found. The cache is where the download lands and is shared between sibling
# checkouts; the machine never reads a bank out of it directly. It is per-machine either way and is
# in no carrier.
#
# **Installing an override into `local/assets/soundfont/gm.sf2` instead is the trap this avoids**:
# the checkout overlay is read only when the machine is run from a checkout, so a bank put there is
# inaudible to the staged build in `dist/bin` and to an installed one -- and, worse, is silently
# beaten by `audio.soundfont` wherever that is set. One door. See the `Switching the SoundFont
# locally` decision in docs/decisions/.
#
# The overlay itself exists and is untouched: it is how a built wallpaper pack reaches a
# `cargo run`, and a `gm.sf2` somebody puts there by hand still wins. Nothing writes one.

set -euo pipefail

cd "$(dirname "$0")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

SF_DEST="assets/soundfont"

# The eleven banks, their sources and what each was measured at. See the header of that file.
. tools/setup/soundfont-banks.sh

BANK="generaluser"
FORCE=0
CACHE_ONLY=0
FONT_ONLY=0

# -- the lyric font ------------------------------------------------------------------------------
#
# **DejaVu Sans, and the same face the Linux tarball stages**, so the two carriers that cannot name a
# font package draw the words identically. `A font, in the tarball only` in docs/decisions/ is the
# entry; this is the second carrier it describes, and the reason there is a download here at all is
# that a Mac has no DejaVu at a fixed path the way the Debian build container does.
#
# **Pinned, and the archive and the member are verified separately.** The second check is what says
# `tar` took the file this pin means, which is the same bargain `cache_member` makes for the one bank
# published inside a zip.
#
# The license travels with it, for the reason the bank's does: the terms are what make shipping it
# legitimate, and `assets/soundfont/LICENSE.txt` shipping a stranger's terms is the fault that taught
# it. DejaVu's is Bitstream Vera plus public-domain additions.
FONT_VER="2.37"
FONT_ARCHIVE="dejavu-fonts-ttf-$FONT_VER.tar.bz2"
FONT_URL="https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/$FONT_ARCHIVE"
FONT_ARCHIVE_DIGEST="fa9ca4d13871dd122f61258a80d01751d603b4d3ee14095d65453b4e846e17d7"
FONT_MEMBER="dejavu-fonts-ttf-$FONT_VER/ttf/DejaVuSans.ttf"
FONT_NAME="DejaVuSans-$FONT_VER.ttf"
FONT_DIGEST="7da195a74c55bef988d0d48f9508bd5d849425c1770dba5d7bfc6ce9ed848954"
FONT_LIC_MEMBER="dejavu-fonts-ttf-$FONT_VER/LICENSE"
FONT_LIC_NAME="DejaVuSans-$FONT_VER-LICENSE.txt"
FONT_LIC_DIGEST="7a083b136e64d064794c3419751e5c7dd10d2f64c108fe5ba161eae5e5958a93"

while [ $# -gt 0 ]; do
  case "$1" in
    --force) FORCE=1 ;;
    --bank) shift; BANK="${1:-}"; [ -n "$BANK" ] || { echo "fetch-assets: --bank needs a name" >&2; exit 2; } ;;
    --bank=*) BANK="${1#--bank=}" ;;
    # Cache the bank, print its path, install nothing. What tools/dev/soundfont.sh calls, so that
    # downloading lives in exactly one script rather than in two that could drift about checksums.
    --cache-only) CACHE_ONLY=1 ;;
    # The lyric font, and nothing else -- no bank is touched and nothing is installed into
    # `assets/`. `tools/port/machine/ios/assets.sh` is the caller; see `fetch_font` below.
    --font) FONT_ONLY=1 ;;
    --list)
      echo "banks:"
      for name in $KM_BANKS; do
        km_bank "$name"
        printf '  %-12s %10s  %-7s  %s\n' "$name" "$SF_SIZE" "$SF_STATUS" "$SF_NOTE"
      done
      cat <<'EOF'

Only `generaluser` is installed into assets/soundfont and shipped. The others are cached and played
by naming them in settings.json, which `task soundfont BANK=<name>` does:

  task soundfont:list             what these are, with the loudness spread each was measured at
  task soundfont BANK=musescore   fetch it and point this machine at it
  task soundfont:clear            back to the bundled bank

A `manual` bank cannot be fetched for you -- ask for it and this says where to get it.
See crates/machine/km-banks/data/soundfont-banks.conf for what all fifteen were measured at,
including the four that do not load at all.
EOF
      exit 0 ;;
    -h|--help)
      sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
      exit 0 ;;
    *) echo "fetch-assets: unknown argument $1 (try --list)" >&2; exit 2 ;;
  esac
  shift
done

# Skipped under `--font`, which touches no bank: resolving one would refuse a run that never wanted
# it, and `SF_*` is unset for the rest of that path.
if [ "$FONT_ONLY" -eq 0 ] && ! km_bank "$BANK"; then
  echo "fetch-assets: no bank called '$BANK' (try --list)" >&2
  exit 2
fi

if [ "$FONT_ONLY" -eq 0 ] && [ "$SF_STATUS" = "manual" ]; then
  echo "fetch-assets: $BANK has no direct download address" >&2
  echo "      Get it from $SF_PAGE ($SF_SIZE)," >&2
  echo "      then: task soundfont FILE=<the file you downloaded>" >&2
  exit 2
fi

# **Everything this prints goes to stderr under --cache-only**, so that the one thing on stdout is
# the path the caller asked for. A caller capturing `$(... --cache-only)` must not also capture
# "fetching  MuseScore_General.sf2".
exec 3>&1
if [ "$CACHE_ONLY" -eq 1 ]; then exec 1>&2; fi

# `shasum` on macOS and the BSDs, `sha256sum`/`sha1sum` on most Linuxes. One set is always there.
if command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
  sha1() { shasum -a 1 "$1" | cut -d' ' -f1; }
elif command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
  sha1() { sha1sum "$1" | cut -d' ' -f1; }
else
  echo "fetch-assets: need shasum or sha256sum to verify the download" >&2
  exit 1
fi

# Does this file match the digest the table gives? A bare value is a sha256; `sha1:` is archive.org's
# own published digest for that item, which is what the six banks hosted there are pinned by -- see
# "Two kinds of digest" in tools/setup/soundfont-banks.sh. An empty spec means "not pinned", and
# **no row is unpinned any more** -- GeneralUser's LICENSE.txt was the one, and being unpinned is
# half of how it came to ship the wrong license. The branch stays because a new row can be added
# before its digest is known, but a file that ships should not use it.
digest_ok() {
  local file="$1" spec="$2"
  local got

  case "$spec" in
    '') return 0 ;;
    sha1:*) got="$(sha1 "$file")"; [ "$got" = "${spec#sha1:}" ] ;;
    sha256:*) got="$(sha256 "$file")"; [ "$got" = "${spec#sha256:}" ] ;;
    *) got="$(sha256 "$file")"; [ "$got" = "$spec" ] ;;
  esac
}

# Respects KM_ASSET_CACHE, then the platform's usual place. Outside the repo either way, so
# `cargo clean` and a fresh clone both leave it alone. Defined once, in tools/setup/asset-cache.sh,
# because tools/dev/clean.sh has to agree with this exactly in order to empty it.
. tools/setup/asset-cache.sh
mkdir -p "$CACHE"
if [ "$FONT_ONLY" -eq 0 ] && [ "$BUNDLED" -eq 1 ]; then
  mkdir -p "$SF_DEST"
fi

# A path somebody can paste into settings.json. Git Bash's `/c/Users/...` is not a path Windows
# resolves -- `PathBuf` would read it as `C:\c\Users\...` -- so ask cygpath for the native spelling.
# `-m` gives forward slashes, which is what JSON wants anyway. Same idiom as tools/platform/linux/check.sh.
host_path() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

# Fetches one file into the cache, verifying it if a digest was given. Downloads to a temporary
# name and moves it into place, so an interrupted run cannot leave a half file looking cached.
cache_file() {
  local name="$1" url="$2" want="$3"
  # A separate `local`: a single one expands all its arguments before assigning any, so a `path` that
  # refers to `name` on the same line would read an unset variable -- fatal under `set -u`.
  local path="$CACHE/$name"

  if [ -f "$path" ] && [ "$FORCE" -eq 0 ]; then
    if digest_ok "$path" "$want"; then
      echo "cached    $name"
      return 0
    fi
    echo "stale     $name (checksum differs; re-fetching)"
  fi

  echo "fetching  $name"
  curl -fL --progress-bar --retry 3 --connect-timeout 20 -o "$path.part" "$url"

  if ! digest_ok "$path.part" "$want"; then
    rm -f "$path.part"
    echo "fetch-assets: $name failed verification" >&2
    echo "  expected $want" >&2
    exit 1
  fi

  mv "$path.part" "$path"
}

# Puts DejaVu Sans and its license in the cache, and prints where they are.
#
# Two lines on stdout, `<font path>` then `<license path>`, because the caller wants both and parsing
# one line for two paths is worse than reading two. Everything else goes to stderr, on
# `--cache-only`'s reasoning: a caller capturing this must not also capture "fetching".
#
# Nothing is installed into `assets/`. That directory is what the *repository* ships and a fetched
# font is not that -- `A font, in the tarball only` says so, and the carrier stages it instead. Here
# the carrier is `tools/port/machine/ios/assets.sh`.
fetch_font() {
  local font="$CACHE/$FONT_NAME" lic="$CACHE/$FONT_LIC_NAME"

  if [ "$FORCE" -eq 0 ] &&
    [ -f "$font" ] && digest_ok "$font" "$FONT_DIGEST" &&
    [ -f "$lic" ] && digest_ok "$lic" "$FONT_LIC_DIGEST"; then
    echo "cached    $FONT_NAME" >&2
  else
    cache_file "$FONT_ARCHIVE" "$FONT_URL" "$FONT_ARCHIVE_DIGEST" >&2

    # `-O` to stdout rather than extracting in place: the member is `ttf/DejaVuSans.ttf` inside a
    # versioned directory and the cache key carries the version instead, so this is a rename as well
    # as an extraction. `-j` would flatten but still write the archive's own name.
    echo "extracting $FONT_MEMBER" >&2
    tar xjf "$CACHE/$FONT_ARCHIVE" -O "$FONT_MEMBER" >"$font.part"
    if ! digest_ok "$font.part" "$FONT_DIGEST"; then
      rm -f "$font.part"
      echo "fetch-assets: $FONT_MEMBER out of $FONT_ARCHIVE failed verification" >&2
      exit 1
    fi
    mv "$font.part" "$font"

    tar xjf "$CACHE/$FONT_ARCHIVE" -O "$FONT_LIC_MEMBER" >"$lic.part"
    if ! digest_ok "$lic.part" "$FONT_LIC_DIGEST"; then
      rm -f "$lic.part"
      echo "fetch-assets: $FONT_LIC_MEMBER out of $FONT_ARCHIVE failed verification" >&2
      exit 1
    fi
    mv "$lic.part" "$lic"
  fi

  host_path "$font" >&3
  echo >&3
  host_path "$lic" >&3
  echo >&3
}

if [ "$FONT_ONLY" -eq 1 ]; then
  fetch_font
  exit 0
fi

# One bank -- FluidR3 -- is published only inside a zip, so fetching it is fetch-then-extract. The
# archive is verified as a whole and the member is verified again on the way out, because the point
# of the second check is that `unzip` picked the file this table means.
cache_member() {
  local path="$CACHE/$SF_NAME"

  if [ -f "$path" ] && [ "$FORCE" -eq 0 ] && digest_ok "$path" "$SF_DIGEST"; then
    echo "cached    $SF_NAME"
    return 0
  fi

  command -v unzip >/dev/null 2>&1 || {
    echo "fetch-assets: $BANK is published only inside a zip and there is no unzip here" >&2
    echo "      Take $SF_MEMBER out of $SF_URL by hand," >&2
    echo "      then: task soundfont FILE=<the file you extracted>" >&2
    exit 1
  }

  cache_file "$SF_ARCHIVE" "$SF_URL" "$SF_ARCHIVE_DIGEST"
  echo "extracting $SF_MEMBER"
  # `-p` to stdout rather than `-j` into the cache: the member is named `FluidR3 GM2-2.SF2` inside
  # the archive and the bank is `FluidR3_GM.sf2` here, so it is being renamed as well as extracted.
  unzip -p "$CACHE/$SF_ARCHIVE" "$SF_MEMBER" >"$path.part"

  if ! digest_ok "$path.part" "$SF_DIGEST"; then
    rm -f "$path.part"
    echo "fetch-assets: $SF_MEMBER out of $SF_ARCHIVE failed verification" >&2
    exit 1
  fi
  mv "$path.part" "$path"
}

if [ -n "$SF_MEMBER" ]; then
  cache_member
else
  cache_file "$SF_NAME" "$SF_URL" "$SF_DIGEST"
fi
# The license travels with the bank, where the bank has one: docs/ARCHITECTURE.md's risk note asks
# that the terms be recorded next to the asset, and the terms are what make bundling it legitimate.
# Most of these eleven ship no license file at all -- §8 of the research note reads their terms out
# of the file's own `ICOP` chunk instead -- so this is conditional rather than assumed.
# **Qualified by the bank, because the cache is shared and `LICENSE.txt` is not a unique name.**
# GeneralUser's license file is called exactly that, so while it was cached under its bare name any
# other file called `LICENSE.txt` already in the cache was accepted as it -- `cache_file` returns
# early for a file that exists, and this row had no digest to catch the substitution. The visible
# result was the bundled bank shipping with a stranger's terms in `assets/soundfont/LICENSE.txt`,
# which is a licensing fault rather than a cosmetic one. It is installed under `LIC_NAME` below; only
# the cache key changes here.
LIC_CACHED=""
if [ -n "$LIC_NAME" ]; then
  LIC_CACHED="$BANK-$LIC_NAME"
  cache_file "$LIC_CACHED" "$LIC_URL" "$LIC_DIGEST"
fi

if [ "$CACHE_ONLY" -eq 1 ]; then
  host_path "$CACHE/$SF_NAME" >&3
  exit 0
fi

if [ "$BUNDLED" -eq 1 ]; then
  # `cached:installed` pairs, because the license's two names differ -- see LIC_CACHED above. The
  # bank's do not, and saying so twice is cheaper than a special case.
  for pair in "$SF_NAME:$SF_NAME" "$LIC_CACHED:$LIC_NAME"; do
    cached="${pair%%:*}"
    name="${pair##*:}"
    [ -n "$name" ] || continue
    if [ "$FORCE" -eq 1 ] || ! cmp -s "$CACHE/$cached" "$SF_DEST/$name"; then
      cp "$CACHE/$cached" "$SF_DEST/$name"
      echo "installed $SF_DEST/$name"
    else
      echo "current   $SF_DEST/$name"
    fi
  done

  echo
  echo "SoundFont ready. The app finds it automatically: $SF_DEST/$SF_NAME is one of the"
  echo "paths km-app looks for, relative to the working directory."
  echo "Cache: $CACHE"
else
  # Cached and nothing else. **Nothing is installed anywhere**, for the reason the header gives:
  # `assets/` is shipped, and the checkout overlay cannot be read by the staged build in dist/bin
  # or by an installed one.
  echo
  echo "Cached, and not installed anywhere -- $SF_SIZE, $SF_LICENSE."
  echo "  $(host_path "$CACHE/$SF_NAME")"
  echo
  echo "To play it, point this machine's settings.json at it:"
  echo
  echo "  task soundfont BANK=$BANK"
  echo
  echo "which writes \`audio.soundfont\` (and checks the bank actually opens first). That reaches"
  echo "every build on this box -- a cargo run, dist/bin, and an installed one alike."
  if [ -n "$SF_VOLUME" ]; then
    echo "It also sets music_volume $SF_VOLUME, which this bank needs: it exceeds full scale at 1.0."
  fi
  echo "Undo: task soundfont:clear"
fi
