#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Builds the plugin and installs it for the current user's OBS (dev builds).
#   scripts/install-dev-macos.sh [--release]
# Settings: ~/Library/Application Support/obs-studio/plugin_config/obs-softphone/config.json
# (written by Tools → SIP Call-In…; the password goes to the Keychain).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [[ "${1:-}" == "--release" ]]; then
  BUNDLE="$("$ROOT/scripts/package-macos.sh" | head -1)"
else
  BUNDLE="$("$ROOT/scripts/package-macos.sh" --debug | head -1)"
fi

DEST="$HOME/Library/Application Support/obs-studio/plugins/obs-softphone.plugin"
rm -rf "$DEST"
mkdir -p "$(dirname "$DEST")"
ditto "$BUNDLE" "$DEST"
echo "Installed $DEST"
echo "Restart OBS, then open Docks → Call-In."
