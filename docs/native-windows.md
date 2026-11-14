# AirDrop on Windows without WSL

Until 0.19.2 howldrop.exe ran the radio in WSL 2: usbipd-win handed the USB
stick to WSL, the Linux kernel's driver ran it, and `howldrop-agent` (Owly's
crates) did AWDL and AirDrop on top. It worked, but it was a lot of
machinery: WSL, a 250 MB Linux part, usbipd, and a Linux kernel whose Wi-Fi
stack could wedge after a sleep (only `wsl --shutdown` freed it).

Now the same agent runs as a Windows program, driving the stick itself over
USB, and the WSL route is gone: the RTL8852BU, which only it ran, is the
Home Assistant add-on's alone.

## What stays and what goes

`owly-awdl` and `owly-airdrop` build on Windows (Owly 0.4.0 and later). The
node reaches the radio through two traits in `owly-awdl::driver`:

- `Air`: 802.11 frames out and in.
- `Netif`: the host's network interface for the link, where IPv6 packets come
  and go.

On Linux, `owly-radio` implements both with a monitor interface and a TAP
device, on top of the kernel's driver. On Windows we have our own:

| Layer | Linux (the add-on) | Windows |
|---|---|---|
| AirDrop, AWDL | `owly-airdrop`, `owly-awdl` | the same |
| `Air` | `owly-radio` + the kernel's `mt76x2u` | a user-space MT7612U driver over USB (`nusb`, WinUSB) |
| `Netif` | TAP | the agent's own TCP/IP stack (smoltcp) |
| BLE wake | BlueZ | the tray, Windows Bluetooth |
| Contacts check | Rust (`pki.rs`) | the same |
| Previews (FileIcon) | Python + Pillow | Rust (`image`, OpenJPEG) |
| Delivery | `deliver.py` | the agent (`--deliver`) |

The tray reads the agent's `@howldrop` events, as it read them from WSL, so
the Windows app changed little: it starts `howldrop-agent.exe` itself, as
the user, instead of `wsl.exe ... /wsl-run.sh`.

## The driver

Linux's `mt76x2u` (openwrt/mt76: `mt76x2/usb*.c`, `mt76x02_*`, `usb.c`) is
BSD-3-Clause-Clear / ISC, so it can be ported. Only a monitor-and-inject
radio is needed, not a mac80211 driver: no station or AP mode, scanning,
beacons, PCI, DFS or debugfs. What it has to do:

1. **Registers**: read and write through USB vendor control requests
   (`MT_VEND_*`); the chip's ID is register 0 (`0x76120044` on the tested stick).
2. **Power and firmware**: `mt76x2u_power_on`, then the ROM patch
   (`mt7662_rom_patch.bin`) and firmware (`mt7662.bin`) over bulk OUT with
   their DMA headers (`mt76x2u_mcu_fw_init`), then the MCU's init commands.
3. **EEPROM and calibration**: the MAC address, per-channel power and the
   calibration the MCU runs on a channel change (`mt76x2u_phy_set_channel`).
