#!/usr/bin/env bash
#
# Clears this checkout's own crates out of the shared cargo cache, when the cache was last written by
# a *different* checkout.
#
#   tools/platform/linux/clean-checkout.sh                # from /src, inside the build image
#   tools/platform/linux/clean-checkout.sh --record-only  # record this checkout as the owner, clean nothing
#
# Called by deb-in-container.sh and tarball-in-container.sh before a release build, and by check.sh
# in `--record-only` form. Not meant to be run by hand, and pointless outside the container -- the
# cache it is protecting against is the Docker volume, not target/.
#
# **Why a clean is needed at all.** The `karaokemachine-deb-build` volume is shared by deb.sh,
# tarball.sh and check.sh, and every checkout -- the main one and every git worktree -- bind-mounts
# its own root at the same container path, `/src`. Cargo's fingerprints record that path, so the
# cache genuinely cannot tell two checkouts apart, and will hand a run artifacts compiled from
# different source at the same location. See "The warm cache lies when there is more than one
# checkout" in docs/architecture/distribution.md.
#
# For a *check* that is a thing to recognize: you read the odd baffling compiler error and clean. For
# a **release artifact** it is not. The failure there is not a confusing message but a package or a
# tarball quietly built from another checkout's code, and nothing downstream would catch it -- the
# .deb in particular is what deploy.sh installs on the appliance.
#
# **Why it is now conditional.** It used to run on every release build, and the cost is worse than
# the "about a minute" it used to claim: the clean throws away the *release* compilation of every
# workspace crate, so two `deb.sh` runs in a row recompiled km-audio, km-display, km-remote-pages and
# karaokemachine from scratch both times. That is the whole iteration loop for anyone working on
# packaging, and it was being paid to defend against a situation -- a second checkout -- that most
# runs are not in.
#
# The situation is detectable. What collides is the *host* path of the checkout root, because that is
# precisely what differs between two trees mounted at the same `/src`, and it is the one fact cargo
# is blind to. The drivers pass it in as KM_CHECKOUT (they already compute it for the bind mount),
# and this script keeps the last one in a stamp file at the volume root.
#
# Within one checkout cargo is honest -- content changes and branch switches are fingerprinted
# correctly -- so "same host path as last time" is the entire question, and neither the branch nor a
# content hash would add anything.
#
# **THE INVARIANT, and it is the part to preserve if this is ever edited:**
#
#     The stamp names whoever last wrote to /build/target.
#     Every writer records -- unless recording would erase a mismatch it did not resolve.
#     Only the release paths react to a mismatch.
#
# The second line is why check.sh calls this with `--record-only`. check.sh does not clean and
# deliberately never will, but it *does* write into /build/target from an arbitrary checkout. Without
# it recording, this sequence ships a wrong package:
#
#     deb.sh   from checkout A   -- cleans, stamps A, builds
#     check.sh from checkout B   -- leaves B's artifacts behind; stamp still says A
#     deb.sh   from checkout A   -- stamp says A, so it skips the clean, and ships a package
#                                   built partly from B's source
#
# which is exactly the failure the unconditional clean existed to prevent, reintroduced and quieter.
#
# **The middle line's exception was found by running into it**, and it is the mirror image of that
# sequence rather than a different problem. Recording unconditionally does two jobs, and only one of
# them was intended: it makes a *later* release build clean, and it also **erases a mismatch that is
# still outstanding**, because this form cleans nothing. So:
#
#     deb.sh   from checkout B   -- cleans, stamps B, builds release
#     check.sh from checkout A   -- stamps A; B's release artifacts are untouched, since a check
#                                   writes the debug profile and cargo-deb builds the release one
#     deb.sh   from checkout A   -- stamp says A, so it skips the clean, over B's release tree
#
# A stamp naming somebody else is therefore left alone. What makes that safe to reason about is the
# direction: preserving a mismatch can only ever cause *more* cleaning, never less, and what is being
# defended is a release artifact rather than a build minute.
#
# Only the workspace's own members are cleaned. SDL3, SDL3_ttf and the bundled SQLite -- the
# minutes-long part -- stay cached.
#
# **A race this does not close, deliberately.** Two runs from different checkouts can still interleave
# between the clean and the stamp write. That race exists in the unconditional version too (both
# clean, both build into one target dir) so nothing is made worse here, and the repository's stance on
# concurrent checkouts is that they are settled by naming rather than by locking -- see "Working in
# parallel" in CLAUDE.md.

