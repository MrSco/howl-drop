# AirDrop on Windows without WSL

Today howldrop.exe runs the radio in WSL 2: usbipd-win hands the USB stick to
WSL, the Linux kernel's driver runs it, and `howldrop-agent` (Owly's crates)
does AWDL and AirDrop on top. It works, but it is a lot of machinery: WSL, a
250 MB Linux part, usbipd, and a Linux kernel whose Wi-Fi stack can wedge after
a sleep (only `wsl --shutdown` frees it).

This branch is about running the same agent as a Windows program, driving the
stick itself over USB.

## What stays and what goes

`owly-awdl` and `owly-airdrop` build on Windows (Owly 0.4.0 and later). The
node reaches the radio through two traits in `owly-awdl::driver`:

- `Air`: 802.11 frames out and in.
- `Netif`: the host's network interface for the link, where IPv6 packets come
  and go.

On Linux, `owly-radio` implements both with a monitor interface and a TAP
device, on top of the kernel's driver. On Windows we need our own:

| Layer | Linux (today) | Windows (this branch) |
|---|---|---|
| AirDrop, AWDL | `owly-airdrop`, `owly-awdl` | the same |
| `Air` | `owly-radio` + the kernel's `mt76x2u` | a user-space MT7612U driver over USB (`nusb`, WinUSB) |
| `Netif` | TAP | Wintun |
| BLE wake | the tray, Windows Bluetooth | the same |
| Contacts check | the `openssl` command | Rust (`x509-parser`/`cms`, or `openssl` the crate) |
| Previews (FileIcon) | Python + Pillow | Rust (`image` + a JPEG 2000 encoder) or the Windows Imaging Component |
| Delivery | `deliver.py` | the tray, which already delivers what the add-on forwards |

The tray keeps reading the agent's `@howldrop` events on stdout, so the
Windows app changes little: it starts `howldrop-agent.exe` instead of
`wsl.exe ... /wsl-run.sh`.

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
   needing to know what either is (open questions below). WSL stays as the
   fallback, and for sticks without a Windows driver of ours (the RTL8852BU).

## Open questions

- **Binding the stick to WinUSB** for users. Zadig/libwdi installs a
  self-signed certificate; UsbDk is signed (Red Hat) and libusb can use it; a
  signed INF of our own needs a code-signing certificate. During development,
  Zadig on the developer's machine is enough, and uninstalling the device's
  driver undoes it.
- **Wintun needs an administrator** to create its adapter: once at setup, or
  a small service.
- **Timing**: AWDL's availability windows are 16 TU; the node runs its timers
  in software (`timer late avg` about 1.1 ms on Linux). Windows wakes them on
  its 15.6 ms tick (6.5 ms late on average, 16 ms at worst) unless the
  process asks for 1 ms (`timeBeginPeriod`) and opts out of Windows 11's
  timer throttling; `howldrop-native` does both and gets 1.2 ms.

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
5. **Network, on Linux first**: `howldrop-agent --usb-radio FW` runs the
   receiver on the stick, `awdl0` the TAP owly-radio makes (`Native::start_air`).
   The iPhone found "HowlDrop Native", asked and sent a 1.1 MB photo in
   1.1 s. With Block Ack in hardware (the peer in the WCID table, its
   TID's bit set as the node accepts the session) its 14.7 MB video took
   3.8 s, against 3.2 s on the kernel's driver.
6. **Network, on Windows**: `howldrop-native` (the stick on WinUSB, a Wintun
   adapter as the link, run as administrator). The radio is up in 0.7 s; the
   iPhone sent a 2 MB photo in 0.6 s and the 14.7 MB video in 4.2 s.
   - Windows gives the Wintun link a random link-local address of its own
     once the session starts, and sends from it; no MAC maps to it, so the
     link deletes every address but the AWDL one, and keeps doing so.
   - Nothing resets the stick on Windows (Linux resets it on probe), so a
     run after another finds the chip still receiving and its IN endpoints
     full; `start_mcu` first stops the MAC and drains them.
   - The service name is kept beside the receiver's identity: a share sheet
     left open across a restart still reaches it.

Not ported yet: the EEPROM's receive gain corrections and per-rate transmit
power, 2.4 GHz transmit power, and the periodic recalibration. Startup in WSL
takes 1 to 10 s, nearly all of it the EEPROM read over usbip; on Windows it's
0.7 s.

## Trying it

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

On Windows, with the stick on WinUSB (Zadig) and WireGuard's `wintun.dll`
beside the program, as administrator:

```
cargo build --release -p howldrop-native
howldrop-native FW STAGING_DIR "HowlDrop Native" 600
```
