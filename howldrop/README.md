# HowlDrop for Windows

HowlDrop sends a file and clipboard text to another HowlDrop PC on the same IP network. Ethernet is enough. The program does not use a Wi-Fi card, Bluetooth, or AWDL.

```
howldrop                 start and leave the window open
howldrop peers           list PCs that are visible
howldrop send NAME FILE  send FILE to the PC named NAME
howldrop clip            send the current clipboard text
howldrop stop            stop HowlDrop on this PC
```

Received files land in `Downloads\HowlDrop`. Clipboard text replaces the PC clipboard.

The HowlDrop Home Assistant add-on (`addon/` in this repo) runs the AirDrop radio and `howldrop-bridge`, which forwards each file or shared text an iPhone sends to this program. The PC does not need a Wi-Fi card, and this program does not create one.

Build with CMake and Visual Studio, or with MinGW:

```
cmake -S . -B build
cmake --build build --config Release
```

A cross-build from this tree uses `cmake/mingw-w64-x86_64.cmake`.
