#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Downloads an official OBS release and keeps only the frameworks the plugin
# links against, in third_party/obs-app (use OBS_APP=third_party/obs-app).
# For CI and machines without OBS installed.
#   OBS_VERSION=32.2.2 OBS_FLAVOR=Apple|Intel scripts/fetch-obs-macos.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TP="$ROOT/third_party"
OBS_VERSION="${OBS_VERSION:-32.2.2}"
OBS_FLAVOR="${OBS_FLAVOR:-Apple}"
DEST="$TP/obs-app/Contents/Frameworks"

if [[ -f "$DEST/obs-frontend-api.dylib" ]]; then
  echo "OBS frameworks: already present"
  exit 0
fi

mkdir -p "$TP" "$DEST"
DMG="$TP/obs.dmg"
MNT="$TP/obs-mnt"
curl -fsSL -o "$DMG" \
  "https://github.com/obsproject/obs-studio/releases/download/$OBS_VERSION/OBS-Studio-$OBS_VERSION-macOS-$OBS_FLAVOR.dmg"
mkdir -p "$MNT"
hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$MNT" "$DMG" >/dev/null
trap 'hdiutil detach "$MNT" -quiet || true; rm -f "$DMG"' EXIT
SRC="$MNT/OBS.app/Contents/Frameworks"
for item in libobs.framework QtCore.framework QtGui.framework QtWidgets.framework obs-frontend-api.dylib; do
  ditto "$SRC/$item" "$DEST/$item"
done
echo "OBS $OBS_VERSION ($OBS_FLAVOR) frameworks in $DEST"
