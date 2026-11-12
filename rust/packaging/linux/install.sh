#!/bin/sh
# Installs GCarve for this user: ~/.local/bin/gcarve, menu entry and icon. Runtime needs (usually present on a
# desktop): libGL/libEGL (Mesa or the GPU driver), libxkbcommon-x11 (X11) or libwayland-client, and an
# xdg-desktop-portal for the file dialogs.
set -e
cd "$(dirname "$0")"
mkdir -p ~/.local/bin ~/.local/share/applications ~/.local/share/icons/hicolor/1024x1024/apps
cp gcarve ~/.local/bin/
# full paths: ~/.local/bin is often not on a menu's PATH, and many hicolor themes have no 1024x1024 folder
sed -e "s#^Exec=gcarve#Exec=$HOME/.local/bin/gcarve#" -e "s#^Icon=.*#Icon=$HOME/.local/share/icons/hicolor/1024x1024/apps/gcarve.png#" \
  gcarve.desktop > ~/.local/share/applications/gcarve.desktop
cp gcarve.png ~/.local/share/icons/hicolor/1024x1024/apps/
echo "Installed: ~/.local/bin/gcarve (make sure ~/.local/bin is on PATH)"
