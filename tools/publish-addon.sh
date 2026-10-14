#!/bin/bash
# Commit the staged add-on (tools/stage-addon.sh) to the ha-addon branch: a
# Home Assistant add-on repository, which users add as
#
#   https://github.com/MrSco/howl-drop#ha-addon
#
# Supervisor builds an add-on from its own folder only, and on main the add-on
# needs pins, patches/, userspace/ and howldrop/ from beside it; the branch
# holds the assembled folder instead. CI runs this after the tests pass on
# main. Each run adds one commit when the add-on changed, none otherwise.
#
#   tools/publish-addon.sh [--no-push]
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
BRANCH=ha-addon
STAGED=$ROOT/build/addon/howldrop
PUSH=1
[ "${1:-}" = --no-push ] && PUSH=0

[ -f "$STAGED/config.yaml" ] || { echo "nothing staged at $STAGED; run tools/stage-addon.sh first"; exit 1; }
version=$(awk -F'"' '/^version:/ {print $2}' "$STAGED/config.yaml")
source_rev=$(git -C "$ROOT" rev-parse --short HEAD)

WORK=$(mktemp -d)
trap 'git -C "$ROOT" worktree remove --force "$WORK" 2>/dev/null || true' EXIT
if git -C "$ROOT" fetch -q origin "$BRANCH" 2>/dev/null; then
  git -C "$ROOT" worktree add -q -B "$BRANCH" "$WORK" FETCH_HEAD
else
  git -C "$ROOT" worktree add -q --orphan -b "$BRANCH" "$WORK"
fi

# The branch is generated: replace everything but .git.
find "$WORK" -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
cp -r "$STAGED" "$WORK/howldrop"
cp "$ROOT/tools/ha-repository.yaml" "$WORK/repository.yaml"
cat > "$WORK/README.md" <<EOF
# HowlDrop add-on repository

Generated from [main](https://github.com/MrSco/howl-drop) by
\`tools/publish-addon.sh\`; don't edit this branch. In Home Assistant: Settings,
Add-ons, Add-on store, menu, Repositories, add
\`https://github.com/MrSco/howl-drop#ha-addon\`, then install HowlDrop.

HowlDrop $version, from $source_rev.
EOF

git -C "$WORK" add -A
if git -C "$WORK" diff --cached --quiet; then
  echo "ha-addon already holds HowlDrop $version from these sources"
  exit 0
fi
git -C "$WORK" -c user.name='github-actions[bot]' \
  -c user.email='41898282+github-actions[bot]@users.noreply.github.com' \
  commit -q -m "HowlDrop $version from $source_rev"
echo "committed HowlDrop $version ($source_rev) to $BRANCH"
if [ "$PUSH" = 1 ]; then
  git -C "$WORK" push -q origin "$BRANCH"
  echo "pushed $BRANCH"
fi
