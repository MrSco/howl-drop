#!/bin/sh
# Install one of this repository's packages from a build tree, the one
# install step every package runs: the AUR's PKGBUILDs, the Nix package and
# `just install`. Other distributions' packagers can run it too.
#
#   packaging/install.sh owly          owlyd, the owly CLI, and their
#                                      systemd, D-Bus, polkit,
#                                      NetworkManager and udev files
#   packaging/install.sh omdrop-owl    omdrop-core's radio hooks, on owly,
#                                      and omdrop-awdl's helper and tools
#                                      for Omdrop without omdrop-core
#   packaging/install.sh --list PKG    print the paths it would install, for
#                                      uninstalling (without DESTDIR)
#
# It builds nothing. Before it, for owly:
#
#   cargo build --release --locked -p owlyd -p owly-cli
#
# and for omdrop-owl:
#
#   cargo build --release --locked -p omdrop-owl
#
# Settings, from the environment:
#
#   DESTDIR      staging root, as in make (default: none)
#   PREFIX       /usr
#   BINDIR       $PREFIX/bin, for the owly CLI
#   LIBDIR       $PREFIX/lib. The hooks go in $LIBDIR/omdrop-owl
#   LIBEXECDIR   $LIBDIR. owlyd goes in $LIBEXECDIR/owly. With SELinux, use
#                /usr/libexec: from /usr/lib, systemd leaves owlyd in init_t,
#                which can't reach nl80211
#   DATADIR      $PREFIX/share
#   LICENSEDIR   $DATADIR/licenses/<package>
#   DOCDIR       $DATADIR/doc/<package>
#   HOOKDIR      /usr/lib/omdrop/radio, the directory omdrop-core runs
#                hooks from. It's fixed in omdrop-core, so it doesn't follow
#                PREFIX
#   HELPERDIR    /usr/lib/omdrop, where Omdrop's bash and Python version
#                finds omdrop-discoverable and send-to-peer, also fixed
#   TARGET       the cargo target directory ($CARGO_TARGET_DIR, else
#                ./target): the binaries in $TARGET/release
set -eu
umask 022

ROOT=$(cd "$(dirname "$0")/.." && pwd)
LIST=
if [ "${1:-}" = --list ]; then
  LIST=1
  shift
fi
PKG=${1:-}
case "$PKG" in
  owly | omdrop-owl) ;;
  *)
    echo "usage: packaging/install.sh [--list] owly|omdrop-owl" >&2
    exit 2
    ;;
esac

DESTDIR=${DESTDIR:-}
PREFIX=${PREFIX:-/usr}
BINDIR=${BINDIR:-$PREFIX/bin}
LIBDIR=${LIBDIR:-$PREFIX/lib}
LIBEXECDIR=${LIBEXECDIR:-$LIBDIR}
DATADIR=${DATADIR:-$PREFIX/share}
LICENSEDIR=${LICENSEDIR:-$DATADIR/licenses/$PKG}
DOCDIR=${DOCDIR:-$DATADIR/doc/$PKG}
HOOKDIR=${HOOKDIR:-/usr/lib/omdrop/radio}
HELPERDIR=${HELPERDIR:-/usr/lib/omdrop}
TARGET=${TARGET:-${CARGO_TARGET_DIR:-$ROOT/target}}
BUILT=$TARGET/release

# put MODE SRC DEST: install a file, creating its directory.
put() {
  if [ -n "$LIST" ]; then
    echo "$3"
    return
  fi
  [ -f "$2" ] || { echo "packaging/install.sh: $2 is missing; build it first" >&2; exit 1; }
  install -D -m "$1" "$2" "$DESTDIR$3"
}

# link TARGET DEST: a symlink to TARGET, an absolute path on the installed
# system.
link() {
  if [ -n "$LIST" ]; then
    echo "$2"
    return
  fi
  install -d "$DESTDIR$(dirname "$2")"
  ln -sfn "$1" "$DESTDIR$2"
}

