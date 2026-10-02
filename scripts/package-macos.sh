#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Builds the plugin and packages it in dist/:
#   obs-softphone.plugin                        (ad-hoc signed bundle)
#   obs-softphone-<ver>-macos-<arch>.zip        (the bundle, zipped)
#   obs-softphone-<ver>-macos-<arch>.pkg        (installer; not with --debug)
# ARCHS="arm64 x86_64" builds a universal plugin (default: the host's arch).
# Each arch links against third_party/obs-app-<arch> (scripts/fetch-obs-macos.sh)
# or the installed OBS; see crates/plugin/build.rs.
#   scripts/package-macos.sh [--debug]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARCHS="${ARCHS:-$(uname -m)}"
PROFILE=release
CARGO_FLAGS=(--release)
if [[ "${1:-}" == "--debug" ]]; then
  PROFILE=debug
  CARGO_FLAGS=()
fi

triple() {
  case "$1" in
    arm64) echo "aarch64-apple-darwin" ;;
    x86_64) echo "x86_64-apple-darwin" ;;
    *) echo "unknown arch $1" >&2; exit 1 ;;
  esac
}

SLICES=()
for arch in $ARCHS; do
  t="$(triple "$arch")"
  cargo build -p obs-softphone --target "$t" ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"} --manifest-path "$ROOT/Cargo.toml"
  SLICES+=("$ROOT/target/$t/$PROFILE/libobs_softphone.dylib")
done

VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
read -r -a ARCH_LIST <<< "$ARCHS"
LABEL="$([[ ${#ARCH_LIST[@]} -gt 1 ]] && echo universal || echo "${ARCH_LIST[0]}")"
DIST="$ROOT/dist"
BUNDLE="$DIST/obs-softphone.plugin"
NAME="obs-softphone-$VERSION-macos-$LABEL"

rm -rf "$DIST"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
lipo -create "${SLICES[@]}" -output "$BUNDLE/Contents/MacOS/obs-softphone"

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
(cd "$DIST" && ditto -c -k --norsrc --noextattr --keepParent obs-softphone.plugin "$NAME.zip")
echo "$BUNDLE"
echo "$DIST/$NAME.zip"

[[ "$PROFILE" == release ]] || exit 0

# --- Installer -----------------------------------------------------------------
# Like OBS's plugin template: installs into the user's
# ~/Library/Application Support/obs-studio/plugins (no admin password).
WORK="$DIST/pkg-work"
PLUGINS="$WORK/root/Library/Application Support/obs-studio/plugins"
mkdir -p "$PLUGINS"
ditto "$BUNDLE" "$PLUGINS/obs-softphone.plugin"

# Never "relocate" the install to some other copy of the bundle on disk.
pkgbuild --analyze --root "$WORK/root" "$WORK/component.plist" >/dev/null
plutil -replace 0.BundleIsRelocatable -bool NO "$WORK/component.plist"

pkgbuild --identifier io.github.lastpatriot.obs-softphone --version "$VERSION" \
  --root "$WORK/root" --component-plist "$WORK/component.plist" \
  "$WORK/obs-softphone.pkg" >/dev/null

cat > "$WORK/distribution.xml" <<XML
<?xml version="1.0" encoding="utf-8" standalone="no"?>
<installer-gui-script minSpecVersion="1.0">
    <title>SIP Call-In for OBS $VERSION</title>
    <options rootVolumeOnly="true" hostArchitectures="arm64,x86_64" customize="never" allow-external-scripts="no" />
    <domains enable_currentUserHome="true" enable_anywhere="false" enable_localSystem="false" />
    <volume-check>
        <allowed-os-versions><os-version min="12.0" /></allowed-os-versions>
    </volume-check>
    <choices-outline><line choice="obs-softphone" /></choices-outline>
    <choice id="obs-softphone" title="SIP Call-In for OBS"><pkg-ref id="io.github.lastpatriot.obs-softphone" /></choice>
    <pkg-ref id="io.github.lastpatriot.obs-softphone" version="$VERSION">#obs-softphone.pkg</pkg-ref>
</installer-gui-script>
XML

productbuild --distribution "$WORK/distribution.xml" --package-path "$WORK" "$DIST/$NAME.pkg" >/dev/null
rm -rf "$WORK"
echo "$DIST/$NAME.pkg"
