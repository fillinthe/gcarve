#!/bin/sh
# Linux x86_64 + arm64, built in Docker (docker/Dockerfile.build, Debian 11 = glibc 2.31 floor):
# -> rust/packaging/out/GCarve-<version>-Linux-<arch>.tar.gz (gcarve, gcarve.desktop, gcarve.png, install.sh)
. "$(dirname "$0")/../env.sh"
in_docker "$@"
cd "$RUST"
for T in aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
  cargo build -p gcarve-app --profile dist $FEATURES --target $T
  NAME="GCarve-$VERSION-Linux-${T%%-*}"
  STAGE="$OUT/$NAME"; rm -rf "$STAGE" "$STAGE.tar.gz"; mkdir -p "$STAGE/samples"
  cp "$TARGET_DIR/$T/dist/gcarve" "$STAGE/"
  cp "$REPO/app/branding/gcarve.desktop" "$STAGE/"
  cp "$REPO/app/branding/appicon-1024.png" "$STAGE/gcarve.png"
  cp "$PKG/linux/install.sh" "$STAGE/"
  cp "$REPO/samples/showcase.tlp" "$REPO/samples/demo.dxf" "$REPO/samples/demo.svg" "$REPO/samples/pyramid.stl" "$STAGE/samples/"
  tar -C "$OUT" -czf "$STAGE.tar.gz" "$NAME"
  echo "$NAME: glibc symbols up to $(objdump -T "$STAGE/gcarve" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"
  ls -l "$STAGE/gcarve" "$STAGE.tar.gz"
done
