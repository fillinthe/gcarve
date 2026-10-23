#!/bin/sh
# Windows, cross-compiled with mingw-w64:
#   sh build.sh x64        Windows 10/11 x64   (x86_64-pc-windows-gnu, stable; on the Mac: brew install mingw-w64)
#   sh build.sh win7-x64   Windows 7/8 x64     (x86_64-win7-windows-gnu, tier 3: nightly + build-std)
#   sh build.sh win7-x86   Windows 7/8 32-bit  (i686-win7-windows-gnu,  tier 3: nightly + build-std)
# The win7 builds run in Docker (docker/Dockerfile.build): Debian's mingw-w64 links msvcrt.dll, which every
# Windows has; Homebrew's links the UCRT, missing on Windows 7/8.1 without update KB2999226.
# -> rust/packaging/out/GCarve-<version>-Windows-<variant>.zip (gcarve.exe, README.txt, run-tests.cmd, samples\)
. "$(dirname "$0")/../env.sh"
VARIANT="${1:-x64}"; EXTRA=""
HERE="$PKG/windows"
case "$VARIANT" in
  x64)      T=x86_64-pc-windows-gnu;  CC=x86_64-w64-mingw32; TC=stable ;;
  win7-x64) T=x86_64-win7-windows-gnu; CC=x86_64-w64-mingw32; TC=nightly ;;
  win7-x86) T=i686-win7-windows-gnu;  CC=i686-w64-mingw32;   TC=nightly; EXTRA="-C panic=abort" ;;  # mingw i686 unwinding (SJLJ) is not what Rust expects
  *) echo "usage: $0 x64|win7-x64|win7-x86"; exit 2 ;;
esac
[ $TC = nightly ] && in_docker "$@"
command -v $CC-gcc >/dev/null || { echo "needs mingw-w64: brew install mingw-w64"; exit 1; }
cd "$RUST"

# icon + version resource, linked straight into the exe (no build.rs change needed)
OBJ="$OUT/obj-$VARIANT"; mkdir -p "$OBJ"
V4="$(echo "$VERSION" | tr . ,),0"
sed -e "s#@ICON@#$REPO/app/branding/appicon.ico#" -e "s/@V4@/$V4/g" -e "s/@VERSION@/$VERSION/g" "$HERE/gcarve.rc.in" > "$OBJ/gcarve.rc"
$CC-windres "$OBJ/gcarve.rc" -O coff -o "$OBJ/gcarve-res.o"

TUP="$(echo $T | tr a-z- A-Z_)"
export CARGO_TARGET_${TUP}_LINKER=$CC-gcc
# GUI subsystem (no console window); the resource object carries icon + version info
# no build-machine paths in the exe (generated bindings embed their OUT_DIR)
export CARGO_TARGET_${TUP}_RUSTFLAGS="-C link-arg=-Wl,--subsystem,windows -C link-arg=$OBJ/gcarve-res.o --remap-path-prefix=$HOME=/home --remap-path-prefix=$REPO=/gcarve $EXTRA"
export CC_$(echo $T | tr - _)=$CC-gcc
if [ $TC = nightly ]; then
  rustup component add rust-src --toolchain nightly >/dev/null
  cargo +nightly build -p gcarve-app --profile dist $FEATURES --target $T -Zbuild-std=std,panic_abort
else
  rustup target add $T >/dev/null
  cargo build -p gcarve-app --profile dist $FEATURES --target $T
fi

EXE="$TARGET_DIR/$T/dist/gcarve.exe"
NAME="GCarve-$VERSION-Windows-$VARIANT"
STAGE="$OUT/$NAME"; rm -rf "$STAGE" "$STAGE.zip"; mkdir -p "$STAGE/samples"
cp "$EXE" "$STAGE/gcarve.exe"
case $VARIANT in win7-*)  # combase.dll -> ole32.dll, then fail if any Windows 8+ DLL is still imported
  python3 "$HERE/imports.py" "$STAGE/gcarve.exe" --fix-combase > "$OBJ/imports.txt" || { cat "$OBJ/imports.txt"; exit 1; }
  tail -1 "$OBJ/imports.txt" ;;
esac
cp "$HERE/run-tests.cmd" "$STAGE/"
sed "s/@VARIANT@/$VARIANT/" "$HERE/README.txt" > "$STAGE/README.txt"
cp "$REPO/samples/showcase.tlp" "$REPO/samples/demo.dxf" "$REPO/samples/demo.svg" "$REPO/samples/pyramid.stl" "$STAGE/samples/"
# CRLF for the Windows text files
for f in "$STAGE/run-tests.cmd" "$STAGE/README.txt"; do sed 's/$/\r/' "$f" > "$f.tmp" && mv "$f.tmp" "$f"; done
(cd "$OUT" && zip -qr "$NAME.zip" "$NAME")
ls -l "$STAGE/gcarve.exe" "$STAGE.zip"
