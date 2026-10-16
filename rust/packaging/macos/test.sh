#!/bin/sh
# Starts the packaged app once per slice (arm64 native, x86_64 under Rosetta) with samples/showcase.tlp
# and saves a screenshot: out/shot-macos-arm64.png, out/shot-macos-x86_64.png.
. "$(dirname "$0")/../env.sh"
BIN="$OUT/GCarve.app/Contents/MacOS/gcarve"
codesign --verify --deep --strict "$OUT/GCarve.app"
for a in arm64 x86_64; do
  rm -f "$OUT/shot-macos-$a.png"
  GCARVE_SHOT="$OUT/shot-macos-$a.png" arch -$a "$BIN" "$REPO/samples/showcase.tlp"
  test -s "$OUT/shot-macos-$a.png" && echo "PASS macOS $a" || { echo "FAIL macOS $a"; exit 1; }
done
