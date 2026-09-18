#!/usr/bin/env bash
#
# Nothing committed may describe the machine it was written on.
#
#   tools/dev/check-no-local-refs.sh            # exit 1 and name every offending line
#   tools/dev/check-no-local-refs.sh --list     # ...and print the shapes it looks for, then check
#
# The standing decision is `What a committed file may say about the machine it was written on` in
# docs/decisions/repository.md, and the short form is in CLAUDE.md and CONTRIBUTING.md. This script
# keeps *part* of it true, because a rule with nothing enforcing it rots -- `What the README may show
# of a catalog` said the same thing about pictures, was believed, and was being broken by nine
# files of plain text at the time.
#
# **It enforces three shapes and not the whole rule.** The rule also forbids naming personal hardware
# or a person, and neither has a shape: a hostname, a model number, an email address and a name all
# read as ordinary prose. Two got through for months and were found by reading rather than by running
# this -- the appliance's hostname as test data in km-package-builder, and a stranger's email address
# and telephone number quoted into km-song and a decision. **New prose needs a human pass**, which is
# a convention rather than a check; see CONTRIBUTING.md.
#
# **It matches by shape and never by value, and that is the whole design.** A deny-list naming the
# owner's corpus root, home subnet or account would put those strings into a tracked file in order to
# keep them out of tracked files, which is not a check but a leak with a rationale. So it looks for
# the *form* of a machine-local reference and lets a small allowlist through:
#
#   * a drive-letter path, except the platform's own (C:\Windows, C:\Program Files, C:\Users\Public)
#     and the invented samples this repository standardized on (D:, S:)
#   * a private-range address outside the documented example set -- 192.168.1.x, 192.168.56.1,
#     10.x and 127.x are documentation; anything else inside 192.168/16 or 172.16/12 is a real LAN
#   * a home directory carrying a name: /home/<x>/ or /Users/<x>/ where <x> is not a placeholder
#
# **This file is the one place those shapes may appear**, which is why every pattern below is
# assembled from pieces rather than written out whole: a literal `192.168.68.` in this script would
# be found by this script, and a literal one in a comment explaining the pattern would be worse --
# true, published, and immune to the check that quotes it.
#
# Tracked files only, from `git ls-files`, and binary files are skipped by `grep -I`: a PNG cannot be
# checked this way and the decision covers pictures through `tools/dev/screenshots.sh` instead.
#
# `task check` runs this first, ahead of fmt, because it takes under a second and is the cheapest
# failure in the pass to read. CI runs it first in the `guards` job.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

self='tools/dev/check-no-local-refs.sh'

# A drive-letter path with something after the colon. The allowlist is applied as a second pass
# rather than baked in, because a negative lookahead is not portable to POSIX ERE and a reader has to
# be able to see what is allowed.
#
# `\b` before the letter is doing real work rather than tidiness: without it the `p:/` inside every
# `http:/` matches, as does the `t:\n` inside a printf in minified JavaScript, and the check drowns
# in its own output. A drive letter is one letter with a non-word character in front of it.
#
# The tail is long enough for the allowlist to recognize `C:\Program Files` -- matching one character
# after the slash would hand it `C:\P` and it could not tell Windows from a personal folder.
#
# `…` and `.` are both in the tail class because the elision this repository writes is sometimes one
# and sometimes the other, and if the match stopped short of it the allowlist below would be handed
# `C:/Users/` and could not tell an elided path from a real one.
drive='\b[A-Za-z]:[\\/][A-Za-z0-9_. ()…\\/-]{0,40}'

# Private ranges. `192\.168\.` then anything, and 172.16-31 -- both narrowed afterwards.
lan='192\.168\.[0-9]+\.[0-9]+|172\.(1[6-9]|2[0-9]|3[01])\.[0-9]+\.[0-9]+'

# A home directory with a name in it. `<`, `$`, `.` and the usual placeholders are not names.
home='/(home|Users)/[a-z][a-z0-9_-]*/'

# What may pass. Each is tested against the *matched text*, not against the line, so an allowed
# sample cannot smuggle a disallowed one past on the same line.
#
# Four kinds of drive path are fine, and the third is the one that carries most of the repository's
# existing prose:
#
#   * the platform's own folders -- C:\Windows, C:\Program Files, C:\Users\Public
#   * the invented sample drives this repository standardized on, D: and S:
#   * an **elided** path -- `C:\Users\...`, `C:/…/target`. An ellipsis is the author saying "your
#     path here", which is exactly what this check is asking for, so flagging it would be telling
#     people to fix the thing they already did
#   * an obvious placeholder first segment -- `C:\path\to\song.kar`, `C:/elsewhere/a.kar`, `C:/x/`
#   * a bare drive root, `C:\`, which names no folder at all
allowed_drive='^[Cc]:[\\/](Windows|WINDOWS|Program Files|Program Files \(x86\)|ProgramData|Users[\\/]Public)|^[DdSs]:[\\/]|\.\.\.|…|^[A-Za-z]:[\\/](path|to|elsewhere|x|somewhere|folder|dir)([\\/]|$)|^[A-Za-z]:[\\/]$'
allowed_lan='^192\.168\.1\.[0-9]+$|^192\.168\.56\.[0-9]+$|^172\.(17|28)\.0\.1$|^172\.20\.10\.[0-9]+$'
allowed_home='^/(home|Users)/(you|user|name|me|someone|runner|root|karaoke)/$'

if [ "${1:-}" = "--list" ]; then
  printf 'shapes checked:\n  drive  %s\n  lan    %s\n  home   %s\n\n' "$drive" "$lan" "$home"
fi

found=0

# One pass per shape. `grep -o` gives the matched text so the allowlist can judge it exactly, and
# `-n` keeps the line number for the report; the two are joined by re-running with `-on`.
#
# **The allowlist is matched by the shell, because a process per hit is what this script costs.** The
# three greps read all 736 tracked files in 600ms between them; the hits they return number in the
# hundreds, and a `printf | grep -Eq` pair to judge each one is two process creations apiece on a
# platform where that is the expensive operation. `[[ =~ ]]` is the same POSIX ERE against the same
# pattern, decided in-process.
check() {
  local what="$1" pattern="$2" allow="$3" file line text
  # shellcheck disable=SC2162  # a path cannot contain a newline here and IFS= keeps the spaces
  while IFS= read -r hit; do
    file=${hit%%:*}
    [ "$file" = "$self" ] && continue
    line=${hit#*:}
    text=${line#*:}
    line=${line%%:*}
    if [[ $text =~ $allow ]]; then
      continue
    fi
    printf '%s:%s: %s -- %s\n' "$file" "$line" "$what" "$text" >&2
    found=1
  done < <(git ls-files -z | xargs -0 grep -EIon "$pattern" 2>/dev/null || true)
}

check 'a local path' "$drive" "$allowed_drive"
check 'a private address' "$lan" "$allowed_lan"
check 'a home directory' "$home" "$allowed_home"

if [ "$found" -ne 0 ]; then
  cat >&2 <<'WHY'

No tracked file may name a local drive or folder, a home LAN address, personal hardware or a person.
This script checks the drive path, the private address and the home directory below;
**personal hardware and a person have no shape, so nothing checks them** -- they are a human
pass over new prose.

Use an invented sample -- D:\tunes\karaoke, /tunes/karaoke, 192.168.1.x -- or, where a document has
to stay re-runnable, a variable such as $CORPUS with the real value recorded in CLAUDE.local.md.

See the `What a committed file may say about the machine it was written on` decision in docs/decisions/
for what is deliberately exempt and why.
WHY
  exit 1
fi

echo "check-no-local-refs: clean"
