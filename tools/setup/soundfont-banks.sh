# shellcheck shell=bash
#
# Reads the General MIDI bank table, which lives in a data file rather than in here.
#
#   . tools/setup/soundfont-banks.sh    # sets KM_BANKS and km_bank()
#
# Sourced, never executed -- so no shebang and no `set -euo pipefail`, the same rule
# tools/setup/features.sh, tools/setup/asset-cache.sh and tools/dist/common.sh follow: a sourced file
# that sets shell options changes the caller's shell in ways the caller did not ask for.
#
# **The rows live in `crates/machine/km-banks/data/soundfont-banks.conf` and this is a reader.** A
# `case` here would serve the four shell readers -- `tools/setup/fetch-assets.sh` downloads the
# bundled bank, `tools/dev/soundfont.sh` points a machine at any of them,
# `tools/dev/soundfont-debug.sh` fills the switcher's slots and `tools/dev/soundfont-measure.sh`
# measures them -- and not the fifth: the machine offers the same list from the remote and is written
# in Rust, so the table has to be something both languages read. That file's own header has the
# format, the fields and the reasoning.
#
# The four callers above are untouched: `km_bank <name>` still fills in the same `SF_*`, `LIC_*` and
# `BUNDLED` variables, and `KM_BANKS` is still the list of names in listing order.

# Where the rows are, resolved from this file rather than from the caller's working directory --
# `fetch-assets.sh` runs from the repository root and `soundfont-measure.sh` cds to it, but nothing
# here should depend on that.
KM_BANKS_FILE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/crates/machine/km-banks/data/soundfont-banks.conf"

# The names, in the order the file lists them. Derived rather than written down a second time: the
# order is a property of the table and a hand-kept copy of it is the drift this arrangement exists to
# prevent.
KM_BANKS="$(sed -n 's/^\[\(.*\)\]$/\1/p' "$KM_BANKS_FILE" | tr '\n' ' ')"
KM_BANKS="${KM_BANKS% }"

# Fills in the SF_*, LIC_* and BUNDLED variables for one bank. Returns 2 for a name it does not know,
# so a caller can tell "no such bank" from a download that failed.
km_bank() {
  # Cleared first and every time, so a second call cannot leave a previous bank's license file or
  # archive member attached to a row that has none of its own.
  SF_STATUS=''; SF_NAME=''; SF_URL=''; SF_DIGEST=''; SF_BYTES=''; SF_SIZE=''
  SF_ARCHIVE=''; SF_ARCHIVE_DIGEST=''; SF_MEMBER=''
  SF_PAGE=''; SF_VOLUME=''; SF_SPREAD=''; SF_LICENSE=''; SF_NOTE=''; SF_RANK=''; SF_RECOMMENDED=''
  LIC_NAME=''; LIC_URL=''; LIC_DIGEST=''
  BUNDLED=0

  local want="$1" in_block=0 found=0 line key value id

  while IFS= read -r line || [ -n "$line" ]; do
    # **A trailing CR is stripped here, and this line has a history.** `.gitattributes` pins this
    # file to LF, which is the door that matters -- but a checkout made before that pin existed keeps
    # its CRLF until the file is re-materialised, and `read -r` is the one reader that carries the CR
    # through. `sed` above does not: Git Bash reads in text mode, so `KM_BANKS` was always clean and
    # only this loop failed, leaving `${line%]}` holding `generaluser<CR>` and every lookup returning
    # 2. The symptom was a machine that could not find the bundled bank to download.
    #
    # The Rust reader has always called `trim_end()`. Two readers of one file ought to agree about a
    # stray CR, so this one does now too.
    line="${line%$'\r'}"
    case "$line" in
      '#'* | '') continue ;;
      '['*']')
        id="${line#[}"
        id="${id%]}"
        if [ "$id" = "$want" ]; then
          in_block=1
          found=1
        else
          # Not `break` on leaving the wanted block: a caller asking for a name that is not there
          # must read to the end to find that out, and the file is a few hundred lines.
          in_block=0
        fi
        continue
        ;;
    esac
    [ "$in_block" = 1 ] || continue

    # Split on the first run of whitespace. A value runs to the end of the line and may hold spaces,
    # quotes and `#` -- `license` and `note` both carry the first two -- so nothing here strips or
    # unquotes anything.
    key="${line%%[[:space:]]*}"
    value="${line#"$key"}"
    value="${value#"${value%%[![:space:]]*}"}"

    case "$key" in
      status) SF_STATUS="$value" ;;
      name) SF_NAME="$value" ;;
      url) SF_URL="$value" ;;
      digest) SF_DIGEST="$value" ;;
      bytes) SF_BYTES="$value" ;;
      size) SF_SIZE="$value" ;;
      archive) SF_ARCHIVE="$value" ;;
      archive_digest) SF_ARCHIVE_DIGEST="$value" ;;
      member) SF_MEMBER="$value" ;;
      page) SF_PAGE="$value" ;;
      volume) SF_VOLUME="$value" ;;
      spread) SF_SPREAD="$value" ;;
      license) SF_LICENSE="$value" ;;
      note) SF_NOTE="$value" ;;
      lic_name) LIC_NAME="$value" ;;
      lic_url) LIC_URL="$value" ;;
      lic_digest) LIC_DIGEST="$value" ;;
      rank) SF_RANK="$value" ;;
      recommended) SF_RECOMMENDED="$value" ;;
      bundled) BUNDLED="$value" ;;
      # A key the table grew and this reader does not know yet. Ignored rather than fatal: the Rust
      # reader is the one that would need it, and failing every shell command over a field they do
      # not use would be the wrong way round.
      *) ;;
    esac
  done < "$KM_BANKS_FILE"

  [ "$found" = 1 ] || return 2
}
