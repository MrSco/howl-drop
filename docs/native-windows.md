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
- **Timing**: AWDL's availability windows are 16 TU; the node already runs its
  timers in software on Linux (`timer late avg` about 1.1 ms), but user-space
  USB on Windows adds its own latency. Milestone 4 measures it.
- **Block Ack in hardware** (step 7 above).

## Trying it

Nothing here yet. Milestone 1 will add a `crates/mt7612u` crate and an example
that prints the chip ID.
