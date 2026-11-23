#!/bin/bash
# Builds the Linux packages on Linux:
#   dist/DyoSnap-<version>-linux-<arch>.tar.gz: the program, its .desktop file and icon,
#     LICENSE and the license notices of the Rust crates in it
#   dist/dyosnap_<version>-1_<arch>.deb and dist/dyosnap-<version>-1.<arch>.rpm: the same
#     files, installed to /usr (see [package.metadata.deb] and [...generate-rpm] in Cargo.toml)
#
# Needs Rust, cargo-about (cargo install --locked --features cli cargo-about), cargo-deb,
# cargo-generate-rpm (cargo install --locked cargo-deb cargo-generate-rpm), dpkg-dev and the
# development files of PipeWire, Wayland, xkbcommon and D-Bus plus clang (see README).
# ffmpeg is not bundled: DyoSnap uses the system's.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(uname -m)
TARGET="$ARCH-unknown-linux-gnu"
NAME="DyoSnap-$VERSION-linux-$ARCH"
STAGE="dist/linux/$NAME"
BIN="${CARGO_TARGET_DIR:-target}/release/dyosnap"

command -v cargo-about > /dev/null \
    || { echo "cargo-about is missing: cargo install --locked --features cli cargo-about"; exit 1; }
for tool in cargo-deb cargo-generate-rpm; do
    command -v "$tool" > /dev/null || { echo "$tool is missing: cargo install --locked $tool"; exit 1; }
done

# Keep local paths out of the binary, as bundle.sh does on macOS.
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
CARGO_ENCODED_RUSTFLAGS=$(printf '%s\x1f' \
    "--remap-path-prefix=$HOME=/home" \
    "--remap-path-prefix=$RUSTUP_HOME=/rustup" \
    "--remap-path-prefix=$CARGO_HOME=/cargo" \
    "--remap-path-prefix=$PWD=/dyosnap")
export CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS%$'\x1f'}"

echo "==> Building DyoSnap $VERSION ($TARGET)"
cargo build --release

echo "==> Checking the binary for local paths"
if strings -a "$BIN" | grep -qF -e "$HOME" -e "$PWD"; then
    echo "A local path is in $BIN:"
    strings -a "$BIN" | grep -F -e "$HOME" -e "$PWD" | head -5
    exit 1
fi

echo "==> Writing the license notices"
rm -rf "$STAGE"
mkdir -p "$STAGE"
RUST=$(mktemp)
trap 'rm -f "$RUST"' EXIT
cargo about generate --fail --target "$TARGET" about.hbs -o "$RUST"
{
cat <<'TXT'
THIRD-PARTY NOTICES FOR DYOSNAP (LINUX)

DyoSnap itself is under the MIT license (see LICENSE). The program
(dyosnap) is compiled together with the Rust crates below. Each license
text is listed once, followed by the crates it applies to. Where a crate
offers a choice of licenses, DyoSnap uses it under the license it is
listed with. The source code of every crate is available at the crates.io
link given for it.

DyoSnap does not include FFmpeg on Linux; it uses the one installed on
the system.
TXT
cat "$RUST"
} > "$STAGE/THIRD_PARTY_NOTICES.txt"

echo "==> Packing $NAME"
install -m 755 "$BIN" "$STAGE/dyosnap"
cp LICENSE linux/io.github.pschu13r.DyoSnap.desktop linux/io.github.pschu13r.DyoSnap.png "$STAGE/"
tar -czf "dist/$NAME.tar.gz" -C dist/linux "$NAME"
echo "==> Wrote dist/$NAME.tar.gz"

# The .deb and .rpm take the notices from a fixed path (see Cargo.toml).
cp "$STAGE/THIRD_PARTY_NOTICES.txt" dist/linux/THIRD_PARTY_NOTICES.txt

echo "==> Packing the .deb"
# --no-build: use the binary built above, without local paths. cargo-deb would strip again.
cargo deb --no-build --no-strip --output dist/

echo "==> Packing the .rpm"
cargo generate-rpm --output dist/
