#!/bin/bash
# Lints the Linux packages that scripts/bundle-linux.sh built:
#   lintian for the .deb, rpmlint for the .rpm. rpmlint runs in a Fedora container, so it uses
#   Fedora's rules (Ubuntu's rpmlint has outdated ones, e.g. it does not know SPDX "MIT").
# linux.yml runs this after every package build. Locally it needs lintian and podman or docker.
# Usage: scripts/lint-packages.sh [folder with the packages, default dist]
#
# Left out on purpose:
#   no-changelog, no-changelogname-tag: no Debian/RPM changelog; release notes are on GitHub.
#   package-installs-apt-sources, non-conffile-in-etc: the packages set up the DyoSnap apt/dnf
#     repository and its key on purpose (see Cargo.toml). The key is no config file, so a new
#     key always replaces the old one.
#   no-manual-page, no-manual-page-for-binary: a desktop app, started from the app menu.
#   no-buildhost-tag: cargo-generate-rpm leaves it out.
#   unknown-key: release .rpm files are signed with the repository key, which the container
#     does not know. CI lints them before signing.
set -euo pipefail
cd "$(dirname "$0")/.."
DIR="${1:-dist}"

shopt -s nullglob
debs=("$DIR"/*.deb)
rpms=("$DIR"/*.rpm)
if [ ${#debs[@]} -eq 0 ] || [ ${#rpms[@]} -eq 0 ]; then
    echo "No .deb or .rpm in $DIR: run scripts/bundle-linux.sh first"
    exit 1
fi

echo "==> lintian"
lintian --fail-on error,warning \
    --suppress-tags no-changelog,package-installs-apt-sources,no-manual-page \
    "${debs[@]}"

echo "==> rpmlint (Fedora rules)"
ENGINE=$(command -v podman || command -v docker) \
    || { echo "rpmlint runs in a container: install podman or docker"; exit 1; }
"$ENGINE" run --rm -v "$PWD/$DIR:/p:ro,z" registry.fedoraproject.org/fedora:43 bash -c '
    set -e
    dnf -q -y install rpmlint > /dev/null 2>&1
    cat > /tmp/dyosnap.toml <<TOML
Filters = [
    "no-changelogname-tag",
    "non-conffile-in-etc /etc/pki/rpm-gpg/RPM-GPG-KEY-dyosnap",
    "no-manual-page-for-binary",
    "no-buildhost-tag",
    "unknown-key",
]
TOML
    rpmlint --strict --config /tmp/dyosnap.toml /p/*.rpm
'

echo "==> Packages are fine"
