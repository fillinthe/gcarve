#!/bin/sh
# Renders background.html to background.tiff (1x + 2x in one file; Finder picks per screen).
set -e
cd "$(dirname "$0")"
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
TMP=$(mktemp -d)
for s in 1 2; do
  "$CHROME" --headless --disable-gpu --hide-scrollbars --user-data-dir="$TMP/prof" \
    --force-device-scale-factor=$s --window-size=640,420 \
    --screenshot="$TMP/bg$s.png" "file://$PWD/background.html" 2>/dev/null
done
sips -s dpiWidth 144 -s dpiHeight 144 "$TMP/bg2.png" >/dev/null
tiffutil -cathidpicheck "$TMP/bg1.png" "$TMP/bg2.png" -out background.tiff 2>/dev/null
rm -rf "$TMP"
ls -l background.tiff
