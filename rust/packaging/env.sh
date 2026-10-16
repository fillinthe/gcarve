# Sourced by the packaging scripts: paths, toolchain, version, the Docker hop.
set -eu
SCRIPT="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
PKG="$(cd "$(dirname "$0")/.." && pwd)"   # rust/packaging
RUST="$(dirname "$PKG")"                  # rust/
REPO="$(dirname "$RUST")"
OUT="$PKG/out"
mkdir -p "$OUT"
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
TARGET_DIR="${CARGO_TARGET_DIR:-$RUST/target}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$RUST/Cargo.toml" | head -1)"
# Packaged builds keep the `screenshot` feature: it only acts when GCARVE_SHOT is set, and it is how the
# self-checks (run-tests.cmd, the Linux/Wine Docker tests) prove the window really rendered.
FEATURES="--features screenshot"

# Re-runs the calling script (same arguments) inside the gcarve-rust-build image (docker/Dockerfile.build),
# unless already in a container. Separate target dir: host build scripts must not clobber the Mac ones.
# Cache: Docker volume gcarve-rust-registry (crates), rust/target/docker (build output).
in_docker() {
  [ -f /.dockerenv ] && return 0
  docker build -q -t gcarve-rust-build -f "$PKG/docker/Dockerfile.build" "$PKG/docker" >/dev/null
  exec docker run --rm -v "$REPO:/src" -v gcarve-rust-registry:/opt/cargo/registry \
    -e CARGO_TARGET_DIR=/src/rust/target/docker -w /src gcarve-rust-build sh "/src/${SCRIPT#"$REPO"/}" "$@"
}
