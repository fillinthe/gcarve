#!/bin/sh
# Send the signed dmg (build.sh with GCARVE_RELEASE=1) to Apple's notary service and staple the ticket, so it opens on
# any Mac without the "unidentified developer" warning. Credentials: a notarytool keychain profile named "gcarve"
# (`xcrun notarytool store-credentials gcarve …`, made once by the owner), or an App Store Connect API key:
# GCARVE_ASC_KEY (.p8 path), GCARVE_ASC_KEY_ID, GCARVE_ASC_ISSUER.
set -eu
OUT="$(cd "$(dirname "$0")/.." && pwd)/out"
DMG="$(ls -t "$OUT"/GCarve-*-macOS.dmg | head -1)"
ASC="$HOME/.appstoreconnect"
if [ -z "${GCARVE_ASC_KEY:-}" ] && ls "$ASC"/private_keys/AuthKey_*.p8 >/dev/null 2>&1; then
    GCARVE_ASC_KEY="$(ls "$ASC"/private_keys/AuthKey_*.p8 | head -1)"
    GCARVE_ASC_KEY_ID="$(basename "$GCARVE_ASC_KEY" .p8 | sed 's/^AuthKey_//')"
    GCARVE_ASC_ISSUER="$(tr -d '[:space:]' < "$ASC/issuer")"
fi
if [ -n "${GCARVE_ASC_KEY:-}" ]; then
    xcrun notarytool submit "$DMG" --key "$GCARVE_ASC_KEY" --key-id "$GCARVE_ASC_KEY_ID" --issuer "$GCARVE_ASC_ISSUER" --wait
else
    xcrun notarytool submit "$DMG" --keychain-profile gcarve --wait
fi
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"
spctl -a -t open --context context:primary-signature -v "$DMG"