4. **Tune**: channel 149, HT40+ (as Owly's patched `set_freq` does on Linux).
5. **Receive**: an RX filter that keeps everything, frames off bulk IN with
   their RXWI descriptor (rate, RSSI) stripped.
6. **Send**: frames on bulk OUT with a TXWI descriptor (rate: 36 Mbit/s, as
   `tx_rate` on Linux; queue; no ACK wait for multicast).
7. **ACKs and Block Ack**: the hardware must ACK unicast frames to the node's
   AWDL address (set as the chip's own MAC) and answer Block Ack sessions the
   iPhone opens (its uploads are aggregated: that is where the 3 s video comes
   from). How mt76x02 does this (WCID table, `mt76x02_mac_wcid_setup`, BA
   registers) is the first thing to read once receiving works.

`nusb` also runs on Linux (usbfs), so steps 1–6 can be developed in WSL with
the kernel's `mt76x2u` unloaded, before the stick is switched to WinUSB on
Windows.

## Milestones

1. **Talk to the stick**: open it with `nusb`, read the chip ID and the MAC
   from the EEPROM.
2. **Firmware**: load the ROM patch and firmware; the MCU answers a command.
3. **Listen**: tune to 149 HT40+, receive everything, and see an iPhone's AWDL
   action frames (an `Air` that only receives).
4. **Talk back**: send, and ACK in hardware; an `owly-awdl` node on this `Air`
   syncs with the iPhone (it shows as a peer).
5. **Network**: Wintun as `Netif`; the iPhone sees the PC on its share sheet
   and a photo arrives.
6. **The agent on Windows**: `howldrop-agent.exe` with the Rust contacts check
   and previews; the tray starts it instead of WSL. Gate: as fast as WSL (a
   14.7 MB video from the iPhone in about 3 s, to it in about 16 s), Accept and
   Decline, contacts, Look for devices.
7. **Install**: the stick on WinUSB and Wintun set up without the user
   needing to know what either is. Then WSL goes, and with it the sticks
   without a Windows driver of ours (the RTL8852BU).
8. **No administrator**: the link inside the agent instead of a Wintun
   adapter, so it runs as the user with nothing installed; Windows asks once,
   only to put a stick on WinUSB.

## Open questions

- **Taking it off a PC**: nothing gives the stick its old driver back;
  Device Manager's Update driver, picking the one it had, does, by hand.
- **Timing**: AWDL's availability windows are 16 TU; the node runs its timers
  in software (`timer late avg` about 1.1 ms on Linux). Windows wakes them on
  its 15.6 ms tick (6.5 ms late on average, 16 ms at worst) unless the
  process asks for 1 ms (`timeBeginPeriod`) and opts out of Windows 11's
  timer throttling; the agent does both and gets 1.2 ms.
- **The preview on the iPhone's Accept prompt.** A JPEG 2000 FileIcon shows
  on the "AirDrop Complete" banner, and a plain JPEG doesn't show at all,
  but the prompt shows none, even for one like an iPhone's own in size,
  coding and header. 0.18.0's sender got a 240-pixel JPEG onto the prompt
  (iOS of October 2026); the Linux sender (Pillow) is no better today.
  Perhaps the prompt shows previews only from a sender with an Apple ID
  record.

## Where it is

`crates/mt7612u` does milestones 1 to 5, in WSL (usbfs) with the kernel's
`mt76x2u` set aside and on Windows (WinUSB), on an Alfa AWUS036ACM
(`0e8d:7612`):

1. **Talk to the stick**: chip `0x76120044`, the EEPROM's MAC address.
2. **Firmware**: ROM patch and firmware running in about 0.25 s; the MCU's
   register reads match direct ones.
3. **Listen**: 149 HT40+ tuned in under a second; every frame on the
   channel, among them an iPhone's AWDL frames at -41 to -46.
4. **Talk back**: an `owly-awdl` node on the stick (`awdl::Radio`) synced
   with the iPhone within a second and kept it as a peer; data frames went
   both ways. Broadcast frames must not ask for an ACK (the chip retries
   them to its limit otherwise).
5. **Network**: on Linux first, `howldrop-agent --usb-radio FW` with `awdl0`
   the TAP owly-radio makes (`Native::start_air`): the iPhone sent a 1.1 MB
   photo in 1.1 s, and with Block Ack in hardware (the peer in the WCID
   table, its TID's bit set as the node accepts the session) the 14.7 MB
   video in 3.8 s, against 3.2 s on the kernel's driver. Then on Windows,
   with a Wintun adapter as the link: the radio up in 0.7 s, a 2 MB photo in
   0.6 s.
   - Windows gives the Wintun link a random link-local address of its own
     once the session starts, and sends from it; no MAC maps to it, so the
     link deletes every address but the AWDL one, and keeps doing so.
   - Nothing resets the stick on Windows (Linux resets it on probe), so a
     run after another finds the chip still receiving and its IN endpoints
     full; `start_mcu` first stops the MAC and drains them.
6. **The agent on Windows**: `howldrop-agent.exe` builds for Windows (owly-radio
   and BlueZ are Linux's only), and howldrop.exe runs it in place of WSL.
   On the user's PC, from the tray:
   - receiving: Accept on the PC, a contact accepted without asking (its
     Apple ID record and certificate checked in Rust, `pki.rs`: Apple's
     records are BER, which the check reads as openssl did), progress, and
     the 14.7 MB video in 3.8 s, delivered into the inbox (`--deliver`, as
     deliver.py did); the service name kept beside the identity, so a share
     sheet left open across a restart still reaches it;
   - sending: Look for devices found the iPhone, and a photo went to it with
     the tray's Bluetooth wake; Decline on the iPhone came back as declined;
   - the preview (JPEG 2000, OpenJPEG) shows on the iPhone's banner, not its
     prompt (open questions);
   - howldrop.exe stops the agent by disconnecting its pipe, and the agent
     gives the stick back (exit 0).

   Not yet measured: the video to the iPhone (about 16 s on WSL). Not done:
   fetching what a link points at (a link stays a `.url` shortcut).
7. **Install**: the first time it's turned on, howldrop.exe runs
   `howldrop-agent.exe --setup` as administrator (Windows asks, once; again
   only for a new agent), which (setup.rs):
   - copies the agent, its launcher howldrop-agentw.exe, wintun.dll and the
     firmware into Program Files\HowlDrop, where only an administrator can
     change them;
   - registers the task `\HowlDrop\AirDrop`, which runs the launcher with
     the user's full rights, at normal priority (a task's default runs it
     below normal) and without a console window, reading the agent's
     options from `%LOCALAPPDATA%\HowlDrop\native\agent.args`; folders
     there outside the user's profile are refused;
   - puts each supported stick on Windows' own WinUSB (winusb.inf's "WinUsb
     Device", as Device Manager's "Let me pick" does: no package or
     certificate of ours), then restarts it by cycling its hub port, since
     the driver it had (`netr28ux`, Windows' own) leaves the chip
     answering nothing.

   From then on howldrop.exe writes the options, starts the task, and the
   agent connects to its pipe: no prompt, no window. A stick that answers
   nothing at start (after a sleep) is restarted the same way, once; one
   not on WinUSB makes the agent say so (`stick needs-winusb`) and
   howldrop.exe run the setup again. A release carries howldrop-native.zip
   (the agent, launcher, wintun.dll, firmware and their licences, built and
   checked by hash in release-windows.yml), which howldrop.exe downloads as
   it does the Linux part; a local build uses the agent beside it.

   Tested on the user's PC from a stick on `netr28ux`: one prompt, then the
   stick on WinUSB and the agent on the share sheet; off and on without a
   prompt. Released as 0.20.0, which Microsoft Defender's behaviour
   monitoring took for persistence (`Behavior:Win32/Persistence.A!ml`: an
   elevated scheduled task, from a program that downloads executables).
8. **No administrator**: the link moved into the agent (`netstack.rs`), so
   nothing is installed and nothing runs elevated:
   - a TCP/IP stack of its own (smoltcp, through tokio-smoltcp) on the
     node's IPv6 packets, with 1 MB TCP buffers; the receiver listens on it
     and the sender connects through it (owly-airdrop's `Listen` and
     `Connect`, a fork patch);
   - mDNS, UDP to and from port 5353, taken off the node's packets before
     the stack sees them and written as packets of its own (`Datagrams`);
   - a connection closes as Windows' do: shutdown sends the FIN without
     waiting for the peer's (an iPhone closes seconds later, after the
     sender's 3 s knock is over), and a dropped one finishes closing in the
     background;
   - howldrop.exe starts the agent as the user, on a named pipe of its own,
     with the agent and firmware in `%LOCALAPPDATA%\HowlDrop\native\bin`. Only a
     stick not yet on WinUSB needs the administrator: the agent says
     `stick needs-winusb`, and howldrop.exe runs `--setup` elevated, which
     puts it on WinUSB and cycles its port, as above.

   On the user's PC: on the share sheet with no prompt; the 14.7 MB video
   from the iPhone in 5.1 s (3.8 s on Wintun); Accept on the PC; Look for
   devices found the iPhone, and a photo went to it.

Not ported yet: the EEPROM's receive gain corrections, 2.4 GHz transmit
power, and the periodic recalibration (TSSI compensation and DPD run once,
after the first frames go out: without DPD, 54 Mbit/s and MCS 7 frames needed
about two retries each). Startup in WSL
takes 1 to 10 s, nearly all of it the EEPROM read over usbip; on Windows it's
0.7 s.

## Local send jobs and TCP

The tray hands a whole selection to the agent in an atomic
`ID.howldrop-job` manifest in its private outbox. The UTF-8 lines are
`HowlDrop job 1`, the recipient (empty for automatic selection), then 1–256
absolute input paths. The tray writes a `.partial` file and renames it only
after closing it. Source files are read in place, and owly-airdrop prepares
one compressed archive for one Ask. Folders use its existing recursive
archive writer. Duplicate top-level names are refused by that writer.

Manifest processing requires `--local-jobs`. The add-on's network-facing
outbox does not enable it: a received file must never be interpreted as a
request to read paths on the host. Ordinary file outboxes retain their
settling and target-sidecar behavior.

`ID.howldrop-cancel` cancels discovery, the acceptance wait or upload. The
archive worker checks an atomic cancellation flag before opening files and
between blocks and directory entries. Failed and cancelled manifests move
to `failed/` for Retry; successful ones are deleted. Source files are never
deleted by job cleanup. Retry reads the sources again. Structured outgoing
events carry the job ID, item names, progress and terminal result; heartbeat
events keep long preparation from appearing stalled.

The Windows TCP stack enables smoltcp's Reno congestion controller. Without
either Reno or Cubic compiled in, smoltcp defaults to no congestion control,
which can overwhelm the radio with retransmissions after a loss. Socket
flush queues data as an OS TCP socket does; connection shutdown still drains
the pending send queue. An in-memory two-stack test holds acknowledgements,
drops a data packet, and checks a transfer larger than the TCP buffers.

Receive queue totals and drops are logged at the normal log level. Trace
logging for `howldrop_agent::netstack` adds TCP header summaries (sequence,
acknowledgement, flags and lengths), without TLS payload or shared contents.
The tray's transmit rate is passed to the agent. Its default, `auto`, has
the MT7612U radio choose the unicast data rate from 12, 24, 36 and 54 Mbit/s
once a second, from the chip's retry and failure counters (TX_STA_CNT0/1);
mt76x2u leaves this to mac80211's minstrel, which needs per-frame TX status.
A 13.2 MB batch to an iPhone takes about 8 s at 54 Mbit/s.

Hardware qualification still needed: sustained transfers at higher rates,
different MT7612U adapter variants and distances, channel alternatives, and
the receive gain corrections above. The experimental Owly Intel, Raspberry
Pi and protocol-tuning PRs are not enabled by this Windows change.

## Trying it

### Optional send-speed experiments

Three experiments are available for hardware comparisons, all off by
default. They have not yet been measured on an iPhone:

- `HOWLDROP_DEEP_QUEUE=1` sizes the chip's data queue for about 4 ms of
  airtime rather than always four frames, capped at 32 frames.
- Sending speed `auto-ht` extends the automatic ladder to 40 MHz HT,
  through MCS 15. The rates are PHY rates, not measured file throughput.
- `HOWLDROP_AMPDU=1` negotiates an outgoing Block Ack session and enables
  A-MPDU transmission. It is best tried with the deeper queue. A dropped
  frame can leave a gap that the phone must time out: outgoing BAR recovery
  is not implemented yet.

Build the Windows app and agent first, with firmware beside the agent as
below. Then compare the same files with the phone in the same place:

```powershell
tools/test-send-speed.ps1 -Mode baseline -Files C:\Temp\photo.png
tools/test-send-speed.ps1 -Mode deep -Files C:\Temp\photo.png
tools/test-send-speed.ps1 -Mode ht -Files C:\Temp\photo.png
tools/test-send-speed.ps1 -Mode ampdu -Files C:\Temp\photo.png
tools/test-send-speed.ps1 -Mode all -Files C:\Temp\photo.png
```

The script restores the previous setting, experiment flags and running app
when it finishes, and keeps a radio log under `build/send-speed/`. Its timing
starts when the upload is observed, after Accept; it does not include the
time spent finding the phone or waiting for approval.

### Radio examples

With the stick attached to WSL (`usbipd attach --wsl`) and HowlDrop's AirDrop
on this PC off, as root, with linux-firmware's `mt7662_rom_patch.bin` and
`mt7662.bin` in `FW`:

```
cargo run -p mt7612u --example info
cargo run -p mt7612u --example firmware -- FW
cargo run -p mt7612u --example listen -- FW 30
cargo run -p mt7612u --example awdl -- FW 60
```

`listen` and `awdl` want an iPhone nearby with its share sheet open.

On Windows, from a local build: put `howldrop-agent.exe` (`cargo build
--release -p howldrop-agent`) and a `firmware` folder with the two files
beside howldrop.exe, then turn AirDrop on this PC on; for a stick not yet on
WinUSB, let Windows run the setup.
`howldrop-agent --restart-stick`, as administrator, restarts the stick as
unplugging it would.
