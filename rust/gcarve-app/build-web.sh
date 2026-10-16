#!/bin/sh
# Web build of the Rust app: rust/gcarve-app/web/dist/ (index.html, gcarve.js, gcarve_bg.wasm). Needs wasm-bindgen-cli
# (same version as Cargo.lock) and binaryen's wasm-opt. Serve the folder from any static server.
set -e
cd "$(dirname "$0")/.."
export PATH=/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH
cargo build -p gcarve-app --target wasm32-unknown-unknown --profile release-web
rm -rf gcarve-app/web/dist && mkdir -p gcarve-app/web/dist
wasm-bindgen --target web --no-typescript --out-dir gcarve-app/web/dist --out-name gcarve target/wasm32-unknown-unknown/release-web/gcarve.wasm
wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-reference-types -o gcarve-app/web/dist/gcarve_bg.wasm gcarve-app/web/dist/gcarve_bg.wasm
cp gcarve-app/web/index.html gcarve-app/web/dist/
# script + program are loaded as ?v=<fingerprint of this build> (index.html itself is never cached): Cloudflare gives
# .js a 4 hour browser cache, and an old gcarve.js with a new .wasm does not start ("LinkError")
b=$(cat gcarve-app/web/dist/gcarve.js gcarve-app/web/dist/gcarve_bg.wasm | shasum | cut -c1-10)
sed -i '' "s|v=BUILD|v=$b|g" gcarve-app/web/dist/index.html
sed -i '' "s|RAWSIZE|$(wc -c < gcarve-app/web/dist/gcarve_bg.wasm | tr -d ' ')|" gcarve-app/web/dist/index.html
grep -q "v=$b" gcarve-app/web/dist/index.html || { echo "build stamp missing from index.html"; exit 1; }
# the inline-JS snippets too (Web Serial, WebHID, …): wasm-bindgen numbers them inline0.js, inline1.js … in an order that
# shifts when one is added, so a cached copy under the same name is a different file. Only the import statements: the
# same paths further down are the wasm's import names and must stay as they are.
sed -i '' "s|^\\(import .* from ['\"]\\./snippets/[^'\"]*\\)\\(['\"]\\)|\\1?v=$b\\2|" gcarve-app/web/dist/gcarve.js
if grep -q "^import .*snippets/" gcarve-app/web/dist/gcarve.js; then
  ! grep "^import .*snippets/" gcarve-app/web/dist/gcarve.js | grep -qv "?v=$b" || { echo "snippet imports not stamped"; exit 1; }
fi
# the site's account bar is cached a day: point at this build's copy of it (changes show up at once)
v=$(shasum ../site/public/assets/accountbar.js | cut -c1-8)
sed -i '' "s|/assets/accountbar.js\"|/assets/accountbar.js?v=$v\"|" gcarve-app/web/dist/index.html
# Brotli copies for the site's file server (`precompressed br`): the program is 3.0 MB instead of 4.1 MB gzipped
if command -v brotli >/dev/null; then
  brotli -q 11 -k -f gcarve-app/web/dist/gcarve_bg.wasm gcarve-app/web/dist/gcarve.js
else
  echo "brotli not installed: the site will serve gzip (brew install brotli)"
fi
ls -l gcarve-app/web/dist
