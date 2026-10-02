#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Fetches and builds the native dependencies into third_party/ (macOS):
#   - Opus, static, per architecture
#   - pjproject, static, per architecture: no sound/video devices, TLS through
#     Apple's Network.framework (no OpenSSL), Opus
#   - obs-deps Qt6 headers matching OBS (for the dock)
# ARCHS: space-separated, default the host's ("arm64" or "x86_64").
#   ARCHS="arm64 x86_64" scripts/bootstrap-macos.sh   # for a universal build
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TP="$ROOT/third_party"
JOBS="$(sysctl -n hw.ncpu)"
ARCHS="${ARCHS:-$(uname -m)}"
MIN_MACOS="12.0"

PJ_VERSION="2.17"
OPUS_VERSION="1.5.2"
OPUS_SHA256="65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1" # xiph SHA256SUMS.txt
QT6_DEPS_VERSION="2026-07-15" # obs-studio 32.2.2 CMakePresets.json
QT6_DEPS_SHA256="d4b8058612a7067e44b2205fe7925ee24e9ec6b15ac8c29d3e6230c70030b102"

# Bump when the build options below change, to force a rebuild.
BUILD_REV="v3-apple-tls"

mkdir -p "$TP"

host_triple() {
  case "$1" in
    arm64) echo "aarch64-apple-darwin" ;;
    x86_64) echo "x86_64-apple-darwin" ;;
    *) echo "unknown arch $1" >&2; exit 1 ;;
  esac
}

# --- Opus ------------------------------------------------------------------
fetch_opus() {
  local tarball="$TP/opus-$OPUS_VERSION.tar.gz"
  if [[ ! -d "$TP/opus-src" ]]; then
    curl -fsSL -o "$tarball" "https://downloads.xiph.org/releases/opus/opus-$OPUS_VERSION.tar.gz"
    echo "$OPUS_SHA256  $tarball" | shasum -a 256 -c - >/dev/null
    mkdir -p "$TP/opus-src"
    tar -xzf "$tarball" -C "$TP/opus-src" --strip-components 1
    rm "$tarball"
  fi
}

build_opus() {
  local arch="$1" prefix="$TP/opus-$1" build="$TP/build/opus-$1"
  if [[ -f "$prefix/.rev-$BUILD_REV" ]]; then
    echo "opus ($arch): already built"
    return
  fi
  rm -rf "$prefix" "$build"
  mkdir -p "$build"
  (
    cd "$build"
    CFLAGS="-O2 -arch $arch -mmacosx-version-min=$MIN_MACOS" \
      "$TP/opus-src/configure" --host="$(host_triple "$arch")" --prefix="$prefix" \
      --disable-shared --enable-static --disable-doc --disable-extra-programs >/dev/null
    make -j"$JOBS" >/dev/null
    make install >/dev/null
  )
  rm -rf "$build"
  touch "$prefix/.rev-$BUILD_REV"
  echo "opus ($arch): built"
}

# --- pjproject -------------------------------------------------------------
fetch_pjproject() {
  if [[ ! -d "$TP/pjproject-src" ]]; then
    git clone -q --depth 1 --branch "$PJ_VERSION" https://github.com/pjsip/pjproject "$TP/pjproject-src"
  fi
}

build_pjproject() {
  local arch="$1" prefix="$TP/pjproject-$1" build="$TP/build/pjproject-$1"
  if [[ -f "$prefix/.rev-$BUILD_REV" ]]; then
    echo "pjproject ($arch): already built"
    return
  fi
  # pjproject builds in its source tree: one copy per architecture.
  rm -rf "$prefix" "$build"
  mkdir -p "$TP/build"
  cp -R "$TP/pjproject-src" "$build"
  cat > "$build/pjlib/include/pj/config_site.h" <<'EOF'
/* obs-softphone: one account, one call, no sound device, no video. */
#define PJ_HAS_SSL_SOCK            1
#if defined(__APPLE__)
/* TLS through Apple's Network.framework: TLS 1.3, system trust store. */
#  undef  PJ_SSL_SOCK_IMP
#  define PJ_SSL_SOCK_IMP          PJ_SSL_SOCK_IMP_APPLE
#endif
#define PJSUA_MAX_CALLS            4
#define PJSUA_MAX_ACC              2
#define PJMEDIA_HAS_VIDEO          0
#define PJMEDIA_CONF_USE_SWITCH_BOARD 0
/* Answer with our codec order (opus, G722, PCMU, PCMA), not the offerer's. */
#define PJMEDIA_SDP_NEG_PREFER_REMOTE_CODEC_ORDER 0
EOF
  (
    cd "$build"
    export CFLAGS="-O2 -fPIC -arch $arch -mmacosx-version-min=$MIN_MACOS"
    export LDFLAGS="-arch $arch"
    # Network.framework TLS needs these; configure records them in the .pc
    # file so the Rust build links them too.
    export LIBS="-framework Network -framework Security -framework CoreFoundation"
    # --disable-darwin-ssl: don't pick the deprecated Secure Transport
    # backend (config_site.h selects Network.framework). --with-ssl=no and
    # no OpenSSL on the default paths: nothing links OpenSSL.
    ./configure --host="$(host_triple "$arch")" --prefix="$prefix" \
      --disable-sound --disable-video --disable-pjsua2 \
      --disable-v4l2 --disable-sdl --disable-ffmpeg --disable-openh264 --disable-vpx \
      --disable-libyuv --disable-libwebrtc --disable-upnp \
      --disable-speex-codec --disable-speex-aec --disable-ilbc-codec --disable-gsm-codec \
      --disable-l16-codec --disable-silk --disable-bcg729 \
      --disable-darwin-ssl --with-ssl=no \
      --with-opus="$TP/opus-$arch" >/dev/null
    make dep >/dev/null
    make -j"$JOBS" >/dev/null
    make install >/dev/null
  )
  if grep -q -- '-lssl\|-lcrypto' "$prefix/lib/pkgconfig/libpjproject.pc"; then
    echo "pjproject ($arch): unexpectedly linked OpenSSL" >&2
    exit 1
  fi
  rm -rf "$build"
  touch "$prefix/.rev-$BUILD_REV"
  echo "pjproject ($arch): built"
}

fetch_opus
fetch_pjproject
for arch in $ARCHS; do
  build_opus "$arch"
  build_pjproject "$arch"
done

# --- Qt6 headers (obs-deps) --------------------------------------------------
QT_DIR="$TP/obs-deps-qt6"
if [[ ! -d "$QT_DIR/lib/QtWidgets.framework/Headers" ]]; then
  ARCHIVE="$TP/macos-deps-qt6-$QT6_DEPS_VERSION-universal.tar.xz"
  curl -fsSL -o "$ARCHIVE" \
    "https://github.com/obsproject/obs-deps/releases/download/$QT6_DEPS_VERSION/macos-deps-qt6-$QT6_DEPS_VERSION-universal.tar.xz"
  echo "$QT6_DEPS_SHA256  $ARCHIVE" | shasum -a 256 -c - >/dev/null
  mkdir -p "$QT_DIR"
  tar -xJf "$ARCHIVE" -C "$QT_DIR"
  xattr -r -d com.apple.quarantine "$QT_DIR" 2>/dev/null || true
  rm "$ARCHIVE"
else
  echo "qt6 headers: already present"
fi

echo "bootstrap done ($ARCHS)"
