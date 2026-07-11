#!/usr/bin/env bash
# Remove the xubamp desktop entry and icons installed by install-icons.sh.
set -euo pipefail

data="${XDG_DATA_HOME:-$HOME/.local/share}"
rm -f "$data/applications/xubamp.desktop"
find "$data/icons/hicolor" -name 'xubamp.png' -path '*/apps/*' -delete 2>/dev/null || true

command -v gtk-update-icon-cache >/dev/null 2>&1 &&
    gtk-update-icon-cache -f -t "$data/icons/hicolor" >/dev/null 2>&1 || true
command -v update-desktop-database >/dev/null 2>&1 &&
    update-desktop-database "$data/applications" >/dev/null 2>&1 || true

echo "Removed xubamp.desktop and icons from $data"
