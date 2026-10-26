#!/bin/sh
# Installs Fluxo for the current user (no sudo): binary, icon and menu shortcut.
set -e
cd "$(dirname "$0")"
install -Dm755 fluxo "$HOME/.local/bin/fluxo"
install -Dm644 fluxo.png "$HOME/.local/share/icons/hicolor/256x256/apps/fluxo.png"
sed "s|^Exec=fluxo|Exec=$HOME/.local/bin/fluxo|" fluxo.desktop > "$HOME/.local/share/applications/fluxo.desktop"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
echo "Fluxo installed. Look for \"Fluxo\" in your application menu."
