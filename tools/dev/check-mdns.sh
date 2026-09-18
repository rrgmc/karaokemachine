#!/usr/bin/env bash
#
# One function opens every mDNS daemon, and this is what keeps that true.
#
#   tools/dev/check-mdns.sh                   # exit 1 and name every escape
#
# `mdns_sd::ServiceDaemon::new` binds UDP `0.0.0.0:5353` and `[::]:5353`, which is what Windows
# Firewall answers with a dialog and a rule keyed on the full image path. `km_api::discover::daemon`
# is the only caller, so that `KM_NO_MDNS` means the same thing everywhere; a second caller would be
# a switch honoured in three places out of four, and the symptom is a prompt months later that
# nobody connects to the setting.
#
# The standing decision is `KM_NO_MDNS declines the multicast socket, and one function honours it`
# in docs/decisions/api-and-network.md, and the test half is `No test binds a non-loopback address`
# in CONTRIBUTING.md.
#
# **It matches by shape, so it is a floor rather than a pass.** Lines that begin with `//` are
# skipped, because the rule is discussed in prose in four files and a check that forbade naming it
# would forbid its own rationale -- so a call written after a comment on the same line escapes. It
# cannot see a `#[cfg(test)]` block inside a `src` file either.
#
# `task check` runs it with the other text checks, ahead of fmt, because it reads the tree and says
# one line. CI runs it in the `guards` job.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

self='check-mdns'
owner='crates/machine/km-api/src/discover.rs'
found=0

fail() {
  printf '%s: %s\n' "$self" "$1" >&2
  found=1
}

# Every tracked Rust file, so a crate added outside the workspace is covered too.
files="$(git ls-files '*.rs')"

# -- The constructor, which belongs to one file ---------------------------------------------------

opens=0
while IFS= read -r file; do
  [ -n "$file" ] || continue
  # `grep -v` on a line whose first non-space characters are `//` drops doc comments and ordinary
  # ones together, which is what lets the four files that explain this rule go on explaining it.
  hits="$(grep -n 'ServiceDaemon::new' "$file" 2>/dev/null | grep -v ':[[:space:]]*//' || true)"
  [ -n "$hits" ] || continue
  if [ "$file" = "$owner" ]; then
    opens="$(printf '%s\n' "$hits" | grep -c '' )"
    continue
  fi
  while IFS= read -r hit; do
    fail "$file:${hit%%:*} opens an mDNS daemon; call km_api::discover::daemon instead"
  done <<< "$hits"
done <<< "$files"

if [ "$opens" -eq 0 ]; then
  fail "$owner opens no mDNS daemon; km_api::discover::daemon is where the socket is opened"
elif [ "$opens" -ne 1 ]; then
  fail "$owner opens $opens mDNS daemons; there is one, in km_api::discover::daemon"
fi

# -- ...and the locators a test may not reach for -------------------------------------------------

# The other half of the invariant, and the half that was obeyed wrongly: a browse opens the same
# socket a bind does, so a test that builds a watcher or a real locator raises the same dialog
# without ever naming an address. Tests drive a `Registry` by hand and pass `find::NoLocator`.
while IFS= read -r file; do
  [ -n "$file" ] || continue
  case "$file" in
    */tests/*|tests/*|*/tests.rs|tests.rs|*/testing.rs|testing.rs) ;;
    *) continue ;;
  esac
  hits="$(grep -n -e 'Watcher::start' -e 'Mdns::new' "$file" 2>/dev/null | grep -v ':[[:space:]]*//' || true)"
  [ -n "$hits" ] || continue
  while IFS= read -r hit; do
    fail "$file:${hit%%:*} opens a browse from a test; pass find::NoLocator or drive a Registry"
  done <<< "$hits"
done <<< "$files"

if [ "$found" -ne 0 ]; then
  printf '\n%s\n' "A daemon is opened in one place so that KM_NO_MDNS means one thing --
'KM_NO_MDNS declines the multicast socket, and one function honours it' in
docs/decisions/api-and-network.md." >&2
  exit 1
fi

echo "$self: clean, one daemon and no test browses"