set -euo pipefail

RECORD_ONLY=0
case "${1:-}" in
  --record-only) RECORD_ONLY=1 ;;
  "") ;;
  *) echo "clean-checkout: unknown option $1" >&2; exit 2 ;;
esac

# The stamp lives at the volume root, not under /build/target: the root is where this repository
# already keeps non-cargo state about the volume (`/build/ffmpeg-lgpl/<id>/.km-complete`), and a
# stamp inside the directory it describes would be destroyed by the very thing it is recording.
# Human-readable rather than hashed, because its second job is telling you *which* other checkout
# took the cache.
STAMP="${KM_STAMP:-/build/.km-checkout}"

# Empty means the caller did not say, which is the pre-KM_CHECKOUT drivers and anyone running this by
# hand. Unknown identity must mean clean -- every uncertain case resolves towards the safe answer,
# because the unsafe one ships a wrong package to the appliance.
me="${KM_CHECKOUT:-}"
last="$(cat "$STAMP" 2>/dev/null || true)"

record() {
  mkdir -p "$(dirname "$STAMP")"
  printf '%s' "$me" > "$STAMP"
}

if [ "$RECORD_ONLY" = "1" ]; then
  # A stamp naming a different checkout is a mismatch this form has not resolved -- it cleans
  # nothing -- so overwriting it would hide it from the release path that does react. See the
  # exception under THE INVARIANT above.
  if [ -n "$last" ] && [ "$last" != "$me" ]; then
    echo "== cache owner left as $last; nothing cleaned"
    echo "   (this run writes the debug profile only, so the release half is still that checkout's"
    echo "    and the next release build here has to clean)"
    exit 0
  fi
  # Write first and do nothing else. check.sh never cleans, so a run of it that dies partway must
  # still leave the stamp naming this checkout -- its artifacts are in the target dir either way, and
  # the next release build has to know that.
  record
  echo "== cache owner recorded (${me:-unknown}); nothing cleaned"
  exit 0
fi

if [ -n "$me" ] && [ "$me" = "$last" ]; then
  echo "== clean skipped: this volume was last written by this checkout"
  echo "   $me"
  exit 0
fi

if [ -n "$last" ]; then
  echo "== clean: the volume was last written by a different checkout"
  echo "   was $last"
  echo "   now ${me:-unknown}"
else
  echo "== clean: no record of who last wrote this volume"
fi

# **Derived rather than listed, and the reason is one crate.** `crates/machine/karaokemachine`'s package is named
# `karaokemachine`, not `km-app`, so a hand-kept list has to remember that the binary crate is spelled
# differently from its directory -- and a list that forgets it silently protects everything except the
# thing being shipped. Reading the name out of each manifest cannot make that mistake, and cannot go
# stale when a crate is added either.
#
# The first `name = "..."` in a manifest is the `[package]` one: `[package]` is the opening section in
# every manifest here, and the keys that could collide (`[[bin]]`, `[dependencies]`) come after it.
crates=()
for manifest in crates/*/*/Cargo.toml; do
  name="$(sed -n 's/^name = "\(.*\)"/\1/p' "$manifest" | head -1)"
  [ -n "$name" ] && crates+=(-p "$name")
done

if [ "${#crates[@]}" -eq 0 ]; then
  echo "clean-checkout: no crates found under crates/ -- is the cwd the repository root?" >&2
  exit 1
fi

# Tolerant of a crate cargo does not know about, which is what the `|| true` in the version this
# replaces was for: `cargo clean -p` on a package absent from the resolved graph is an error, and a
# feature-gated or newly-added crate can be exactly that.
cargo clean "${crates[@]}" 2>/dev/null || true

# Written last, and only after the clean has actually run. If the clean dies partway the stamp still
# names the previous owner, so the next run cleans again -- whereas writing first would leave a stamp
# claiming this checkout owns a target directory that is still contaminated.
record
