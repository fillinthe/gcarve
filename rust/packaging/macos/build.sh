#!/bin/sh
# macOS: universal (arm64 + x86_64) GCarve.app + .dmg in rust/packaging/out/.
# Sign identity: GCARVE_CODESIGN_IDENTITY (e.g. "GCarve Dev"), default ad-hoc ("-"). Releases: GCARVE_RELEASE=1 signs with
# the Developer ID certificate in the gitignored signing/ folder (its own keychain, unlocked here), hardened runtime +
# timestamp, so notarize.sh can send it to Apple.
. "$(dirname "$0")/../env.sh"
cd "$RUST"
# no build-machine paths in the binary
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=/home --remap-path-prefix=$REPO=/gcarve"
MIN_X86=10.13   # Rust's x86_64-apple-darwin floor is 10.12
MIN_ARM=11.0    # first macOS on Apple silicon
rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
MACOSX_DEPLOYMENT_TARGET=$MIN_ARM cargo build -p gcarve-app --profile dist $FEATURES --target aarch64-apple-darwin
MACOSX_DEPLOYMENT_TARGET=$MIN_X86 cargo build -p gcarve-app --profile dist $FEATURES --target x86_64-apple-darwin

APP="$OUT/GCarve.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/gcarve" target/aarch64-apple-darwin/dist/gcarve target/x86_64-apple-darwin/dist/gcarve
cp "$REPO/app/branding/appicon.icns" "$REPO/app/branding/docicon.icns" "$APP/Contents/Resources/"
# app/Info.plist.in with the CMake placeholders filled; minimum = the Intel floor (the arm64 slice needs 11 anyway)
sed -e 's/${MACOSX_BUNDLE_EXECUTABLE_NAME}/gcarve/' -e "s/\${MACOSX_BUNDLE_BUNDLE_VERSION}/$VERSION/" \
    -e "s/\${MACOSX_BUNDLE_SHORT_VERSION_STRING}/$VERSION/" -e "s#<string>12.0</string>#<string>$MIN_X86</string>#" \
    "$REPO/app/Info.plist.in" > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist" >/dev/null
if [ "${GCARVE_RELEASE:-}" = 1 ]; then
    SIGN="$REPO/signing"
    KC="$SIGN/gcarve-release.keychain-db"
    security unlock-keychain -p "$(cat "$SIGN/keychain-password")" "$KC"
    # by fingerprint: the same certificate may also sit in the login keychain (the name alone is then ambiguous)
    codesign --force --deep --options runtime --timestamp --keychain "$KC" --sign "$(cat "$SIGN/identity-sha1")" "$APP"
else
    codesign --force --deep --sign "${GCARVE_CODESIGN_IDENTITY:--}" "$APP"
fi

DMG="$OUT/GCarve-$VERSION-macOS.dmg"
sh "$REPO/rust/packaging/macos/dmg/make.sh" "$APP" "$DMG"   # drag-to-Applications window on dmg/background.tiff
if [ "${GCARVE_RELEASE:-}" = 1 ]; then
    codesign --force --timestamp --keychain "$KC" --sign "$(cat "$SIGN/identity-sha1")" "$DMG"
fi
codesign -dv "$APP" 2>&1 | grep -E "^Authority" | head -1
lipo -info "$APP/Contents/MacOS/gcarve"
ls -l "$APP/Contents/MacOS/gcarve" "$DMG"
