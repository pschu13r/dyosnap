#!/bin/bash
# Writes THIRD_PARTY_NOTICES.txt: the licenses of everything DyoSnap.app ships
# besides its own code (ffmpeg and the Rust crates). bundle.sh runs this and puts
# the result into the app. Run it again after changing Cargo.lock or ffmpeg.
#
# Needs cargo-about (cargo install --locked --features cli cargo-about) and the
# ffmpeg build from scripts/build-ffmpeg.sh.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=THIRD_PARTY_NOTICES.txt
BUILDCONF=build/ffmpeg/BUILDCONF.txt
FF_VERSION=$(sed -n 's/^VERSION=//p' scripts/build-ffmpeg.sh)
FF_SHA256=$(sed -n 's/^SHA256=//p' scripts/build-ffmpeg.sh)
FF_LICENSE="build/ffmpeg-$FF_VERSION/COPYING.LGPLv2.1"
FF_IJG="build/ffmpeg-$FF_VERSION/libavcodec/jrevdct.c"

command -v cargo-about > /dev/null \
    || { echo "cargo-about is missing: cargo install --locked --features cli cargo-about"; exit 1; }
for f in "$BUILDCONF" "$FF_LICENSE" "$FF_IJG"; do
    [ -f "$f" ] || { echo "$f is missing: run scripts/build-ffmpeg.sh first"; exit 1; }
done

RUST=$(mktemp)
trap 'rm -f "$RUST" "$OUT.part"' EXIT
cargo about generate --fail about.hbs -o "$RUST"

{
cat <<TXT
THIRD-PARTY NOTICES FOR DYOSNAP

DyoSnap itself is under the MIT license (see LICENSE). It ships with the
third-party software listed below, each under its own license.

  1. FFmpeg $FF_VERSION (LGPL 2.1 or later), as Contents/MacOS/ffmpeg
  2. Rust crates compiled into Contents/MacOS/dyosnap

================================================================================
1. FFMPEG
================================================================================

DyoSnap.app contains a build of FFmpeg (https://ffmpeg.org), a separate
program that DyoSnap starts to record, play, trim and convert video.
FFmpeg is copyright (c) the FFmpeg developers and is licensed under the GNU
Lesser General Public License version 2.1 or later. The full license text
follows at the end of this section.

This build contains no GPL or nonfree parts (no --enable-gpl, no
--enable-nonfree, no x264). Video is encoded with Apple's VideoToolbox,
audio with Apple's AudioToolbox.

Source code
-----------
  FFmpeg $FF_VERSION, official release tarball:
    https://ffmpeg.org/releases/ffmpeg-$FF_VERSION.tar.xz
    SHA-256: $FF_SHA256
    Signed by the FFmpeg release key (fingerprint
    FCF9 86EA 15E6 E293 A564 4F10 B432 2F04 D676 58D8):
    https://ffmpeg.org/releases/ffmpeg-$FF_VERSION.tar.xz.asc
  The same tarball and its signature are attached to every DyoSnap release:
    https://github.com/pschu13r/dyosnap/releases
  The patch and build script are also in the DyoSnap source:
    https://github.com/pschu13r/dyosnap

Changes by DyoSnap
--------------------
  One patch is applied to the source before building:
    ffmpeg-avfoundation-audio-queue.patch
    (changes libavdevice/avfoundation.m: queue audio frames from the
    microphone instead of overwriting the one not read yet)
  The patch and the build script (build-ffmpeg.sh) are inside the app at
    DyoSnap.app/Contents/Resources/ffmpeg/

Build
-----
  build-ffmpeg.sh downloads the tarball above, checks it, applies the patch
  and builds with the following options (from "ffmpeg -buildconf"):

TXT
grep -E '^ +--' "$BUILDCONF" | sed 's/^ */    /'
cat <<'TXT'

  Built for macOS 13 or later on Apple Silicon (arm64), linked statically
  against FFmpeg's own libraries and dynamically only against macOS system
  frameworks.

Replacing FFmpeg
----------------
  You may change and rebuild FFmpeg and use your build with DyoSnap:
  build it with build-ffmpeg.sh (or your own options), then replace
  DyoSnap.app/Contents/MacOS/ffmpeg with your binary and sign the app
  again, for example ad hoc:
    codesign --force --deep --sign - DyoSnap.app

Independent JPEG Group
----------------------
  This software is based in part on the work of the Independent JPEG Group.
  (FFmpeg's files libavcodec/jfdctfst.c, libavcodec/jfdctint_template.c and
  libavcodec/jrevdct.c are taken from libjpeg; DyoSnap does not change
  them.)

GNU Lesser General Public License, version 2.1
----------------------------------------------

TXT
cat "$FF_LICENSE"
cat <<'TXT'

================================================================================
2. RUST CRATES
================================================================================

The DyoSnap program (Contents/MacOS/dyosnap) is compiled together with
the Rust crates below. Each license text is listed once, followed by the
crates it applies to. Where a crate offers a choice of licenses, DyoSnap
uses it under the license it is listed with. The source code of every crate
is available at the crates.io link given for it.
TXT
cat "$RUST"
} > "$OUT.part"

mv "$OUT.part" "$OUT"
echo "==> Wrote $OUT ($(wc -l < "$OUT" | tr -d ' ') lines)"
