#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Builds the plugin and wraps it as dist/obs-softphone.plugin (ad-hoc signed),
# plus a zip of it for CI artifacts and releases.
#   scripts/package-macos.sh [--debug]
# Env: OBS_APP (OBS to link against), and the overrides build.rs understands.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE=release
CARGO_FLAGS=(--release)
if [[ "${1:-}" == "--debug" ]]; then
  PROFILE=debug
  CARGO_FLAGS=()
fi

cargo build -p obs-softphone ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"} --manifest-path "$ROOT/Cargo.toml"

VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
ARCH="$(uname -m)"
DIST="$ROOT/dist"
BUNDLE="$DIST/obs-softphone.plugin"
rm -rf "$DIST"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
cp "$ROOT/target/$PROFILE/libobs_softphone.dylib" "$BUNDLE/Contents/MacOS/obs-softphone"

cat > "$BUNDLE/Contents/Info.plist" <<PLIST
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
PLIST

codesign --force --sign - "$BUNDLE" >/dev/null
ZIP="$DIST/obs-softphone-$VERSION-macos-$ARCH.zip"
(cd "$DIST" && ditto -c -k --norsrc --noextattr --keepParent obs-softphone.plugin "$(basename "$ZIP")")
echo "$BUNDLE"
echo "$ZIP"
