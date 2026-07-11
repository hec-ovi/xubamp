#!/usr/bin/env bash
# Install the xubamp app icon and desktop entry into the current user's local data dir, so
# GNOME shows the icon on the running window (dash, overview, alt-tab) and can launch it.
# Idempotent: re-run after changing the artwork. Undo with packaging/uninstall-icons.sh.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
apps="$data/applications"

mkdir -p "$apps"
install -Dm644 "$root/packaging/xubamp.desktop" "$apps/xubamp.desktop"

for png in "$root"/icons/*x*.png; do
    base="$(basename "$png" .png)" # e.g. 256x256
    case "$base" in
        [0-9]*x[0-9]*)
            dir="$data/icons/hicolor/$base/apps"
            mkdir -p "$dir"
            install -Dm644 "$png" "$dir/xubamp.png"
            ;;
    esac
done

command -v gtk-update-icon-cache >/dev/null 2>&1 &&
    gtk-update-icon-cache -f -t "$data/icons/hicolor" >/dev/null 2>&1 || true
command -v update-desktop-database >/dev/null 2>&1 &&
    update-desktop-database "$apps" >/dev/null 2>&1 || true

echo "Installed xubamp.desktop and icons under $data"
echo "The window app_id is 'xubamp', matching xubamp.desktop, so GNOME will show the icon."
echo "If it does not appear on an already-running window, the icon lands on the next launch."
