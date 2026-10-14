# Development tasks. `just` lists them.

[private]
default:
    @just --list

# Run the helper's exclusive-mode tests (command doubles, no radio)
test:
    python -m unittest tests.test_exclusive

# Assemble the Home Assistant add-on into build/addon/howldrop, and copy it to DEST if given
stage-addon DEST="":
    tools/stage-addon.sh {{DEST}}

# Build howldrop.exe and howldrop-bridge into howldrop/build-win (Windows: tools/build-howldrop.ps1)
howldrop:
    cmake -S howldrop -B howldrop/build-win -G Ninja -DCMAKE_BUILD_TYPE=Release
    cmake --build howldrop/build-win
