# HowlDrop add-on

AirDrop from an iPhone to Home Assistant OS. While the add-on runs,
**HowlDrop** appears on the iPhone's share sheet; files and text sent to it
land in `/share/howldrop`, which the Samba share add-on shows to every PC on
the network as `\\homeassistant\share\howldrop`.

That is the recommended way to use it, and the default: nothing to install
on any PC. `howldrop.exe` is optional, for a Windows PC that should also get
each file in its Downloads and shared text on its clipboard
(`forward_to_pc`).

Home Assistant OS stays as it is: the add-on uses a Wi-Fi card the host
kernel already drives, and loads no driver or firmware of its own.

## What it needs

- **A Wi-Fi card with an omdrop-owl profile** (`userspace/owl-profiles`),
  within AirDrop range of the phone. The add-on picks one by itself:
  - a card that is **not** Home Assistant's network uplink runs **exclusive
    mode** for as long as the add-on runs (it leaves any Wi-Fi network). Best
    is a card with an active-monitor profile, such as a USB **MediaTek
    MT7612U** (Alfa AWUS036ACM): it follows the phone onto 5 GHz, where the
    iPhone runs its transfers. Cards with an IBSS profile (Intel AX2xx,
    Realtek RTL8822BE) hold channel 6 only and are slow.
  - a card that **is** the uplink, connected on channel 6, 44 or 149, can run
    **shared mode** if its profile says so. That keeps the connection, and is
    untried on Home Assistant OS.

  A USB card needs no passthrough: the host's driver makes the interface and
  the add-on shares the host network. [docs/hardware-notes.md](../docs/hardware-notes.md)
  has what was measured.
- **Bluetooth on the host** (`hci0`): the add-on advertises through the
  host's `bluetoothd` so a phone wakes AirDrop for it.
- An amd64 machine (tested) or aarch64 (builds the same way, untested). The
  first install compiles OWL, which needs a network path to GitHub and takes a
  while.

## Install

Supervisor builds a local add-on from its own folder, so `tools/stage-addon.sh`
assembles a self-contained one at `build/addon/howldrop/` (this folder, plus
the radio stack and the `howldrop-bridge` source).

1. Install the **Samba share** add-on (or another way onto `/addons`).
2. From the repo root: `tools/stage-addon.sh //homeassistant/addons/howldrop`
   (use your Home Assistant's name or address), or run it with no argument and
   copy `build/addon/howldrop/` to `/addons/howldrop/` yourself.
3. Settings, Add-ons, Add-on store, menu, **Check for updates**. HowlDrop
   appears under Local add-ons. Install it.
4. Turn **Protection mode off**. The add-on needs the host network, the host
   PID namespace, `NET_ADMIN`, `NET_RAW`, `SYS_ADMIN`, `/dev/net/tun`, the
   host D-Bus and AppArmor off, which Supervisor refuses while protection mode
   is on. Leave full access off: it is not needed.
5. Start it, and open the share sheet on the phone.

To deploy a change, bump `version` in `config.yaml` and stage again; without
a new version Supervisor keeps the old image. Boot is manual: start the
add-on when you want AirDrop. Stopping it gives the card back.

## Options

Every option can stay at its default.

`pc_name` (default `HowlDrop`) is the name on the share sheet, at most 63
characters. Empty with `forward_to_pc` on: the name a running `howldrop.exe`
announces in the first 10 s, so the sheet shows that PC's name.

`save_folder` (default `/share/howldrop`) is where files land, and shared
text as UTF-8 files in `text/` under it.

`senders` (default empty: anyone nearby can send) restricts who may send,
like a Mac set to Contacts Only. List the phone numbers and Apple ID email
addresses of the people allowed, one per entry (one entry with commas also
works). Write phone numbers with the country code, e.g. `+1 555 123 4567`:
Apple compares the number with its country code, so `555 123 4567` never
matches. With any sender listed, a transfer is accepted only from an iPhone
that proves, with Apple's signature, that it belongs to an Apple ID with one
of those numbers or addresses, over a connection whose certificate Apple
issued to that same Apple ID; anyone else gets "declined" and nothing is
saved. The add-on also stays off the share sheet of a phone it can identify
as someone else's. The sender must be signed in to iCloud. The log says
`Senders: Contacts Only` and how many are listed (never the numbers), then
`ask accepted` or `ask REFUSED (contacts only)` with the reason. Being on
your Wi-Fi does not matter: AirDrop does not use your network.

`fetch_links` (default on): sharing a picture from Safari or Chrome sends its
web address, not the picture, and an address is stored as a `.url` shortcut.
With this on, a shortcut whose address serves a file (anything but a web
page) is replaced by the file. Fetching is fenced, because anyone allowed to
send can send a link: http and https only, public internet addresses only,
checked again on every redirect (nothing on your LAN), up to `fetch_max_mb`
(default 100), 30 s timeout. `Deliver:` lines say what was fetched or why
not.

`forward_to_pc` (default off; optional) also sends each new file to
`howldrop.exe` when a PC runs it: the file goes to that PC's
`Downloads\HowlDrop` and text to its clipboard. Files stay in `save_folder`
either way. Build `howldrop.exe` with `tools\build-howldrop.ps1`. Allow UDP 47631 and TCP
47632 from the LAN on that PC. Set `pc_host` to the PC's address only when
its beacon does not reach Home Assistant.

`interface` (default `auto`) names the Wi-Fi interface to use, e.g.
`wlan1`. `auto` picks one: an exclusive-mode card that is not the uplink
(active monitor first), else a shared-mode card that is. When none fits, the
log lists every Wi-Fi interface and why it was passed over.

`allow_untested` (default off) lets the add-on use a card whose profile is
untested, including the catch-all line for drivers the table does not name.
If you try one, the omdrop-owl README says how to report it.

`channels` (exclusive mode) is the channels to try, in order, separated by
spaces: `6`, `36`, `44` or `149`. Empty or `auto`: an active-monitor
card listens about 5 s each on 149, 44, 36 and 6 for the phone (keep the
share sheet open) and starts on 5 GHz, where the phone was heard if anywhere,
else 149; an IBSS card uses 6. Shared mode ignores this: it runs on its
network's channel.

`verbose` (default off) logs the radio in detail: every request's headers,
OWL's peer schedule changes, each TCP connection, frame counts on the
monitor, mDNS and announcer lines, and a radio check every 30 s. Without it
the log keeps the start-up, each transfer, deliveries and failures.

`debug_capture` (default off) writes pcap files of the monitor interface and
the AirDrop TCP to `save_folder/debug` (a ring: up to 160 MB and 80 MB).
Turn it off and delete the folder when done.

## Reading the log

- `Interface <name>: driver ...` and `Profile: ...` say which card runs and
  how.
- `Window is up on <name>` means the phone can see the name now.
- `ask from <phone>` and `upload ...: stored <file>`, then
  `Deliver: delivered <file>`, are a transfer.
- `OWL: ... add peer <mac>` means OWL synced with a phone. None at all, with
  the share sheet open, means the radio never heard it: check the card's
  channel and distance.
- `upload ...: no closing chunk after 3 s, but the archive is complete;
  storing it` is normal: the iPhone sometimes never ends its upload, and the
  receiver answers once the archive is whole
  ([hardware notes](../docs/hardware-notes.md)).
- `Forward:` lines are the forwarder: `no HowlDrop PC answering` (files stay
  put), `forwarding to <PC>`, `sent file <name>`.
