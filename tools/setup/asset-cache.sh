# shellcheck shell=bash
#
# Where a machine keeps the large files a build here needs but a checkout does not carry.
#
#   . tools/setup/asset-cache.sh     # from the repository root; sets CACHE
#
# Sourced by tools/setup/fetch-assets.sh, tools/setup/fetch-ffmpeg.sh,
# tools/port/machine/android/ffmpeg.sh and tools/dev/clean.sh. No shebang and no `set -euo pipefail`,
# for the reason tools/dist/common.sh gives: a sourced file must not change the caller's shell.
#
# **Outside the repository, and that is the whole convention** -- so `cargo clean` and a fresh clone
# both leave it alone, and every worktree shares one copy of a 31 MiB SoundFont and an ffmpeg tree
# rather than one each. `KM_ASSET_CACHE` moves it; the platform's usual place is the default.
#
# It lives in a file of its own because the fourth caller was the one that made a copy dangerous
# rather than merely repetitive: three *fetchers* agreeing by accident produce a second download,
# while a *cleaner* that disagreed with them would report having removed a directory it had not
# touched, leaving the real cache in place and the caller believing otherwise. The same bargain
# tools/setup/features.sh and tools/platform/linux/apt-deps.sh make -- one definition, several
# readers.
if [ -n "${KM_ASSET_CACHE:-}" ]; then
  CACHE="$KM_ASSET_CACHE"
elif [ "$(uname -s)" = "Darwin" ]; then
  CACHE="$HOME/Library/Caches/karaokemachine/assets"
else
  CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/karaokemachine/assets"
fi
