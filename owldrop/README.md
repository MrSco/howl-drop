# OwlDrop for Windows

OwlDrop sends a file and clipboard text to another OwlDrop PC on the same IP network. Ethernet is enough. The program does not use a Wi-Fi card, Bluetooth, or AWDL.

```
owldrop                 start and leave the window open
owldrop peers           list PCs that are visible
owldrop send NAME FILE  send FILE to the PC named NAME
owldrop clip            send the current clipboard text
owldrop stop            stop OwlDrop on this PC
```

Received files land in `Downloads\OwlDrop`. Clipboard text replaces the PC clipboard.

A Raspberry Pi beside the iPhone runs the AirDrop radio and `owldrop-bridge`, which forwards each received file or shared text to this program. The PC does not need a Wi-Fi card, and this program does not create one. See the Pi setup notes that ship with OwlDrop.

Build with CMake and Visual Studio, or with MinGW:

```
cmake -S . -B build
cmake --build build --config Release
```

A cross-build from this tree uses `cmake/mingw-w64-x86_64.cmake`.
