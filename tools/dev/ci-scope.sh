#!/usr/bin/env bash
#
# The packages a pull request's build jobs lint and test.
#
#   <changed paths, one a line> | bash tools/dev/ci-scope.sh   # two lines for $GITHUB_OUTPUT
#   bash tools/dev/ci-scope.sh --leaves                        # a leaf and its readers, one a line
#
# A change confined to leaf programs is checked in those programs alone. A leaf is a program under
# `tools/cmd/` that no other package depends on, so nothing else can break when it changes. Any
# other path means the whole workspace, and so does no path at all.
#
# It prints `packages=`, the `-p` arguments for cargo, and `video=`, the video features of those
# packages out of `tools/setup/features.sh`. Both are empty for the whole workspace, where the
# `km-lint` and `km-test` aliases carry their own lists.
#
# **A feature is named only for a package in the scope.** Cargo refuses `--features a/x` unless `a`
# is selected or is a dependency of what is. So the two `testing` features in the aliases stay out:
# each leaf turns on what its own tests need.
#
# `tools/dev/check-ci-paths.sh` asserts that every entry below is still a leaf with those readers. The standing decision
# is `master takes pull requests, and CI is one required check` in docs/decisions/repository.md.

set -uo pipefail

cd "$(dirname "$0")/../.." || exit 2

# A folder under `tools/cmd/` and the package in it carry one name. The first word of an entry is
# the leaf. The words after it are the programs that read a file out of its folder, and a change to
# the leaf checks those too: two programs embed the copy of htmx that `km-package-builder` vendors.
LEAVES=(
  "km-carols"
  "km-lyrics"
  "km-package-builder km-package-simple km-song-sync"
  "km-package-simple"
  "km-site-pack"
  "km-song-sync"
)

if [ "${1:-}" = "--leaves" ]; then
  printf '%s\n' "${LEAVES[@]}"
  exit 0
fi

# shellcheck source=tools/setup/features.sh
. tools/setup/features.sh

whole() {
  echo "packages="
  echo "video="
  exit 0
}

chosen=" "
while IFS= read -r path; do
  path="${path%$'\r'}"
  [ -z "$path" ] && continue
  hit=""
  for entry in "${LEAVES[@]}"; do
    if [[ "$path" == "tools/cmd/${entry%% *}/"* ]]; then
      hit="$entry"
      break
    fi
  done
  [ -z "$hit" ] && whole
  for package in $hit; do
    [[ "$chosen" == *" $package "* ]] || chosen="$chosen$package "
  done
done

[ "$chosen" = " " ] && whole

packages=""
video=""
for leaf in $chosen; do
  packages="$packages -p $leaf"
  IFS=',' read -ra features <<< "$KM_FEATURES_VIDEO"
  for feature in "${features[@]}"; do
    [ "$feature" = "$leaf/video" ] && video="$video,$feature"
  done
done

echo "packages=${packages# }"
echo "video=${video#,}"
