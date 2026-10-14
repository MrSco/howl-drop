#!/bin/bash
# Assemble the Home Assistant local add-on into build/addon/howldrop: a
# self-contained folder, because Supervisor builds an add-on from its own
# directory and nothing above it.
#
#   tools/stage-addon.sh [DEST]
#
# tools/publish-addon.sh commits this folder to the ha-addon branch, the
# add-on repository Home Assistant installs from.
#
# DEST, when given, is where the folder is copied as well: Home Assistant's add-on
# share, e.g. //homeassistant/addons/howldrop from Git Bash on Windows
# (Samba share add-on), or /addons/howldrop from the SSH add-on. Its generated
# omdrop-owl/ and howldrop/ are replaced whole; other files are overwritten.
#
# The image builds OWL and the radio helper from this checkout (pins,
# patches/, tools/, userspace/), and howldrop-bridge from howldrop/, so what
# is edited here is what runs in Home Assistant.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=$ROOT/build/addon/howldrop
DEST=${1:-}

rm -rf "$OUT"
mkdir -p "$OUT/omdrop-owl/tools" "$OUT/howldrop"
cp "$ROOT"/addon/config.yaml "$ROOT"/addon/Dockerfile "$ROOT"/addon/run.sh "$ROOT"/addon/deliver.py "$ROOT"/addon/senders.py "$ROOT"/addon/patch_receiver.py "$ROOT"/addon/README.md "$ROOT"/addon/icon.png "$ROOT"/addon/logo.png "$OUT/"
cp -r "$ROOT/addon/translations" "$OUT/"
cp -r "$ROOT/pins" "$ROOT/patches" "$ROOT/userspace" "$OUT/omdrop-owl/"
cp "$ROOT/tools/build-owl.sh" "$ROOT/tools/stage.sh" "$OUT/omdrop-owl/tools/"
cp -r "$ROOT/howldrop/CMakeLists.txt" "$ROOT/howldrop/src" "$ROOT/howldrop/tests" "$OUT/howldrop/"

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
  # omdrop-owl/ and howldrop/ are generated here whole, so they are replaced
  # whole: a renamed patch left behind would be applied twice and break the
  # OWL build. Only in a folder that is this add-on already.
  if [ -e "$DEST/config.yaml" ] && ! grep -q '^slug: howldrop$' "$DEST/config.yaml"; then
    echo "$DEST holds another add-on (no 'slug: howldrop'); not touching it"
    exit 1
  fi
  rm -rf "$DEST/omdrop-owl" "$DEST/howldrop"
  cp -r "$OUT"/. "$DEST"/
  echo "copied to $DEST; in Home Assistant: Add-on store, menu, Check for updates, then update HowlDrop"
fi
