#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Downloads official OBS releases and keeps only the frameworks the plugin
# links against, in third_party/obs-app-<arch> (found by the build).
# libobs is single-architecture, so a universal plugin needs both.
#   scripts/fetch-obs-macos.sh [arm64] [x86_64]     (default: the host's)
#   OBS_VERSION=32.2.2
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TP="$ROOT/third_party"
OBS_VERSION="${OBS_VERSION:-32.2.2}"
ARCHS=("$@")
[[ ${#ARCHS[@]} -gt 0 ]] || ARCHS=("$(uname -m)")

for arch in "${ARCHS[@]}"; do
  # SHA-256 of the official 32.2.2 downloads (GitHub release asset digests).
  case "$arch" in
    arm64) flavor=Apple; sha=920d6f26703d2df6e4085bd3c1cbed30488325084136c7a6e9e37021fbd6aaf7 ;;
    x86_64) flavor=Intel; sha=f8d8afe3dffdc86efa0698c02ff0c997866bac3e6208ddaf56d37108baacf197 ;;
    *) echo "unknown arch $arch" >&2; exit 1 ;;
  esac
  DEST="$TP/obs-app-$arch/Contents/Frameworks"
  if [[ -f "$DEST/obs-frontend-api.dylib" ]]; then
    echo "OBS frameworks ($arch): already present"
    continue
  fi
  mkdir -p "$DEST"
  DMG="$TP/obs-$arch.dmg"
  MNT="$TP/obs-mnt-$arch"
  curl -fsSL -o "$DMG" \
    "https://github.com/obsproject/obs-studio/releases/download/$OBS_VERSION/OBS-Studio-$OBS_VERSION-macOS-$flavor.dmg"
  if [[ "$OBS_VERSION" == 32.2.2 ]]; then
    echo "$sha  $DMG" | shasum -a 256 -c - >/dev/null
  fi
  mkdir -p "$MNT"
  hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$MNT" "$DMG" >/dev/null
  SRC="$MNT/OBS.app/Contents/Frameworks"
  for item in libobs.framework QtCore.framework QtGui.framework QtWidgets.framework obs-frontend-api.dylib; do
    ditto "$SRC/$item" "$DEST/$item"
  done
  hdiutil detach "$MNT" -quiet
  rm -rf "$DMG" "$MNT"
  echo "OBS $OBS_VERSION ($flavor) frameworks in $DEST"
done
