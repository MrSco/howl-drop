# HowlDrop for Windows

`howldrop.exe` gets what an iPhone AirDrops, and AirDrops to it, two ways:

- **AirDrop on this PC**: with a USB MT7612U Wi-Fi stick in the PC, the PC
  itself is on the iPhone's share sheet. No Home Assistant (below).
- **Through Home Assistant**: the HowlDrop add-on (`addon/` in this repo)
  runs the radio and forwards each file or text here (its `forward_to_pc`).
  Ethernet is enough: the PC needs no Wi-Fi card or Bluetooth.

It also sends files and clipboard text between HowlDrop PCs on the same
network.

Download `howldrop.exe` from
[Releases](https://github.com/MrSco/howl-drop/releases/latest). It is one
file with nothing to install; put it somewhere it can stay (say
`%LOCALAPPDATA%\Programs\HowlDrop`), since **Start with Windows** and the
firewall rule remember its path.

## The tray app

Start `howldrop.exe` (double-click it). It runs by the clock:

<p>
  <img src="../docs/screenshots/window.png" alt="The HowlDrop window" width="300">
  &nbsp;
  <img src="../docs/screenshots/card-ask.png" alt="An incoming AirDrop asking to be accepted" width="280">
  <img src="../docs/screenshots/card-received.png" alt="A received video, with Open and Show in folder" width="280">
  <img src="../docs/screenshots/card-sending.png" alt="Sending: accept it on the iPhone" width="280">
</p>

- A card above the tray for each transfer, coming in or going out: an
  incoming one asks first (below), then shows a progress bar with size and
  speed, and ends with a thumbnail, **Open** and **Show in folder**. Sending
  shows each stage (looking for the iPhone, accept it on the iPhone,
  uploading, sent). Cards go by themselves a few seconds after the end,
  unless the pointer is on them.
- Files land in `Downloads\HowlDrop`. Shared text goes on the clipboard.
  Settings can choose another parent folder; HowlDrop uses a dedicated
  `HowlDrop` subfolder there after the app restarts.
- Click the icon to open the **HowlDrop window**: a drop zone (drag files on
  it, or click to choose), **Send to** (any nearby iPhone, or a device from
  **Look for devices**), and **Recent** (click to open, right-click to show in
  its folder). Its gear has the settings. While it is open it shows the
  transfers itself, and no cards or notifications pop up; an AirDrop coming
  in brings it to the top.
- With **AirDrop on this PC**, a selection of files and folders is sent
  together, with one acceptance on the iPhone. Use **Choose a folder** or
  drop a folder on the window. Active outgoing transfers have **Cancel**;
  unsuccessful or cancelled ones have **Retry**. Keep the originals
  available until sending finishes; Retry sends their current contents.
  The Home Assistant route continues to send individual files.
- Tab and Shift+Tab move between the window's controls; Enter or Space
  activates one. Escape returns from settings or hides the window. The
  controls expose their names and current settings to Windows screen readers.
- Right-click the icon for the menu:

| Menu item | Default | |
|---|---|---|
| Open HowlDrop | | The window (also a click on the icon). |
| AirDrop on this PC (USB Wi-Fi stick) | off | The PC's own radio (below). The line under it says how it is going. |
| Who can send to this PC | Everyone, but ask me | **Everyone, without asking**; **Everyone, but ask me** (your contacts go straight through, anyone else waits for Accept); **Contacts only** (only your contacts see the PC). Contacts are set in the window's settings. |
| Delete files here after 30 days | on | Files in `Downloads\HowlDrop` go 30 days after they arrived, as the add-on's `delete_after_days`. |
| Show notifications | on | Cards for finished transfers, and notifications. An ask always shows. |
| Open links in the browser | on | A shared web page opens, as on a Mac. Only `http` and `https` addresses. |
| Open photos and videos | off | Opens each one in your photo or video app. |
| Open PDFs and text files | off | `.pdf` and `.txt` only. |
| Send to iPhone | | **Look for devices**, then a device (or **Any nearby iPhone**), then files: AirDropped by this PC's radio when it is on, else through the add-on (below). |
| "iPhone" in Explorer's Send to menu | on | Right-click a file, **Send to**, **iPhone (HowlDrop)**. |
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

## AirDrop on this PC

Windows' own Wi-Fi drivers cannot do AirDrop's Wi-Fi (AWDL), so HowlDrop
drives a USB Wi-Fi stick itself: `howldrop-agent.exe` runs the stick from
user space, AWDL and AirDrop on top of it, with a network stack of its own
as the link (`docs/native-windows.md`). It runs as you, with nothing
installed. You need:

- Windows 11 (or 10, version 1803 or later).
- A USB Wi-Fi stick with a MediaTek **MT7612U** (e.g. Alfa AWUS036ACM,
  `0e8d:7612`, or Netgear A6210: any ID Linux's `mt76x2u` driver takes). A
  USB 2.0 port worked best. The PC's built-in Wi-Fi can't do it;
  [Which Wi-Fi adapter?](../docs/adapters.md) has what works and what
  doesn't.

Turn on **AirDrop on this PC** in the menu. The first time, HowlDrop
downloads its agent (about 3.5 MB, from its release, checked against the SHA-256
built into the exe). If the stick isn't on Windows' own WinUSB driver yet,
Windows asks once to let HowlDrop put it there, so the agent can drive it;
after that it starts without asking. The dot
on the icon says how it is going: amber while it starts, green when the PC is
on the share sheet (by its name), red when it needs you (no stick, say), and
none while it is off. Turning it off, or quitting, gives the stick back. If
the radio stops by itself once it has been on the share sheet (the PC slept
and the stick went with it), HowlDrop starts it again, amber meanwhile; red
means four tries in a row didn't bring it back.

Files land in `Downloads\HowlDrop` (one copy; the agent writes there
directly), text goes on the clipboard, and nothing listens on the network
between the two halves.

**Who can send** decides what happens when an iPhone sends. Contacts are
phone numbers (with the country code, +1 555 123 4567) or Apple ID emails, in
the window's settings; a sender counts as one only when its Apple-signed
record says so, as with AirDrop's own Contacts Only. With **Everyone, but ask
me**, anyone else's send waits on a card for **Accept** or **Decline**, and no
answer in a minute is Decline. Changing it restarts the radio. Sending to the iPhone needs the PC's Bluetooth on:
HowlDrop wakes the phone with a Bluetooth advert while it sends or looks for
devices, and your Bluetooth devices stay connected. The radio's log is
`%LOCALAPPDATA%\HowlDrop\radio.log`.

**Slow?** With the MT7612U a photo takes about a second and a 15 MB video
about five. Much slower is most often the iPhone busy on its own Wi-Fi (an
iCloud Photos upload, a VPN with an exit node): it then spends less time on
AirDrop's channel. On the iPhone, open Control Center and tap the Wi-Fi icon
once; AirDrop keeps working. `radio.log`'s `schedule` lines say how many of
its 16 time slots each nearby Apple device gives HowlDrop's channel.

Sending speed is Automatic by default: HowlDrop sends at up to 54 Mbit/s and
steps down to 36, 24 or 12 when frames start needing retries, as at range.
Settings can fix it instead at `12` Mbit/s, `54` Mbit/s, or `mcs7` (HT). Receive speed is also
limited by the iPhone's schedule. The radio log includes network queue-drop
counts to help diagnose a slow or interrupted transfer.

## Sending to your iPhone

Set the iPhone's AirDrop to **Everyone for 10 Minutes**, send, and accept on
the phone. With **AirDrop on this PC** on, the PC sends it itself. Otherwise
it goes to the add-on, which AirDrops it: turn on the add-on's
`send_to_iphone` (with `pc_key` set). The add-on's relay shows up in
`howldrop peers` as **iPhone (HowlDrop)**. A notification says how it went.

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
howldrop iphone [--to NAME] FILE...  AirDrop FILE (this PC's radio, or the add-on)
howldrop cancel ID        cancel a local outgoing job (ID from radio.log)
howldrop retry ID         retry a failed or cancelled local job
howldrop nearby [--scan]  the devices it can send to
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
`howldrop.exe` on every push (the `howldrop-windows` download on the run's
page).

To release it: set `kVersion` in `src/config.hpp` and the agent's version in
`crates/howldrop-agent/Cargo.toml` to the same one, push it, then push a tag
`windows-vVERSION` (e.g. `windows-v0.20.0`).
`.github/workflows/release-windows.yml` builds the agent and packs
`howldrop-native.zip` (the agent and the stick's firmware, with its
licence), builds and tests the exe with its address and SHA-256 built in,
and publishes a GitHub release with both attached and `RELEASE.md` as its
notes. A local build has no agent to download; it uses the one beside it
(`howldrop-agent.exe` and `firmware\`).

The pictures in `docs/screenshots` are drawn by the app itself, from made-up
data (`src/screenshots_win.cpp`); after changing the window or the cards,
make them again:

```powershell
howldrop.exe screenshots docs\screenshots
```

`tests/` runs on Linux (`howldrop-test`, `howldrop-bridge-test`) and on
both (`howldrop-open-test`, the rules for what opens by itself, in
`src/open_policy.cpp`; `howldrop-radio-test`, what AirDrop on this PC reads
from the agent, and the 30-day cleanup;
`howldrop-transfers-test`, what the cards and the window show for each
transfer, and the Recent list's history).
