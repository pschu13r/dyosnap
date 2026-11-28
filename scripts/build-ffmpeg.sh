#!/bin/bash
# Builds the ffmpeg that ships inside DyoSnap.app, from the official source.
#
# License: LGPL 2.1 or later. No --enable-gpl, no --enable-nonfree, no x264.
# Video is encoded with Apple's h264_videotoolbox, audio with aac_at.
# The source is built unchanged, without patches.
# Only what DyoSnap uses is built in: screen capture (avfoundation), sound output
# for the player (audiotoolbox), mp4/mov/gif in and out, a handful of filters,
# and local files only (no network).
#
# Result: build/ffmpeg/bin/ffmpeg (Apple Silicon, macOS 13 or later).
# The source tarball stays in build/ so it can be passed on with the app (LGPL).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=9.1
SHA256=1a4d394ae63d190a526cd052eb4415c156b5c5e2df37efab41a7de0b8fde2433
# FFmpeg release signing key <ffmpeg-devel@ffmpeg.org>
KEY_FPR=FCF986EA15E6E293A5644F10B4322F04D67658D8

ROOT="$PWD/build"
TARBALL="$ROOT/ffmpeg-$VERSION.tar.xz"
SRC="$ROOT/ffmpeg-$VERSION"
OUT="$ROOT/ffmpeg"
mkdir -p "$ROOT"

if [ ! -f "$TARBALL" ]; then
    echo "==> Downloading ffmpeg $VERSION"
    curl -fL --proto '=https' -o "$TARBALL.part" "https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz"
    curl -fL --proto '=https' -o "$TARBALL.asc" "https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz.asc"
    mv "$TARBALL.part" "$TARBALL"
fi

echo "==> Checking the source"
if [ "$SHA256" != "__PIN_ME__" ]; then
    echo "$SHA256  $TARBALL" | shasum -a 256 -c -
else
    # First run: check the release signature with a throwaway keyring, then pin the hash.
    GNUPGHOME=$(mktemp -d)
    export GNUPGHOME
    trap 'rm -rf "$GNUPGHOME"' EXIT
    curl -fsSL --proto '=https' https://ffmpeg.org/ffmpeg-devel.asc | gpg --quiet --import
    gpg --quiet --verify --status-fd 1 "$TARBALL.asc" "$TARBALL" | grep -q "VALIDSIG $KEY_FPR" \
        || { echo "Signature check failed"; exit 1; }
    echo "    Signature OK. Pin this in scripts/build-ffmpeg.sh:"
    echo "    SHA256=$(shasum -a 256 "$TARBALL" | cut -d' ' -f1)"
fi

rm -rf "$SRC" "$OUT"
tar -xJf "$TARBALL" -C "$ROOT"
cd "$SRC"

echo "==> Configuring"
./configure \
    --prefix=/usr/local \
    --arch=arm64 --target-os=darwin --cc=clang \
    --extra-cflags="-mmacosx-version-min=13.0" \
    --extra-ldflags="-mmacosx-version-min=13.0" \
    --disable-everything --disable-autodetect \
    --disable-doc --disable-ffplay --disable-ffprobe --disable-network \
    --disable-debug --enable-static --disable-shared \
    --enable-avfoundation --enable-videotoolbox --enable-audiotoolbox \
    --enable-indev=avfoundation --enable-outdev=audiotoolbox \
    --enable-protocol=file,pipe \
    --enable-demuxer=mov,concat,gif \
    --enable-muxer=mp4,mov,gif,rawvideo \
    --enable-decoder='h264,hevc,prores,mpeg4,aac,mp3,alac,pcm*,rawvideo,gif' \
    --enable-hwaccel=h264_videotoolbox,hevc_videotoolbox \
    --enable-encoder=h264_videotoolbox,aac_at,gif,rawvideo,pcm_s16le,pcm_f32le \
    --enable-parser=h264,hevc,aac,mpegaudio,gif \
    --enable-bsfs \
    --enable-filter='buffer,buffersink,abuffer,abuffersink,format,aformat,null,anull,crop,scale,fps,split,palettegen,paletteuse,aresample,asetnsamples,apad,trim,atrim,setpts,asetpts,copy,acopy' \
    > "$ROOT/ffmpeg-configure.log"

echo "==> Building (takes a few minutes)"
make -j"$(sysctl -n hw.ncpu)" > "$ROOT/ffmpeg-make.log" 2>&1 || { tail -30 "$ROOT/ffmpeg-make.log"; exit 1; }
# Only the program is needed. No "make install": the prefix is a neutral placeholder,
# because ffmpeg stores it in the binary (-buildconf) and a local path does not belong there.
mkdir -p "$OUT/bin"
install -m 755 ffmpeg "$OUT/bin/ffmpeg"

# Keep the exact configure line for the license notice.
"$OUT/bin/ffmpeg" -hide_banner -buildconf > "$OUT/BUILDCONF.txt"
if grep -qE "enable-(gpl|nonfree)" "$OUT/BUILDCONF.txt"; then
    echo "GPL or nonfree code got in"; exit 1
fi
echo "==> Done: $OUT/bin/ffmpeg ($("$OUT/bin/ffmpeg" -hide_banner -version | head -1))"
