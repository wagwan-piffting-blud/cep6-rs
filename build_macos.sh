#!/bin/bash
# Build cep6 for macOS as one universal binary (Apple silicon + Intel), sign it
# with the Developer ID identity in the login keychain, and notarize it.
#
#     ./build_macos.sh
#     NOTARY_ISSUER= ./build_macos.sh      # sign only
#
# Run on the Mac, from this folder. Over SSH the login keychain is locked and
# codesign fails with errSecInternalComponent; run `security unlock-keychain`
# first. A bare Mach-O cannot be stapled, so Gatekeeper looks the ticket up
# online the first time the binary runs.

set -euo pipefail

# By SHA-1: the expiring and the new Developer ID Application certificates
# share a name, so the name alone is ambiguous to codesign. This is the new
# one (valid 2026-10-02 to 2031-09-17).
IDENTITY="${IDENTITY:-303D624AEF460613E64D9F807DBB15B7939369B9}"
NOTARY_KEY="${NOTARY_KEY:-$HOME/Desktop/AuthKey_PQ62K6LJPG.p8}"
NOTARY_KEY_ID="${NOTARY_KEY_ID:-PQ62K6LJPG}"
NOTARY_ISSUER="${NOTARY_ISSUER:-cf1db9e5-65cc-4af6-a1a5-a6516e4b1d88}"
DIST="${DIST:-dist}"

export PATH="$HOME/.cargo/bin:$PATH"
# the source sits on an SMB share; building there is slow and would share
# target/ with the Windows builds
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/ceps-target}"

for t in aarch64-apple-darwin x86_64-apple-darwin; do
    rustup target list --installed | grep -qx "$t" || rustup target add "$t"
    echo "=== $t"
    MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --target "$t" --bin cep6
done

mkdir -p "$DIST"
out="$DIST/cep6-macos-universal"
lipo -create -output "$out" \
    "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/cep6" \
    "$CARGO_TARGET_DIR/x86_64-apple-darwin/release/cep6"
lipo -info "$out"

codesign --force --timestamp --options runtime --sign "$IDENTITY" "$out"
codesign --verify --strict --verbose=2 "$out"

if [ -z "$NOTARY_ISSUER" ]; then
    echo "signed $out; set NOTARY_ISSUER to notarize"
    exit 0
fi

zip="$CARGO_TARGET_DIR/cep6-macos-universal.zip"
rm -f "$zip"
ditto -c -k --keepParent "$out" "$zip"
xcrun notarytool submit "$zip" --wait \
    --key "$NOTARY_KEY" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER"
echo "notarized $out"
