#!/bin/sh
# make.sh <GCarve.app> <out.dmg>: dmg with the drag-to-Applications window laid out on background.tiff.
# Finder writes the layout into the volume's .DS_Store (needs Finder automation permission the first time).
set -e
APP=$1; DMG=$2
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
mkdir -p "$WORK/stage/.background"
cp -R "$APP" "$WORK/stage/"
ln -s /Applications "$WORK/stage/Applications"
cp "$HERE/background.tiff" "$WORK/stage/.background/"
hdiutil create -quiet -volname GCarve -srcfolder "$WORK/stage" -format UDRW -fs HFS+ "$WORK/rw.dmg"
MNT=$(hdiutil attach -readwrite -noverify -noautoopen "$WORK/rw.dmg" | awk -F'\t' '/\/Volumes\//{print $NF}')
sleep 2
osascript <<OSA
with timeout of 120 seconds
tell application "Finder"
  tell disk "$(basename "$MNT")"
    open
    set w to container window
    set current view of w to icon view
    set toolbar visible of w to false
    set statusbar visible of w to false
    set the bounds of w to {200, 120, 840, 568}
    set o to the icon view options of w
    set arrangement of o to not arranged
    set icon size of o to 128
    set text size of o to 13
    set background picture of o to file ".background:background.tiff"
    set extension hidden of item "GCarve.app" to true
    set position of item "GCarve.app" of w to {170, 200}
    set position of item "Applications" of w to {470, 200}
    update without registering applications
    delay 1
    close
  end tell
end tell
end timeout
OSA
SetFile -a V "$MNT/.background" 2>/dev/null || true
sync
hdiutil detach -quiet "$MNT"
rm -f "$DMG"
hdiutil convert -quiet "$WORK/rw.dmg" -format UDZO -imagekey zlib-level=9 -o "$DMG"
rm -rf "$WORK"
