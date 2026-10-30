#!/bin/bash
# Take in a new Owly (docs/fork.md): fetch upstream, merge upstream/main into
# the current branch, and run the Rust and Python tests. Run it on a branch,
# on Linux (or WSL, from a copy on the Linux file system).
#
#   tools/sync-upstream.sh
#
# A merge that stops on a conflict leaves it to resolve: keep upstream's
# change, put the fork's patch back on top, `git merge --continue`, and run
# this again to test.
set -euo pipefail
cd "$(dirname "$0")/.."

UPSTREAM=https://github.com/t4t5/owly.git

if [ "$(git rev-parse --abbrev-ref HEAD)" = main ]; then
    echo "On main: make a branch for the merge first (git switch -c owly-sync)." >&2
    exit 1
fi
git remote get-url upstream >/dev/null 2>&1 || git remote add upstream "$UPSTREAM"
git fetch upstream

if git merge-base --is-ancestor upstream/main HEAD; then
    echo "Already up to date with upstream/main ($(git rev-parse --short upstream/main))."
else
    echo "Merging upstream/main ($(git rev-parse --short upstream/main)):"
    git log --oneline HEAD..upstream/main | head -20
    git merge --no-edit upstream/main
fi

cargo test --workspace
python3 -m unittest discover -s tests
echo "Merged and tested. Changes to upstream's files: git diff upstream/main HEAD -- \$(git ls-tree -r --name-only upstream/main)"
