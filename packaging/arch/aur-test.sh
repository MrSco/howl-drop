#!/bin/bash
# Build PKG's AUR PKGBUILD (aur.sh) against this checkout, as CI does for
# every pull request and before a release. An archive of the working tree's
# tracked files stands in for the release tarball, which doesn't exist until
# the tag does; for omdrop-owl-bin, the prebuilt tarball packaging/package.sh
# tar makes from the working tree does. Further arguments go to makepkg.
# Leaves the package in OUT ($PWD by default), and omdrop-owl-bin's tarball
# beside it, for its release (.github/workflows/aur-release.yml).
#
#   packaging/arch/aur-test.sh PKG [makepkg args]
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
PKG=${1:?usage: packaging/arch/aur-test.sh owly|omdrop-owl|omdrop-owl-bin [makepkg args]}
shift
VERSION=$(sed -n 's/^pkgver=//p' "$HERE/$PKG/PKGBUILD")
OUT=${OUT:-$PWD}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

"$HERE/aur.sh" "$PKG" "$WORK"
if [ "$PKG" = omdrop-owl-bin ]; then
  OUT=$WORK "$ROOT/packaging/package.sh" tar
  cp "$WORK/omdrop-owl-$VERSION-x86_64.tar.gz" "$OUT/"
else
  # A commit of the working tree when there are changes; nothing when clean.
  # stash create fails quietly on stale stat info (CI's chown leaves it so),
  # so refresh the index first. Untracked files aren't in it: add them first.
  git -C "$ROOT" update-index -q --refresh || true
  TREE=$(git -C "$ROOT" stash create)
  git -C "$ROOT" archive --prefix="$PKG-$VERSION/" \
    -o "$WORK/$PKG-$VERSION.tar.gz" "${TREE:-HEAD}"
fi

# makepkg uses a source already in SRCDEST rather than downloading it.
cd "$WORK"
SRCDEST=$WORK PKGDEST=$OUT makepkg "$@"
