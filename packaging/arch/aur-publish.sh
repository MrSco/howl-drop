#!/bin/bash
# Publish PKG's AUR PKGBUILD (aur.sh) with the release tarball's checksum
# filled in. .github/workflows/release.yml runs this once its tag exists.
# KEY is an SSH key registered with the AUR account that maintains PKG;
# without one, ssh uses its defaults. Does nothing if the AUR already has
# this.
#
#   packaging/arch/aur-publish.sh PKG [KEY]
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
PKG=${1:?usage: packaging/arch/aur-publish.sh owly|omdrop-owl|omdrop-owl-bin [KEY]}
KEY=${2:-}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# The AUR's Ed25519 host key, whose fingerprint
# (SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4) is the one published on
# aur.archlinux.org.
echo 'aur.archlinux.org ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEuBKrPzbawxA/k2g6NcyV5jmqwJ2s+zpgZGZ7tpLIcN' \
  > "$WORK/known_hosts"
GIT_SSH_COMMAND="ssh -o UserKnownHostsFile=$WORK/known_hosts -o StrictHostKeyChecking=yes"
[[ -n "$KEY" ]] && GIT_SSH_COMMAND+=" -i $KEY -o IdentitiesOnly=yes"
export GIT_SSH_COMMAND

# Cloning a package that doesn't exist yet gives an empty repository, and the
# first push creates it.
git clone --quiet "ssh://aur@aur.archlinux.org/$PKG.git" "$WORK/aur"
"$HERE/aur.sh" "$PKG" "$WORK/aur"
cd "$WORK/aur"

# Into an empty SRCDEST, so a local tarball can't stand in for the published
# one.
SRCDEST=$WORK/src updpkgsums
makepkg --printsrcinfo > .SRCINFO

git add PKGBUILD .SRCINFO "$PKG.install"
if git diff --cached --quiet; then
  echo "The AUR already has this PKGBUILD."
  exit 0
fi
VERSION=$(sed -n 's/^pkgver=//p' PKGBUILD)-$(sed -n 's/^pkgrel=//p' PKGBUILD)
git commit --quiet -m "Update to $VERSION"
# The AUR only takes master.
git push --quiet origin HEAD:master
echo "Published $PKG $VERSION to the AUR."
