#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Fetches and builds the native dependencies into third_party/ (macOS, dev builds).
#   - pjproject (static, no sound/video; TLS via Homebrew OpenSSL, Opus via Homebrew)
#   - obs-deps Qt6 headers matching the installed OBS (for the dock shim)
# Release builds (native TLS, universal binaries) are M5; see DESIGN.md §8.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TP="$ROOT/third_party"
JOBS="$(sysctl -n hw.ncpu)"

PJ_VERSION="2.17"
QT6_DEPS_VERSION="2026-07-15" # obs-studio 32.2.2 CMakePresets.json
QT6_DEPS_SHA256="d4b8058612a7067e44b2205fe7925ee24e9ec6b15ac8c29d3e6230c70030b102"

OPENSSL_DIR="${OPENSSL_DIR:-$(brew --prefix openssl@3)}"
OPUS_DIR="${OPUS_DIR:-$(brew --prefix opus)}"

mkdir -p "$TP"

# --- pjproject -------------------------------------------------------------
PJ_STAMP="$TP/pjproject-install/.config-site-v2"
if [[ ! -f "$PJ_STAMP" ]]; then
  rm -rf "$TP/pjproject-install"
  if [[ ! -d "$TP/pjproject" ]]; then
    git clone --depth 1 --branch "$PJ_VERSION" https://github.com/pjsip/pjproject "$TP/pjproject"
  fi
  cat > "$TP/pjproject/pjlib/include/pj/config_site.h" <<'EOF'
/* obs-softphone: one account, one call, no sound device, no video. */
#define PJ_HAS_SSL_SOCK            1
#define PJSUA_MAX_CALLS            4
#define PJSUA_MAX_ACC              2
#define PJMEDIA_HAS_VIDEO          0
#define PJMEDIA_CONF_USE_SWITCH_BOARD 0
/* Answer with our codec order (opus, G722, PCMU), not the offerer's. */
#define PJMEDIA_SDP_NEG_PREFER_REMOTE_CODEC_ORDER 0
EOF
  (
    cd "$TP/pjproject"
    export CFLAGS="-O2 -fPIC -mmacosx-version-min=12.0"
    ./configure --prefix="$TP/pjproject-install" \
      --disable-sound --disable-video --disable-pjsua2 \
      --disable-v4l2 --disable-sdl --disable-ffmpeg --disable-openh264 --disable-vpx \
      --disable-libyuv --disable-libwebrtc --disable-upnp \
      --disable-speex-codec --disable-speex-aec --disable-ilbc-codec --disable-gsm-codec \
      --disable-l16-codec --disable-silk --disable-bcg729 \
      --with-ssl="$OPENSSL_DIR" --with-opus="$OPUS_DIR"
    make clean >/dev/null 2>&1 || true
    make dep >/dev/null
    make -j"$JOBS"
    make install
  )
  touch "$PJ_STAMP"
else
  echo "pjproject: already built"
fi

# --- Qt6 headers (obs-deps) --------------------------------------------------
QT_DIR="$TP/obs-deps-qt6"
if [[ ! -d "$QT_DIR/lib/QtWidgets.framework/Headers" ]]; then
  ARCHIVE="$TP/macos-deps-qt6-$QT6_DEPS_VERSION-universal.tar.xz"
  curl -fL -o "$ARCHIVE" \
    "https://github.com/obsproject/obs-deps/releases/download/$QT6_DEPS_VERSION/macos-deps-qt6-$QT6_DEPS_VERSION-universal.tar.xz"
  echo "$QT6_DEPS_SHA256  $ARCHIVE" | shasum -a 256 -c -
  mkdir -p "$QT_DIR"
  tar -xJf "$ARCHIVE" -C "$QT_DIR"
  xattr -r -d com.apple.quarantine "$QT_DIR" 2>/dev/null || true
  rm "$ARCHIVE"
else
  echo "qt6 headers: already present"
fi

echo "bootstrap done"
