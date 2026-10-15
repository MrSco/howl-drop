# HowlDrop for Windows

`howldrop.exe` gets what an iPhone AirDrops to the HowlDrop Home Assistant
add-on (with the add-on's `forward_to_pc` on), and sends files and clipboard
text between HowlDrop PCs on the same network. Ethernet is enough: the PC
needs no Wi-Fi card, Bluetooth or AWDL. The add-on (`addon/` in this repo)
runs the AirDrop radio and `howldrop-bridge`, which forwards each file or
shared text to this program.

## The tray app

Start `howldrop.exe` (double-click it). It runs by the clock, with no window:

- A notification for each file and each text that arrives. Click it to open
  what arrived (a link opens in the browser; anything else that is not
  opened automatically is shown in its folder).
- Files land in `Downloads\HowlDrop`. Shared text goes on the clipboard.
- Click the icon to open `Downloads\HowlDrop`. Right-click it for the menu:

| Menu item | Default | |
|---|---|---|
| Show notifications | on | |
| Open links in the browser | on | A shared web page opens, as on a Mac. Only `http` and `https` addresses. |
| Open photos and videos | off | Opens each one in your photo or video app. |
| Open PDFs and text files | off | `.pdf` and `.txt` only. |
| Start with Windows | off | Runs HowlDrop when you sign in. |
| Quit HowlDrop | | |

Nothing that runs is ever opened by itself, whatever its name: programs,
scripts, installers, shortcuts to files and archives only notify, and
clicking the notification shows them in their folder. The settings are kept
in `%LOCALAPPDATA%\HowlDrop\settings.ini`, the log in
`%LOCALAPPDATA%\HowlDrop\howldrop.log` (started over past 1 MB).

Windows asks once whether to let `howldrop.exe` through the firewall: allow
it on private networks (UDP 47631, TCP 47632).

## Commands

In a terminal, the same program takes commands:

```
howldrop                 start in the tray
howldrop serve           start in this window instead, with its log
howldrop peers           list PCs that are visible
howldrop send NAME FILE  send FILE to the PC named NAME
howldrop clip            send the current clipboard text
howldrop stop            stop HowlDrop on this PC (the tray icon too)
```

## Building

With CMake, Ninja and a MinGW-w64 g++ (Strawberry Perl ships all three):

```powershell
tools\build-howldrop.ps1 -Run
```

Or with Visual Studio:

```
cmake -S . -B build
cmake --build build --config Release
```

A cross-build from Linux uses `cmake/mingw-w64-x86_64.cmake`. CI builds
`howldrop.exe` on every push; it is the `howldrop-windows` download on the
run's page.

`tests/` runs on Linux (`howldrop-test`, `howldrop-bridge-test`) and on
both (`howldrop-open-test`, the rules for what opens by itself, in
`src/open_policy.cpp`).
