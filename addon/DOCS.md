# HowlDrop

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

- **A Wi-Fi card that [Owly](https://github.com/t4t5/owly)'s driver table
  supports** (`crates/owly-radio/drivers.toml`; [Which Wi-Fi
  adapter?](https://github.com/MrSco/howl-drop/blob/main/docs/adapters.md)),
  within AirDrop range of the phone. The add-on picks one by itself:
  - a card that is **not** Home Assistant's network uplink runs **exclusive**
    or **active-monitor** mode for as long as the add-on runs (it leaves any
    Wi-Fi network). Best is a USB **MediaTek MT7612U** (Alfa AWUS036ACM): its
    active monitor runs on 5 GHz, 40 MHz wide, where the iPhone runs its
    transfers. Cards that run on channel 6 only (Intel's, in exclusive mode)
    work but are slow.
  - a card that **is** the uplink, connected on channel 6, 44 or 149, can run
    **shared mode** where the table says so. That keeps the connection, and
    is untried on Home Assistant OS.

  A USB card needs no passthrough: the host's driver makes the interface and
  the add-on shares the host network. The [hardware notes](https://github.com/MrSco/howl-drop/blob/main/docs/hardware-notes.md)
  have what was measured.
- **Bluetooth on the host** (`hci0`): the add-on advertises through the
  host's `bluetoothd` so a phone wakes AirDrop for it.
- An amd64 machine (tested) or aarch64, such as a Raspberry Pi 3, 4 or 5
  (prebuilt too, untested). Home Assistant OS for the Pi includes the MT7612U
  driver; use a USB 3 port on a Pi 4 or 5. Without Home Assistant OS, the
  same image runs under Docker ([docker/README.md](https://github.com/MrSco/howl-drop/blob/main/docker/README.md)).

## Install

1. Settings, Add-ons, Add-on store, menu (⋮), **Repositories**, and add
   `https://github.com/MrSco/howl-drop#ha-addon`.
2. Install **HowlDrop** from the store. It downloads a prebuilt image.
3. Turn **Protection mode off**. The add-on needs the host network, the host
   PID namespace, `NET_ADMIN`, `NET_RAW`, `SYS_ADMIN`, `/dev/net/tun`, the
   host D-Bus and AppArmor off, which Supervisor refuses while protection mode
   is on. Leave full access off: it is not needed.
4. Start it, and open the share sheet on the phone.

Updates show up in the store like any add-on's. Boot is manual: start the
add-on when you want AirDrop, or set it to start on boot. Stopping it gives
the card back.

### Developing it as a local add-on

`tools/stage-addon.sh` assembles the add-on at `build/addon/howldrop/` (this
folder, plus the radio stack and the `howldrop-bridge` source), and with a
destination copies it there: `tools/stage-addon.sh
//homeassistant/addons/howldrop` over the **Samba share** add-on. It then
appears under Local add-ons and builds on your machine. Bump `version` in
`config.yaml` for every change, or Supervisor keeps the old image.

## Sending to an iPhone (experimental)

With `send_to_iphone` on, HowlDrop AirDrops files to a nearby iPhone too:

1. On the iPhone: Control Center, AirDrop, **Everyone for 10 Minutes**.
   Keep it unlocked. Contacts Only won't work: there the phone takes files
   only from senders with an Apple-signed identity, which only Apple devices
   have. iOS switches back on its own after 10 minutes.
2. Send the file, any of these ways:
   - **Send to iPhone** in the HowlDrop panel.
   - On a paired Windows PC (`pc_key` set): **Send a file to iPhone…** in
     the HowlDrop icon's menu, right-click a file in Explorer and **Send
     to**, **iPhone (HowlDrop)**, or `howldrop iphone FILE`.
   - Copy it into `save_folder/send` (`\\homeassistant\share\howldrop\send`).
3. Accept it on the phone.

The panel shows each send as it goes. The file then moves to `send/sent`,
or `send/failed` if the phone declined or never showed up within 3 minutes;
`Send:` lines in the log say each step.

Which device: **Look for devices** (in the panel, or in the PC's HowlDrop
menu under Send to iPhone) asks every AirDrop device in range for its
name, which takes about 30 seconds, and lists those that answer: an
iPhone set to Everyone for 10 Minutes, a Mac with AirDrop open. Contacts
Only phones don't answer anyone but their contacts. Choose one and send;
only the device answering to that name gets the file. A scan connects to
each device to ask, as a Mac's AirDrop window does, so it runs only when
you ask.

Without a choice, `send_to` decides: the phone's name as AirDrop shows
it, e.g. `Rocco's iPhone` (part of the name will do). Left empty,
HowlDrop sends to the only AirDrop device it hears, or of several the one
whose name says iPhone; with two iPhones in range, it doesn't guess. The PC reaches the
add-on through its relay, which shows up on the network as **iPhone
(HowlDrop)** (TCP 47635) and takes files only with your pairing key.

## The HowlDrop panel

**HowlDrop** in Home Assistant's sidebar (or **Open web UI** on the add-on's
page) lists what arrived, newest first: who sent it, thumbnails of photos,
text you can copy, links you can open, a download for each, and delete. It
works in the Home Assistant app too, and refreshes every few seconds. Turn
**Show in sidebar** off on the add-on's page to hide it.

Only signed-in Home Assistant users reach it: the add-on answers only Home
Assistant's own proxy (ingress), never the network. Anyone in range can send
in Everyone mode, so the panel treats every drop as untrusted: photos,
videos and plain text open in the browser, and anything else (a web page, a
program) is only ever a download.

## Notifications and automations

Every file that arrives fires a `howldrop_received` event in Home Assistant,
with:

| Field | |
|---|---|
| `sender` | the sending device's name, e.g. `Rocco's iPhone` |
| `name`, `path`, `folder` | the file's name, its full path (`/share/howldrop/...`), and `save_folder` |
| `kind` | `image`, `video`, `text`, `link` or `file` |
| `size` | in bytes |
| `url` | for a link, its address |

`notify` lists where to send a notification for each drop, e.g.
`notify.mobile_app_your_phone` (the Home Assistant app) or
`persistent_notification` (the bell in Home Assistant). A photo reads
"Photo from Rocco's iPhone: IMG_4167.JPG"; text shows its first lines; a link
shows its address and opens it when tapped.

For anything else, use the event in an automation. For example, a photo on a
dashboard tablet as it arrives:

```yaml
triggers:
  - trigger: event
    event_type: howldrop_received
    event_data:
      kind: image
actions:
  - action: notify.mobile_app_kitchen_tablet
    data:
      message: "New photo from {{ trigger.event.data.sender }}"
```

## Options

Every option can stay at its default.

`pc_name` (default `HowlDrop`) is the name on the share sheet, at most 63
characters.

On the sheet HowlDrop is listed under Other Devices with a plain contact
silhouette. A picture there isn't possible: the iPhone takes it from its own
Contacts, and only for a receiver with an Apple-issued AirDrop identity, which
this add-on doesn't have. Which Mac model the add-on reports makes no
difference on an iPhone. Empty with `forward_to_pc` on: the name a running `howldrop.exe`
announces in the first 10 s, so the sheet shows that PC's name.

`save_folder` (default `/share/howldrop`) is where files land. Text you
share (from Notes, say) lands as a `.txt` file in `text/` under it.

`delete_after_days` (default 30) deletes files from `save_folder` that many
days after they arrived there, checked when the add-on starts and then
once a day, so the folder doesn't fill
up. `0` keeps everything. Age counts from arrival, not from the file's own
date: a photo taken years ago and AirDropped today stays 30 days, and so
does a file a PC copies into the folder. This covers everything under
`save_folder`, including `text/` and `debug/`. Move anything you want to
keep somewhere else.

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
either way. `howldrop.exe` sits in the tray, shows a Windows notification
for each drop, and can open links, photos and documents as they arrive, as
a Mac does (its menu says which; links are on by default). Download it from
[the latest release](https://github.com/MrSco/howl-drop/releases/latest)
(use the one matching this add-on's version) or build it with
`tools\build-howldrop.ps1`; the repo's `howldrop/README.md` has the details.
Allow UDP 47631 and TCP 47632 from the LAN on that PC. Set `pc_host` to the
PC's address only when its beacon does not reach Home Assistant.

`pc_key` pairs the add-on with that PC. `howldrop.exe` accepts files only
from a sender that knows its pairing key, so nobody else on the network can
send it files, set its clipboard or open pages in its browser; and the add-on
sends only to a PC that proves it knows the key, so a machine pretending to
be your PC gets nothing. On the PC, right-click the HowlDrop icon, choose
**Copy pairing key**, paste it here, and restart the add-on. Every transfer
is signed with it, and a changed or replayed one is dropped.

`interface` (default `auto`) names the Wi-Fi interface to use, e.g.
`wlan1`. `auto` takes the card Owly's probe picks from the driver table.
When none fits, the log says why (`Missing ...`).

`allow_untested` (default off) lets the add-on use a card that Owly's driver
table lists as untested, or doesn't list. If you try one,
[Which Wi-Fi adapter?](https://github.com/MrSco/howl-drop/blob/main/docs/adapters.md)
says how to report it.

`channels` is the channels to try, in order, separated by spaces: `6`, `44`
or `149`. Empty or `auto`: the card's own order from the driver table (149,
44, 6 for the MT7612U: an iPhone runs its transfers on 149). Shared mode
ignores this: it runs on its network's channel.

`aggregation` (default `auto`) lets the phone send in aggregated bursts
(A-MPDU) instead of one frame at a time, which is how Wi-Fi normally reaches
its speed. The iPhone asks for this (a Block Ack session); with this on, the
add-on says yes and puts the frames back in order for TCP, as a normal Wi-Fi
stack does. Each burst also has to be answered with a Block Ack by the card
itself. The MT7612U's active monitor does, so `auto` follows the driver
table, which turns it on there. `on` and `off` force it; if transfers fail
or crawl with it on, turn it off.

`tx_rate` (default empty: 24 Mbit/s) is the rate files go to the iPhone at:
`6` to `54` (Mbit/s), or `mcs0` to `mcs15`. Measured on an MT7612U in
Windows' WSL, `36` sent a 14.7 MB video in 16 s instead of 25 s; `48` was
erratic and `mcs7` made the phone cancel. Raise it only while transfers keep
arriving whole.

`verbose` (default off) logs the radio in detail: each `/Discover` that
reaches the add-on, the radio's set-up, and a copy in `/tmp` of the preview
a phone sends with its files. Without it the log keeps the start-up, each
peer the radio syncs with, each transfer with how long it took, deliveries,
failures, and the radio's counters every 10 s.

`debug_capture` (default off) writes pcap files of the monitor interface and
the AirDrop TCP to `save_folder/debug` (a ring: up to 160 MB and 80 MB).
Turn it off and delete the folder when done.

## Reading the log

The add-on's log is Home Assistant's: it goes to the system journal, which
Home Assistant OS rotates. The radio's own logs inside the add-on are cut
back to their last 2 MB once one passes 20 MB.

- `Wi-Fi card: <name> (driver ..., <usb id>), tested, active-monitor mode`
  says which card runs and how; `active-monitor mode on channel 149` where.
- `Window is up on <name>` means the phone can see the name now.
- `Receive: ask from <phone>` and `Receive: stored <file>: 14.7 MB in 3.2 s
  (4.6 MB/s)`, then `Deliver: delivered <file>`, are a transfer.
  `Receive: not a known contact: ...` says why a sender wasn't taken for one
  of `senders`.
- `add peer <mac>` means the radio synced with a phone. None at all, with
  the share sheet open, means the radio never heard it: check the card's
  channel and distance.
- `Send: ... is here but not taking files yet` means the phone's AirDrop
  refused: its Everyone for 10 Minutes ran out, or it has just finished a
  transfer (an iPhone keeps AirDrop shut for up to a minute after one). The
  send keeps trying.
- `STATS ...` every 10 s are the radio's counters: frames sent and received,
  and `timer late` (how far behind the radio's schedule runs).
- `Forward:` lines are the forwarder: `no HowlDrop PC answering` (files stay
  put), `forwarding to <PC>`, `sent file <name>`.
