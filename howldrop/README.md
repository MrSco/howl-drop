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
| Copy pairing key | | For the add-on's `pc_key` (below). |
| Share what I copy with my other HowlDrop PCs | off | Sends each copied text to your other paired PCs. |
| Start with Windows | off | Runs HowlDrop when you sign in. |
| Quit HowlDrop | | |

Nothing that runs is ever opened by itself, whatever its name: programs,
scripts, installers, shortcuts to files and archives only notify, and
clicking the notification shows them in their folder. The settings are kept
in `%LOCALAPPDATA%\HowlDrop\settings.ini`, the log in
`%LOCALAPPDATA%\HowlDrop\howldrop.log` (started over past 1 MB).

Windows asks once whether to let `howldrop.exe` through the firewall: allow
it on private networks (UDP 47631, TCP 47632).

## Pairing

`howldrop.exe` makes a pairing key the first time it runs (128 random bits,
in `%LOCALAPPDATA%\HowlDrop\pair.key`) and accepts files and text only from
a sender that knows it. Someone else on your network cannot drop files on
the PC, set its clipboard or open pages in its browser. To pair the add-on,
choose **Copy pairing key** from the menu, paste it into the add-on's
`pc_key` option, and restart the add-on. HowlDrop notifies you when it turns
an unpaired sender away.

It works both ways: before sending a byte, the add-on checks that the PC
proves it knows the key, so a machine pretending to be your PC learns
nothing. Each transfer is signed (HMAC-SHA256 with fresh random values from
both ends), so a changed or replayed one is dropped.

Two HowlDrop PCs that send to each other need the same key: run
`howldrop key` on one and `howldrop pair KEY` on the other, then restart
HowlDrop there.

What you copy on the PC stays there unless you turn on sharing, and even
then it goes only to paired PCs.

## Commands

In a terminal, the same program takes commands:

```
howldrop                 start in the tray
howldrop serve           start in this window instead, with its log
howldrop peers           list PCs that are visible
howldrop send NAME FILE  send FILE to the PC named NAME
howldrop clip            send the current clipboard text
howldrop stop            stop HowlDrop on this PC (the tray icon too)
howldrop key             show this PC's pairing key
howldrop pair KEY        take another PC's key (then restart HowlDrop)
howldrop serve --share-clipboard   serve, sending what you copy to paired PCs
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
