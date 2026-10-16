#!/bin/sh
# Regenerates the C++ golden files in rust/testdata/posts/ from the C++ sources of this tree
# (compiled directly rather than linking build/*.a, which can be older than the sources). -ffp-contract=off: Rust never
# fuses a*b+c into FMA, clang on arm64 does by default; that ULP noise can flip knife-edge decisions (arcfit max sweep).
set -e
here=$(cd "$(dirname "$0")" && pwd)
repo="$here/../../../.."
out=$(mktemp -d)
c++ -std=c++20 -O1 -ffp-contract=off -I"$repo/core" -I"$repo/post" "$here/gen_golden.cpp" "$repo/post/postdef.cpp" "$repo/post/wincnc.cpp" \
    "$repo/core/arcfit.cpp" "$repo/core/gcode.cpp" -o "$out/gen_golden"
"$out/gen_golden" "$here/.."
rm -rf "$out"
