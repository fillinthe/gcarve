#!/bin/sh
# Runs the packaged Linux build headless in Docker (Debian 12, Xvfb + Mesa llvmpipe) with samples/showcase.tlp
# and saves out/shot-linux-<arch>.png. Arch: aarch64 (native, default) or x86_64 (qemu emulation, slow).
. "$(dirname "$0")/../env.sh"
A="${1:-aarch64}"
[ "$A" = x86_64 ] && P=linux/amd64 || P=linux/arm64
docker run --rm --platform $P -v "$OUT:/out" debian:bookworm-slim sh -ec "
  apt-get update -qq && apt-get install -y -qq --no-install-recommends xvfb xauth libgl1 libegl1 libgl1-mesa-dri \
    libxkbcommon-x11-0 libxcursor1 libxi6 libxrandr2 >/dev/null
  cd /out/GCarve-$VERSION-Linux-$A
  GCARVE_SHOT=/out/shot-linux-$A.png LIBGL_ALWAYS_SOFTWARE=1 xvfb-run -s '-screen 0 1400x900x24' ./gcarve samples/showcase.tlp"
test -s "$OUT/shot-linux-$A.png" && echo "PASS Linux $A" || { echo "FAIL Linux $A"; exit 1; }
