# Development tasks. `just` lists them.

[private]
default:
    @just --list

# Run the helper's exclusive-mode tests (command doubles, no radio)
test:
    python -m unittest tests.test_exclusive

# Assemble the Home Assistant add-on into build/addon/owldrop, and copy it to DEST if given
stage-addon DEST="":
    tools/stage-addon.sh {{DEST}}

# Build owldrop.exe and owldrop-bridge into owldrop/build
owldrop:
    cmake -S owldrop -B owldrop/build
    cmake --build owldrop/build --config Release
