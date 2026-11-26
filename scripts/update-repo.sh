#!/bin/bash
# Builds the signed apt and dnf repositories that GitHub Pages serves.
#   scripts/update-repo.sh <packages dir> <output dir>
#
# <packages dir> holds the .deb and .rpm files (any depth). <output dir> is replaced
# with the finished site: apt/, rpm/, the public key, the setup files and index.html.
#
# The GPG signing key must already be imported. Set REPO_GPG_KEY_ID to pick a key;
# otherwise the one secret key in the keyring is used.
# Needs apt-utils (apt-ftparchive), rpm (rpmsign), createrepo-c and gnupg. It runs no
# project code.
set -euo pipefail

[ $# -eq 2 ] || { echo "usage: $0 <packages dir> <output dir>"; exit 1; }
SRC=$(cd "$1" && pwd)
OUT=$2
ROOT=$(cd "$(dirname "$0")/.." && pwd)
URL="https://pschu13r.github.io/dyosnap"

for tool in apt-ftparchive rpmsign createrepo_c gpg gzip; do
    command -v "$tool" > /dev/null || { echo "$tool is missing"; exit 1; }
done

if [ -z "${REPO_GPG_KEY_ID:-}" ]; then
    REPO_GPG_KEY_ID=$(gpg --list-secret-keys --with-colons | awk -F: '$1 == "fpr" { print $10; exit }')
fi
[ -n "$REPO_GPG_KEY_ID" ] || { echo "No GPG secret key found"; exit 1; }
echo "==> Signing with key $REPO_GPG_KEY_ID"

rm -rf "$OUT"
mkdir -p "$OUT/apt/pool/main/d/dyosnap" "$OUT/rpm"
OUT=$(cd "$OUT" && pwd)

find "$SRC" -name '*.deb' -exec cp {} "$OUT/apt/pool/main/d/dyosnap/" \;
find "$SRC" -name '*.rpm' -exec cp {} "$OUT/rpm/" \;

echo "==> apt repository"
cd "$OUT/apt"
for arch in amd64 arm64; do
    dir="dists/stable/main/binary-$arch"
    mkdir -p "$dir"
    apt-ftparchive --arch "$arch" packages pool > "$dir/Packages"
    gzip -9kn "$dir/Packages"
done
apt-ftparchive \
    -o APT::FTPArchive::Release::Origin=DyoSnap \
    -o APT::FTPArchive::Release::Label=DyoSnap \
    -o APT::FTPArchive::Release::Suite=stable \
    -o APT::FTPArchive::Release::Codename=stable \
    -o APT::FTPArchive::Release::Components=main \
    -o APT::FTPArchive::Release::Architectures="amd64 arm64" \
    release dists/stable > dists/stable/Release
gpg --batch --yes --local-user "$REPO_GPG_KEY_ID" --clearsign \
    -o dists/stable/InRelease dists/stable/Release
gpg --batch --yes --local-user "$REPO_GPG_KEY_ID" --armor --detach-sign \
    -o dists/stable/Release.gpg dists/stable/Release

echo "==> dnf repository"
cd "$OUT/rpm"
for rpm in *.rpm; do
    [ -e "$rpm" ] || break
    # Replace any old signature, so a package is never signed twice.
    rpmsign --delsign "$rpm" > /dev/null
    rpmsign --addsign --define "_gpg_name $REPO_GPG_KEY_ID" "$rpm" > /dev/null
done
createrepo_c --quiet .
gpg --batch --yes --local-user "$REPO_GPG_KEY_ID" --armor --detach-sign \
    -o repodata/repomd.xml.asc repodata/repomd.xml

echo "==> Site files"
cd "$OUT"
touch .nojekyll
gpg --export --armor "$REPO_GPG_KEY_ID" > dyosnap-archive-keyring.asc
cp "$ROOT/linux/dyosnap.sources" dyosnap.sources
# The package finds the key in a file; a download from the site fetches it by URL.
sed "s|^gpgkey=.*|gpgkey=$URL/dyosnap-archive-keyring.asc|" "$ROOT/linux/dyosnap.repo" > dyosnap.repo
cat > index.html <<HTML
<!doctype html>
<meta charset="utf-8">
<title>DyoSnap packages</title>
<h1>DyoSnap packages</h1>
<p>Signed apt and dnf repositories for <a href="https://github.com/pschu13r/dyosnap">DyoSnap</a>.
Installing the .deb or .rpm from GitHub sets this up for you. To add it by hand:</p>
<h2>Ubuntu, Debian</h2>
<pre>sudo curl -fsSL $URL/dyosnap-archive-keyring.asc -o /usr/share/keyrings/dyosnap-archive-keyring.asc
sudo curl -fsSL $URL/dyosnap.sources -o /etc/apt/sources.list.d/dyosnap.sources
sudo apt update &amp;&amp; sudo apt install dyosnap</pre>
<h2>Fedora</h2>
<pre>sudo dnf config-manager addrepo --from-repofile=$URL/dyosnap.repo
sudo dnf install dyosnap</pre>
HTML
echo "==> Wrote $OUT"
