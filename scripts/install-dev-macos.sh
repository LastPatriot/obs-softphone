#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Builds the plugin and installs it for the current user's OBS (dev builds).
#   scripts/install-dev-macos.sh [--release]
# Settings: ~/Library/Application Support/obs-studio/plugin_config/obs-softphone/config.json
# (created with defaults the first time OBS loads the plugin).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE=debug
CARGO_FLAGS=()
if [[ "${1:-}" == "--release" ]]; then
  PROFILE=release
  CARGO_FLAGS=(--release)
fi

cargo build -p obs-softphone ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"} --manifest-path "$ROOT/Cargo.toml"

VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
DEST="$HOME/Library/Application Support/obs-studio/plugins/obs-softphone.plugin"
rm -rf "$DEST"
mkdir -p "$DEST/Contents/MacOS" "$DEST/Contents/Resources"
cp "$ROOT/target/$PROFILE/libobs_softphone.dylib" "$DEST/Contents/MacOS/obs-softphone"

cat > "$DEST/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>obs-softphone</string>
  <key>CFBundleIdentifier</key><string>io.github.lastpatriot.obs-softphone</string>
  <key>CFBundleName</key><string>obs-softphone</string>
  <key>CFBundlePackageType</key><string>BNDL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
</dict>
</plist>
EOF

codesign --force --sign - "$DEST" >/dev/null
echo "Installed $DEST"
echo "Restart OBS, then open Docks → Call-In."
