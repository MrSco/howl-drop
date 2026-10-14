#!/bin/bash
# Assemble the Home Assistant local add-on into build/addon/owldrop: a
# self-contained folder, because Supervisor builds an add-on from its own
# directory and nothing above it.
#
#   tools/stage-addon.sh [DEST]
#
# DEST, when given, is where the folder is copied as well: the NUC's add-on
# share, e.g. //homeassistant/addons/owldrop from Git Bash on Windows
# (Samba share add-on), or /addons/owldrop from the SSH add-on. Its generated
# omdrop-owl/ and owldrop/ are replaced whole; other files are overwritten.
#
# The image builds OWL and the radio helper from this checkout (pins,
# patches/, tools/, userspace/), and owldrop-bridge from owldrop/, so what
# is edited here is what runs on the NUC.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=$ROOT/build/addon/owldrop
DEST=${1:-}

rm -rf "$OUT"
mkdir -p "$OUT/omdrop-owl/tools" "$OUT/owldrop"
cp "$ROOT"/addon/config.yaml "$ROOT"/addon/Dockerfile "$ROOT"/addon/run.sh "$ROOT"/addon/deliver.py "$ROOT"/addon/patch_receiver.py "$ROOT"/addon/README.md "$OUT/"
cp -r "$ROOT/pins" "$ROOT/patches" "$ROOT/userspace" "$OUT/omdrop-owl/"
cp "$ROOT/tools/build-owl.sh" "$ROOT/tools/stage.sh" "$OUT/omdrop-owl/tools/"
cp -r "$ROOT/owldrop/CMakeLists.txt" "$ROOT/owldrop/src" "$ROOT/owldrop/tests" "$OUT/owldrop/"

# A Windows checkout with core.autocrlf would hand the Linux image CRLF
# scripts; .gitattributes prevents it, and this catches a stray one.
if grep -rlI $'\r' "$OUT" >/dev/null 2>&1; then
  echo "CRLF line endings in: $(grep -rlI $'\r' "$OUT" | tr '\n' ' ')"
  echo "convert the source files to LF (sed -i 's/\r\$//' FILE) and stage again"
  exit 1
fi

version=$(awk -F'"' '/^version:/ {print $2}' "$OUT/config.yaml")
echo "staged $OUT (add-on $version)"

if [ -n "$DEST" ]; then
  mkdir -p "$DEST"
  # omdrop-owl/ and owldrop/ are generated here whole, so they are replaced
  # whole: a renamed patch left behind would be applied twice and break the
  # OWL build. Only in a folder that is this add-on already.
  if [ -e "$DEST/config.yaml" ] && ! grep -q '^slug: owldrop$' "$DEST/config.yaml"; then
    echo "$DEST holds another add-on (no 'slug: owldrop'); not touching it"
    exit 1
  fi
  rm -rf "$DEST/omdrop-owl" "$DEST/owldrop"
  cp -r "$OUT"/. "$DEST"/
  echo "copied to $DEST; in Home Assistant: Add-on store, menu, Check for updates, then update OwlDrop"
fi
