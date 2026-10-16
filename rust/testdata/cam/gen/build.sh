#!/bin/sh
# Rebuilds the C++ goldens for the Rust CAM tests (rust/testdata/cam/*.json).
#
# The engine (core/, post/wincnc.cpp) and Clipper2 are compiled from source with -ffp-contract=off: Rust never fuses
# a*b+c into FMA, while clang on arm64 does by default, which moves results by an ULP and can flip borderline
# decisions (e.g. where fitArcs splits an arc). Without contraction the C++ and Rust engines agree bit for bit.
# LIB=1 links the app's prebuilt build/core/libgcarve_core.a (FMA on) instead, to measure that difference.
# BUILD: a configured C++ build dir (for the Clipper2 sources fetched by CMake), default <repo>/build.
# OUT_DIR: where the JSON goes (default rust/testdata/cam/).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../../.." && pwd)
BUILD=${BUILD:-$ROOT/build}
OUT_DIR=${OUT_DIR:-$ROOT/rust/testdata/cam/}
EXE=${TMPDIR:-/tmp}/gcarve_cam_dump
CLIP="$BUILD/_deps/clipper2-src/CPP/Clipper2Lib"
INC="-I$ROOT/core -I$ROOT/post -I$CLIP/include -isystem $ROOT/third_party/boost_voronoi"
if [ "$LIB" = 1 ]; then
  c++ -std=c++20 -O2 -D_USE_MATH_DEFINES $INC "$HERE/dump.cpp" "$BUILD/post/libgcarve_post.a" "$BUILD/core/libgcarve_core.a" \
    "$BUILD/_deps/clipper2-build/libClipper2.a" -o "$EXE"
else
  c++ -std=c++20 -O2 -ffp-contract=off -D_USE_MATH_DEFINES $INC "$HERE/dump.cpp" "$ROOT"/core/*.cpp "$ROOT/post/wincnc.cpp" \
    "$CLIP"/src/clipper.engine.cpp "$CLIP"/src/clipper.offset.cpp "$CLIP"/src/clipper.rectclip.cpp -o "$EXE"
fi
"$EXE" "$ROOT" "$OUT_DIR"
