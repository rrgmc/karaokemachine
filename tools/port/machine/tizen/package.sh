#!/usr/bin/env bash
#
# Builds the karaoke machine for a Samsung Tizen television as an installable .tpk.
#
#   tools/port/machine/tizen/package.sh            # release
#   tools/port/machine/tizen/package.sh --debug    # a debug library, for a backtrace with names
#
# Three steps:
#
#   1. build.sh --app builds libkm_app.so in Docker, without the assets inside it, and stages the
#      asset tree (bank, font, wallpapers) in local/tizen/assets;
#   2. the release's version, from the workspace's Cargo.toml, is written into a copy of
#      ports/machine/tizen/tizen-manifest.xml;
#   3. the .NET SDK with Samsung's Tizen workload builds ports/machine/tizen into a .tpk carrying the
#      library in lib/, the assets in res/assets/ and the icon in shared/res/.
#
# Writes local/tizen/tpk/KmMachine1.KaraokeMachine-<version>.tpk. It is signed with the workload's
# test author certificate; Apps2Samsung signs it again for the television it installs on, which is
# how it gets onto a stock set.
#
# The .NET SDK is `dotnet` on PATH, or $KM_DOTNET, or ~/.dotnet/dotnet (where Microsoft's
# dotnet-install script puts a per-user SDK), whichever first has the `tizen` workload. Installing
# it is Samsung's workload-install script from the Samsung/Tizen.NET repository on GitHub.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

profile=release
build_args=(--release)
case "${1:-}" in
  --debug) profile=debug; build_args=() ;;
  "") ;;
  *) sed -n '3,6p' "$0" >&2; exit 2 ;;
esac

# The SDK, found before the long build so a missing workload costs nothing.
dotnet=""
for candidate in "${KM_DOTNET:-}" dotnet "$HOME/.dotnet/dotnet"; do
  [ -n "$candidate" ] || continue
  command -v "$candidate" >/dev/null 2>&1 || [ -x "$candidate" ] || continue
  if "$candidate" workload list 2>/dev/null | grep -q '^tizen '; then
    dotnet="$candidate"
    break
  fi
done
[ -n "$dotnet" ] || {
  echo "package: no .NET SDK with the tizen workload; set KM_DOTNET or install the workload" >&2
  exit 1
}

host_path() { if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

bash tools/port/machine/tizen/build.sh --app "${build_args[@]}"

version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[ -n "$version" ] || { echo "package: no version in Cargo.toml's [workspace.package]" >&2; exit 1; }

out=local/tizen/tpk
library="local/tizen/target/armv7-unknown-linux-gnueabi/$profile/libkm_app.stripped.so"
[ -f "$library" ] || { echo "package: $library was not built" >&2; exit 1; }
[ -d local/tizen/assets ] || { echo "package: local/tizen/assets was not staged" >&2; exit 1; }

rm -rf "$out"
mkdir -p "$out"
sed "s/version=\"VERSION\"/version=\"$version\"/" ports/machine/tizen/tizen-manifest.xml \
  > "$out/tizen-manifest.xml"

# Built out of tree, so ports/ stays free of bin/ and obj/.
echo "== KmMachine1.KaraokeMachine $version"
"$dotnet" build ports/machine/tizen/KaraokeMachine.csproj -c Release -nologo -v minimal \
  "-p:BaseOutputPath=$(host_path "$PWD/$out/bin")/" \
  "-p:BaseIntermediateOutputPath=$(host_path "$PWD/$out/obj")/" \
  "-p:TizenManifestFile=$(host_path "$PWD/$out/tizen-manifest.xml")" \
  "-p:KmLibrary=$(host_path "$PWD/$library")" \
  "-p:KmAssets=$(host_path "$PWD/local/tizen/assets")" \
  "-p:KmIcon=$(host_path "$PWD/icon/icon-512.png")"

tpk="$(find "$out/bin" -name '*.tpk' | head -1)"
[ -n "$tpk" ] || { echo "package: the build produced no .tpk" >&2; exit 1; }
final="$out/KmMachine1.KaraokeMachine-$version.tpk"
mv "$tpk" "$final"

# What a set will be handed: the three parts this script put in, and their sizes.
echo
echo "built: $final ($(du -h "$final" | cut -f1))"
listing="$(unzip -l "$final")"
for part in lib/libkm_app.so res/assets/soundfont/GeneralUser-GS.sf2 res/assets/fonts/karaoke.ttf \
    shared/res/karaokemachine.png bin/KaraokeMachine.dll tizen-manifest.xml author-signature.xml; do
  if grep -q " $part\$" <<<"$listing"; then
    echo "  $part"
  else
    echo "package: $final has no $part" >&2
    exit 1
  fi
done
