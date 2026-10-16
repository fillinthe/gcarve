#!/bin/sh
# Runs the packaged Windows builds under Wine in Docker (no Windows PC needed):
#   sh wine-test.sh [x64] [win7-x64] [win7-x86]     (default: all three; build them first with build.sh)
# Per build: unzips the zip into a scratch copy, then
#   1. gcarve.exe samples\showcase.tlp with GCARVE_SHOT -> out/shot-wine-<variant>.png
#   2. run-tests.cmd under `wine cmd` -> out/wine-test-results-<variant>.txt (+ test-shot.png)
#   3. gcarve.exe --control (machine screen, built-in simulator) -> out/shot-wine-<variant>-control.png
# Wine 10 (Debian 13) + Xvfb + Mesa llvmpipe: wine's opengl32 maps to the container's GL (llvmpipe, GL 4.5).
# Image = Dockerfile.wine (amd64). It runs in its own Colima profile `gcarve-wine` (30 GB disk, Rosetta for
# x86_64): the default profile's 20 GB disk is shared with other projects and filled up (VM crash) when the
# image was unpacked there. Created on first use; `colima stop gcarve-wine` / `colima delete gcarve-wine` frees it.
. "$(dirname "$0")/../env.sh"
VARIANTS="${*:-x64 win7-x64 win7-x86}"
colima status -p gcarve-wine >/dev/null 2>&1 ||
  colima start gcarve-wine --vm-type vz --vz-rosetta --arch aarch64 --cpu 8 --memory 8 --disk 30 --mount-type virtiofs
export DOCKER_HOST="unix://$HOME/.colima/gcarve-wine/docker.sock"   # never touch the default profile
docker build -q --platform linux/amd64 -t gcarve-wine-test -f "$PKG/windows/Dockerfile.wine" "$PKG/windows" >/dev/null
FAIL=0
for V in $VARIANTS; do
  NAME="GCarve-$VERSION-Windows-$V"
  [ -f "$OUT/$NAME.zip" ] || { echo "missing $OUT/$NAME.zip (sh build.sh $V)"; FAIL=1; continue; }
  rm -f "$OUT/shot-wine-$V.png" "$OUT/shot-wine-$V-control.png" "$OUT/wine-test-results-$V.txt"
  docker run --rm --platform linux/amd64 -v "$OUT:/out" -e V="$V" -e NAME="$NAME" gcarve-wine-test sh -c '
    Xvfb :9 -screen 0 1400x900x24 >/dev/null 2>&1 & sleep 2; export DISPLAY=:9
    rm -rf /tmp/w && mkdir /tmp/w && cd /tmp/w && unzip -q "/out/$NAME.zip" && cd "$NAME"
    export WINEDEBUG=err+all,fixme-all
    echo "== $V: showcase.tlp"
    GCARVE_SHOT="Z:\\out\\shot-wine-$V.png" GCARVE_SHOT_FRAME=12 timeout 900 wine gcarve.exe "samples\\showcase.tlp"
    echo "exit $?"
    echo "== $V: run-tests.cmd"
    # `start notepad` at the end would block: let it time out, results are written before it
    timeout 900 wine cmd /c run-tests.cmd >/dev/null 2>&1
    tr -d "\r" < test-results.txt > "/out/wine-test-results-$V.txt"; cat "/out/wine-test-results-$V.txt"
    [ -f test-shot.png ] && cp test-shot.png "/out/shot-wine-$V-runtests.png"
    echo "== $V: --control (simulator)"
    GCARVE_SHOT="Z:\\out\\shot-wine-$V-control.png" GCARVE_SHOT_FRAME=12 timeout 900 wine gcarve.exe --control
    echo "exit $?"
    wineserver -k 2>/dev/null; true'
  for f in shot-wine-$V.png shot-wine-$V-control.png; do
    if [ -s "$OUT/$f" ]; then echo "PASS $f"; else echo "FAIL $f"; FAIL=1; fi
  done
  grep -q "RESULT: PASSED" "$OUT/wine-test-results-$V.txt" 2>/dev/null && echo "PASS run-tests.cmd $V" || { echo "FAIL run-tests.cmd $V"; FAIL=1; }
done
exit $FAIL
