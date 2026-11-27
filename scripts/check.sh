#!/bin/bash
# Runs the fast checks before a push: clippy, tests and (on macOS) the
# third-party notices. The pre-push hook in .githooks/ calls this. Turn the
# hook on once per clone with:
#   git config core.hooksPath .githooks
# Skip it for one push with `git push --no-verify`.
#
# Linux is covered by CI on GitHub, not here.
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n==> %s\n' "$1"; }

step "cargo clippy"
cargo clippy --all-targets --quiet -- -D warnings

step "cargo test"
cargo test --quiet

# The notices need cargo-about and the macOS ffmpeg build, so only check them
# where both are there.
if [ "$(uname -s)" = Darwin ]; then
    step "THIRD_PARTY_NOTICES.txt"
    if ! command -v cargo-about > /dev/null || [ ! -f build/ffmpeg/BUILDCONF.txt ]; then
        echo "SKIPPED: needs cargo-about and scripts/build-ffmpeg.sh (see scripts/notices.sh)"
    else
        ./scripts/notices.sh > /dev/null
        if ! git diff --quiet -- THIRD_PARTY_NOTICES.txt; then
            echo "THIRD_PARTY_NOTICES.txt is out of date. scripts/notices.sh has updated it:"
            git diff --stat -- THIRD_PARTY_NOTICES.txt
            echo "Commit the new file and push again."
            exit 1
        fi
        echo "up to date"
    fi
fi

step "All checks passed"