# owlyd's unit, with ExecStart where LIBEXECDIR put it.
unit() {
  if [ -n "$LIST" ]; then
    echo "$1"
    return
  fi
  install -d "$DESTDIR$(dirname "$1")"
  sed "s|^ExecStart=/usr/lib/owly/owlyd$|ExecStart=$LIBEXECDIR/owly/owlyd|" \
    "$ROOT/packaging/owlyd.service" > "$DESTDIR$1"
  chmod 644 "$DESTDIR$1"
  grep -q "^ExecStart=$LIBEXECDIR/owly/owlyd$" "$DESTDIR$1" \
    || { echo "packaging/install.sh: owlyd.service has no ExecStart=/usr/lib/owly/owlyd to rewrite" >&2; exit 1; }
}

# omdrop-discoverable: the hooks' binary, under the name it acts on. A
# script rather than a link, because Omdrop resolves links before it runs
# the helper through pkexec, and the polkit action is for this path.
helper() {
  if [ -n "$LIST" ]; then
    echo "$1"
    return
  fi
  install -d "$DESTDIR$(dirname "$1")"
  printf '#!/bin/sh\nexec %s omdrop-discoverable "$@"\n' "$LIBDIR/omdrop-owl/omdrop-owl" > "$DESTDIR$1"
  chmod 755 "$DESTDIR$1"
}

# The helper's polkit action, bound to where HELPERDIR put it.
helper_policy() {
  if [ -n "$LIST" ]; then
    echo "$1"
    return
  fi
  install -d "$DESTDIR$(dirname "$1")"
  sed "s|>/usr/lib/omdrop/omdrop-discoverable<|>$HELPERDIR/omdrop-discoverable<|" \
    "$ROOT/packaging/io.github.t4t5.omdrop-owl.policy" > "$DESTDIR$1"
  chmod 644 "$DESTDIR$1"
  grep -q ">$HELPERDIR/omdrop-discoverable<" "$DESTDIR$1" \
    || { echo "packaging/install.sh: io.github.t4t5.omdrop-owl.policy has no /usr/lib/omdrop/omdrop-discoverable to rewrite" >&2; exit 1; }
}

case "$PKG" in
  owly)
    put 755 "$BUILT/owlyd" "$LIBEXECDIR/owly/owlyd"
    put 755 "$BUILT/owly" "$BINDIR/owly"
    unit "$LIBDIR/systemd/system/owlyd.service"
    put 644 "$ROOT/packaging/io.github.t4t5.Owly.conf" "$DATADIR/dbus-1/system.d/io.github.t4t5.Owly.conf"
    put 644 "$ROOT/packaging/io.github.t4t5.Owly.service" "$DATADIR/dbus-1/system-services/io.github.t4t5.Owly.service"
    put 644 "$ROOT/packaging/io.github.t4t5.Owly.policy" "$DATADIR/polkit-1/actions/io.github.t4t5.Owly.policy"
    put 644 "$ROOT/packaging/90-owly-unmanaged.conf" "$LIBDIR/NetworkManager/conf.d/90-owly-unmanaged.conf"
    put 644 "$ROOT/packaging/10-owly-awdl0.link" "$LIBDIR/systemd/network/10-owly-awdl0.link"
    put 644 "$ROOT/README.md" "$DOCDIR/README.md"
    put 644 "$ROOT/LICENSE" "$LICENSEDIR/LICENSE"
    ;;
  omdrop-owl)
    put 755 "$BUILT/omdrop-owl" "$LIBDIR/omdrop-owl/omdrop-owl"
    # One binary, run by each hook's name.
    for hook in probe up tick down peers; do
      link "$LIBDIR/omdrop-owl/omdrop-owl" "$HOOKDIR/$hook"
    done
    # For Omdrop without omdrop-core: omdrop-awdl's helper, and its tools
    # beside it (crates/omdrop-owl/omdrop-awdl/README.md).
    helper "$HELPERDIR/omdrop-discoverable"
    helper_policy "$DATADIR/polkit-1/actions/io.github.t4t5.omdrop-owl.policy"
    for tool in send-to-peer airdrop-send.py awdl-af-parse.py awdl_identity.py awdl_debug.py \
      ble-airdrop-adv.py awdl-airdrop-adv.py awdl-mdns-respond.py; do
      put 755 "$ROOT/crates/omdrop-owl/omdrop-awdl/$tool" "$HELPERDIR/$tool"
    done
    put 644 "$ROOT/LICENSE" "$LICENSEDIR/LICENSE"
    put 644 "$ROOT/crates/omdrop-owl/omdrop-awdl/LICENSE" "$LICENSEDIR/omdrop-awdl-LICENSE"
    ;;
esac
