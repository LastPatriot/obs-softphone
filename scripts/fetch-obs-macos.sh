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
  case "$arch" in
    arm64) flavor=Apple ;;
    x86_64) flavor=Intel ;;
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
