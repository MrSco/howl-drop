#!/bin/sh
# Build owly as a .deb for Debian and Ubuntu, or an .rpm for Fedora: the
# AUR's owly with its distribution's package names. Builds owlyd and the CLI
# from this checkout, stages them with packaging/install.sh, and packs that
# with dpkg-deb or rpmbuild (packaging/rpm/owly.spec). Needs no root. CI
# builds both for each owly release (.github/workflows/linux-packages.yml).
#
# tar builds owly and omdrop-owl together instead, for the AUR's
# omdrop-owl-bin: both packages' files, staged the same way, in a tarball
# for the GitHub release (.github/workflows/aur-release.yml).
#
#   packaging/package.sh deb|rpm|tar
#
# The package lands in OUT ($PWD by default). Besides Rust 1.88 or newer,
# it needs dpkg-dev for the deb, or rpm-build for the rpm.
set -eu
umask 022

ROOT=$(cd "$(dirname "$0")/.." && pwd)
FORMAT=${1:-}
case "$FORMAT" in
  deb | rpm | tar) ;;
  *)
    echo "usage: packaging/package.sh deb|rpm|tar" >&2
    exit 2
    ;;
esac
OUT=${OUT:-$PWD}
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}

if [ "$FORMAT" = tar ]; then
  VERSION=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml")
  [ -n "$VERSION" ] || { echo "packaging/package.sh: no workspace version in Cargo.toml" >&2; exit 1; }
  NAME=omdrop-owl-$VERSION-$(uname -m)

  (cd "$ROOT" && cargo build --release --locked -p owlyd -p owly-cli -p omdrop-owl)

  WORK=$(mktemp -d)
  trap 'rm -rf "$WORK"' EXIT
  STAGE=$WORK/$NAME
  unset PREFIX BINDIR LIBDIR LIBEXECDIR DATADIR LICENSEDIR DOCDIR HOOKDIR HELPERDIR
  # Arch keeps a package's licenses under its name.
  export LICENSEDIR=/usr/share/licenses/omdrop-owl-bin
  for pkg in owly omdrop-owl; do
    DESTDIR=$STAGE TARGET=$TARGET "$ROOT/packaging/install.sh" "$pkg"
  done
  strip --strip-unneeded "$STAGE/usr/lib/owly/owlyd" "$STAGE/usr/bin/owly" \
    "$STAGE/usr/lib/omdrop-owl/omdrop-owl"

  PKG=$OUT/$NAME.tar.gz
  tar -C "$WORK" --owner=0 --group=0 --numeric-owner -czf "$PKG" "$NAME"
  echo "built $PKG"
  exit 0
fi

# The workspace version, as the AUR's pkgver is, and the packaging's own
# revision, for a change to the packaging alone.
VERSION=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml")
RELEASE=1
[ -n "$VERSION" ] || { echo "packaging/package.sh: no workspace version in Cargo.toml" >&2; exit 1; }

(cd "$ROOT" && cargo build --release --locked -p owlyd -p owly-cli)

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
STAGE=$WORK/root
# install.sh's default paths, whatever the environment sets. Debian keeps a
# package's license with its docs, and Fedora, with SELinux on, needs owlyd
# in /usr/libexec (install.sh, LIBEXECDIR).
unset PREFIX BINDIR LIBDIR LIBEXECDIR DATADIR LICENSEDIR DOCDIR HOOKDIR
if [ "$FORMAT" = deb ]; then
  export LICENSEDIR=/usr/share/doc/owly
else
  export LIBEXECDIR=/usr/libexec
fi
DESTDIR=$STAGE TARGET=$TARGET "$ROOT/packaging/install.sh" owly
strip --strip-unneeded "$STAGE${LIBEXECDIR:-/usr/lib}/owly/owlyd" "$STAGE/usr/bin/owly"

if [ "$FORMAT" = deb ]; then
  ARCH=$(dpkg --print-architecture)
  install -d "$STAGE/DEBIAN"
  install -m 755 "$ROOT/packaging/deb/postinst" "$ROOT/packaging/deb/prerm" \
    "$ROOT/packaging/deb/postrm" "$STAGE/DEBIAN/"
  # The libraries the binaries link, as dpkg-shlibdeps finds them. It wants
  # a debian/control to read, with only the source stanza, and DEBIAN/ to
  # tell where the package's root is.
  mkdir "$WORK/debian"
  printf 'Source: owly\n\nPackage: owly\nArchitecture: any\n' > "$WORK/debian/control"
  SHLIBS=$(cd "$WORK" && dpkg-shlibdeps -O "$STAGE/usr/lib/owly/owlyd" "$STAGE/usr/bin/owly" \
    | sed -n 's/^shlibs:Depends=//p')
  [ -n "$SHLIBS" ] || { echo "packaging/package.sh: dpkg-shlibdeps found no libraries" >&2; exit 1; }

  # The AUR's depends, by Debian's names: see packaging/arch/owly/PKGBUILD
  # for what each is for.
  cat > "$STAGE/DEBIAN/control" <<EOF
Package: owly
Version: $VERSION-$RELEASE
Architecture: $ARCH
Maintainer: T4T5 <t4t5@hey.com>
Installed-Size: $(du -sk --exclude=DEBIAN "$STAGE" | cut -f1)
Depends: $SHLIBS, dbus, systemd, polkitd, network-manager, iproute2, iputils-ping, mount
Recommends: hostapd
Suggests: bluez
Section: net
Priority: optional
Homepage: https://github.com/t4t5/owly
Description: AirDrop's Wi-Fi radio on Linux
 AWDL in userspace, for Wi-Fi cards without Apple's Broadcom firmware:
 owlyd, the system service that leases the radio out over D-Bus, and the
 owly CLI: doctor, test-card, send and receive.
EOF
  PKG=$OUT/owly_$VERSION-${RELEASE}_$ARCH.deb
  dpkg-deb --root-owner-group -Zxz --build "$STAGE" "$PKG" > /dev/null
else
  ARCH=$(uname -m)
  # %files: what install.sh installs, and the directories that are owly's.
  FILES=$WORK/files
  {
    "$ROOT/packaging/install.sh" --list owly | xargs -n1 dirname | sort -u \
      | grep '/owly$' | sed 's/^/%dir /'
    "$ROOT/packaging/install.sh" --list owly
  } > "$FILES"
  rpmbuild -bb --quiet \
    --define "_topdir $WORK/rpmbuild" \
    --define "owly_version $VERSION" \
    --define "owly_release $RELEASE" \
    --define "owly_stage $STAGE" \
    --define "owly_files $FILES" \
    "$ROOT/packaging/rpm/owly.spec"
  PKG=$OUT/owly-$VERSION-$RELEASE.$ARCH.rpm
  cp "$WORK/rpmbuild/RPMS/$ARCH/owly-$VERSION-$RELEASE.$ARCH.rpm" "$PKG"
fi
echo "built $PKG"
