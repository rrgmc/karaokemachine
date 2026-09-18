# shellcheck shell=bash
#
# Pre-warmed images for the verifiers, so the packages they need are installed once rather than on
# every run.
#
#   . tools/platform/linux/verify-image.sh
#   verify_image_tag    <kind> <base>            # prints the content-addressed tag
#   verify_image_ensure <kind> <base> [--refresh]  # builds it if absent; prints the tag on stdout
#
# `<kind>` is `debian`, `fedora`, `arch` -- a runtime-deps.sh family, for verify-tarball.sh -- or
# `index`, which is verify-deb.sh's much smaller case. `<base>` is the image to derive from, e.g.
# `debian:13-slim`.
#
# Sourced by verify-tarball.sh, verify-deb.sh and prewarm.sh. No shebang and no `set -euo pipefail`,
# for the reason tools/dist/common.sh gives: a sourced file must not change the caller's shell.
#
# **The tag names its own content**, exactly as tools/platform/linux/image-tag.sh does and for the same
# reason: equal inputs share one image, different inputs get different tags and cannot overwrite each
# other. What is hashed here is the *generated Dockerfile text*, which is stronger than hashing the
# inputs separately -- the base ref, the package list and the install command are all covered by
# construction, so there is nothing to forget to add to the hash. Change `--image debian:12-slim` and
# you get a different image automatically; edit a package name in runtime-deps.sh and every stale
# image becomes unreachable rather than silently reused.
#
# **The Dockerfile goes in on stdin** (`docker build -`). There is nothing to COPY, so this needs no
# build context -- which matters here beyond tidiness, because a context is uploaded across the 9p
# bridge on Windows and that is not free. It also leaves nothing behind in tools/platform/linux/.
#
# One thing to *not* flinch at: this repository has been bitten twice by a script arriving on a
# container's stdin and `apt-get` eating the rest of it (see the essays at verify-deb.sh:54 and
# verify-tarball.sh:214). This is not that. That was a *running container's* stdin; this is
# `docker build` reading a Dockerfile, before any container exists, and nothing in the build reads
# stdin at all.
#
# **No package name is ever written in this file.** The whole RUN line is generated from
# runtime-deps.sh. That is deliberate and load-bearing: a prewarmed image is exactly the sort of
# place where somebody later adds "just one more package" to turn a red verification green, and the
# defense against it is that there is nowhere here to put one.

verify_image_dockerfile() { # <kind> <base> -> the generated Dockerfile on stdout
  local kind="$1" base="$2"
  printf 'FROM %s\n' "$base"
  case "$kind" in
    # verify-deb.sh's case: the apt index and *nothing else*. Installing the .deb's Depends closure
    # here would destroy the property that verification exists to check -- see the header of
    # verify-deb.sh. An index is safe because it installs nothing.
    index)
      printf 'RUN apt-get update\n' ;;
    debian|fedora|arch)
      printf 'RUN %s\n' "$(runtime_deps_install_cmd "$kind")" ;;
    *)
      echo "verify-image: unknown kind '$kind'" >&2; return 1 ;;
  esac
}

verify_image_tag() { # <kind> <base> -> karaokemachine-verify-<kind>:<12 hex>
  local kind="$1" base="$2" h
  h="$(verify_image_dockerfile "$kind" "$base" | {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi
  } | cut -c1-12)" || return 1
  printf 'karaokemachine-verify-%s:%s' "$kind" "$h"
}

# Builds the image if it is not already present, and prints the tag. Quiet when there is nothing to
# do, because it is called on the hot path of every verification.
verify_image_ensure() { # <kind> <base> [--refresh] -> tag on stdout, progress on stderr
  local kind="$1" base="$2" refresh="${3:-}" tag
  tag="$(verify_image_tag "$kind" "$base")" || return 1

  if [ "$refresh" = "--refresh" ]; then
    echo "-- rebuilding $tag --" >&2
    docker image rm -f "$tag" >/dev/null 2>&1 || true
    verify_image_dockerfile "$kind" "$base" \
      | docker build --pull --no-cache -t "$tag" - >&2 || return 1
  elif ! docker image inspect "$tag" >/dev/null 2>&1; then
    echo "-- building $tag (once; $base plus the list) --" >&2
    verify_image_dockerfile "$kind" "$base" | docker build -t "$tag" - >&2 || return 1
  fi

  printf '%s' "$tag"
}

# Which family a base image belongs to, or the empty string. **An unrecognized base is not an error
# and must not become one**: `--image` on verify-tarball.sh is described there as the whole point of
# the exercise, so an image this list has never heard of has to keep working -- by falling back to
# the run-time install, exactly as before prewarming existed. This is a fast path, not a whitelist.
verify_image_family() { # <base> -> debian|fedora|arch|""
  case "$1" in
    debian:*|ubuntu:*) printf 'debian' ;;
    fedora:*)          printf 'fedora' ;;
    archlinux:*|arch:*) printf 'arch' ;;
    *)                 printf '' ;;
  esac
}
