#!/usr/bin/env bash
# Install the xubamp app icon and desktop entry into the current user's local data dir, so
# GNOME shows the icon on the running window (dash, overview, alt-tab) and can launch it.
# Idempotent: re-run after changing the artwork. Undo with packaging/uninstall-icons.sh.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
apps="$data/applications"

mkdir -p "$apps"

# Recent GLib (Ubuntu 26.04 ships 2.88) refuses to load a .desktop whose Exec program is
# not resolvable, and GNOME Shell then never registers the app, so the running window gets
# no icon. The packaged .deb is fine (Exec=xubamp resolves from /usr/bin), but a dev
# checkout runs via cargo with xubamp off PATH. So point Exec at the built binary here.
bin=""
for cand in "$root/target/release/xubamp" "$root/target/debug/xubamp"; do
    [ -x "$cand" ] && bin="$cand" && break
done
if [ -n "$bin" ]; then
    sed "s|^Exec=xubamp|Exec=$bin|" "$root/packaging/xubamp.desktop" >"$apps/xubamp.desktop"
    chmod 644 "$apps/xubamp.desktop"
    echo "Exec points at $bin"
else
    install -Dm644 "$root/packaging/xubamp.desktop" "$apps/xubamp.desktop"
    echo "warning: no built xubamp binary found; run 'cargo build' first, else the icon" >&2
    echo "         will not appear until xubamp is on PATH." >&2
fi

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
