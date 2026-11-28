#!/bin/bash
# Builds dist/DyoSnap.app for Apple Silicon and a zip of it to pass around.
#
# Signing: uses the code-signing certificate named in DYOSNAP_SIGN_ID when it is in your
# keychain (default "DyoSnap"). The signature then stays the same from build to build, so
# macOS keeps the screen-recording permission.
# Without the certificate the app is signed ad hoc: it runs, but macOS asks for
# the permission again after every new build. See CONTRIBUTING.md for how to create one.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
SIGN_ID="${DYOSNAP_SIGN_ID:-DyoSnap}"
TARGET=aarch64-apple-darwin
APP=dist/DyoSnap.app

if [ ! -x build/ffmpeg/bin/ffmpeg ]; then
    ./scripts/build-ffmpeg.sh
fi

echo "==> Writing the license notices"
./scripts/notices.sh

# Keep local paths out of the binary (panic messages carry source paths). The last
# matching rule wins, so the catch-all for $HOME comes first. Encoded flags are split
# on \x1f, not on spaces, so folders with spaces in their name work.
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
CARGO_ENCODED_RUSTFLAGS=$(printf '%s\x1f' \
    "--remap-path-prefix=$HOME=/home" \
    "--remap-path-prefix=$RUSTUP_HOME=/rustup" \
    "--remap-path-prefix=$CARGO_HOME=/cargo" \
    "--remap-path-prefix=$PWD=/dyosnap")
export CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS%$'\x1f'}"

echo "==> Building DyoSnap $VERSION ($TARGET)"
MACOSX_DEPLOYMENT_TARGET=27.0 cargo build --release --target "$TARGET"

echo "==> Drawing the app icon"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cargo run --quiet --release --example app_icon -- "$WORK/AppIcon.iconset"

echo "==> Putting $APP together"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/$TARGET/release/dyosnap" "$APP/Contents/MacOS/dyosnap"
# Our own LGPL build (scripts/build-ffmpeg.sh), found next to the app binary at run time.
cp build/ffmpeg/bin/ffmpeg "$APP/Contents/MacOS/ffmpeg"
sed "s/@VERSION@/$VERSION/g" macos/Info.plist > "$APP/Contents/Info.plist"
plutil -lint -s "$APP/Contents/Info.plist"
iconutil -c icns "$WORK/AppIcon.iconset" -o "$APP/Contents/Resources/AppIcon.icns"
# Licenses travel with every copy of the app. The ffmpeg folder holds what the LGPL
# asks for besides the notice: the exact build script.
cp LICENSE THIRD_PARTY_NOTICES.txt "$APP/Contents/Resources/"
mkdir -p "$APP/Contents/Resources/ffmpeg"
cp scripts/build-ffmpeg.sh "$APP/Contents/Resources/ffmpeg/"

# Fail if a local path got into a binary.
for bin in "$APP/Contents/MacOS/"*; do
    if strings -a "$bin" | grep -qF -e "$HOME" -e "$PWD"; then
        echo "Local path found in $bin"; exit 1
    fi
done

# Sign by the certificate's SHA-1, so the name may not stand for two certificates.
# The signature pins this exact certificate: macOS keeps the screen-recording
# permission only while builds use the same one (see README.md, Signing).
HASHES=$(security find-identity -p codesigning | grep -F "\"$SIGN_ID\"" | awk '{print $2}' | sort -u)
if [ "$(printf '%s' "$HASHES" | grep -c .)" -gt 1 ]; then
    echo "More than one certificate \"$SIGN_ID\" in the keychain. Delete the old one:"
    echo "$HASHES"; exit 1
elif [ -n "$HASHES" ]; then
    ID="$HASHES"
    echo "==> Signing with \"$SIGN_ID\" (SHA-1 $ID)"
    CERT=$(security find-certificate -a -Z -p -c "$SIGN_ID" \
        | awk -v h="$ID" '/^SHA-1 hash:/ {keep = ($3 == h); next} keep')
    if ! printf '%s\n' "$CERT" | openssl x509 -noout -checkend 0 > /dev/null; then
        echo "The certificate \"$SIGN_ID\" has expired. See README.md, Signing."; exit 1
    elif ! printf '%s\n' "$CERT" | openssl x509 -noout -checkend $((90 * 86400)) > /dev/null; then
        echo "    WARNING: the certificate expires on $(printf '%s\n' "$CERT" | openssl x509 -noout -enddate | cut -d= -f2)."
        echo "    A new one makes every user grant the screen-recording permission again."
    fi
else
    ID="-"
    echo "==> No certificate \"$SIGN_ID\" in the keychain: signing ad hoc"
    echo "    (macOS will ask for screen-recording permission again after each build)"
fi
# Inside out: ffmpeg first (it records the microphone, so it needs the same entitlement).
for target in "$APP/Contents/MacOS/ffmpeg" "$APP"; do
    codesign --force --options runtime --timestamp=none \
        --entitlements macos/DyoSnap.entitlements --sign "$ID" "$target"
done

# Check what got signed: valid, hardened runtime on both binaries.
check_signature() {
    codesign --verify --deep --strict "$1"
    for bin in "$1/Contents/MacOS/dyosnap" "$1/Contents/MacOS/ffmpeg"; do
        info=$(codesign -dv "$bin" 2>&1)
        grep -q '^CodeDirectory.*(runtime)' <<< "$info" \
            || { echo "No hardened runtime on $bin"; exit 1; }
    done
}
check_signature "$APP"

ZIP="dist/DyoSnap-$VERSION.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
# The zip is what people get: unpack it and check the signature survived.
ditto -x -k "$ZIP" "$WORK/unzipped"
check_signature "$WORK/unzipped/DyoSnap.app"
echo "==> Done: $APP and $ZIP"
