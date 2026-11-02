# Which Wi-Fi adapter do I need?

**Short answer: get a USB Wi-Fi adapter with a MediaTek MT7612U chip, like the
Alfa AWUS036ACM.** It's the one HowlDrop is built and tested with, on
Windows and on Home Assistant, and the only one HowlDrop for Windows can
drive; it's also the fastest: on Windows a 14.7 MB iPhone video arrives in
about 4 seconds.

AirDrop needs a Wi-Fi radio that can do things normal Wi-Fi doesn't
(Apple's AWDL: listening to everything on a channel, sending raw frames, and
following the iPhone onto 5 GHz). Most Wi-Fi hardware can't, or its driver
won't let it. That's why the choice matters.

## How to tell which chip you have

A product's name doesn't tell you its chip, and makers change chips between
versions. What counts is the **USB ID** (vendor:product).

- **Windows:** Device Manager → the adapter → Properties → Details →
  Hardware Ids, and look for `VID_xxxx&PID_xxxx`.
- **Linux / Home Assistant:** `lsusb` for USB adapters, `lspci -nn` for
  built-in cards.

HowlDrop for Windows recognizes a stick by its USB ID: the MT7612U's
`0e8d:7612` (Alfa AWUS036ACM) and every other ID Linux's MT7612U driver
takes. The list is `USB_IDS` in
[crates/mt7612u/src/lib.rs](../crates/mt7612u/src/lib.rs).

## The lists

✅ works, tested · 🟡 might work, not tested (or not supported yet) ·
🟠 tested, doesn't work now, could some day · ❌ can't work (hardware or
Windows limits)

### On Windows: AirDrop on this PC

Windows' own Wi-Fi drivers can't do AWDL, so HowlDrop drives the stick
itself, with a driver of its own for one chip: the MT7612U
([native-windows.md](native-windows.md)). So on Windows it has to be a USB
MT7612U.

| Adapter (chip) | USB ID | | Notes |
|---|---|---|---|
| MediaTek **MT7612U** (Alfa AWUS036ACM) | `0e8d:7612` | ✅ | Tested both ways: a 14.7 MB video from the iPhone in about 4 s. Runs on 5 GHz. |
| MT7612U with another USB ID: Netgear A6210 (`0846:9053`), ASUS USB-AC55 (`0b05:17eb`), and the others Linux's `mt76x2u` driver takes | `045e:02e6` `045e:02fe` `0471:2126` `0471:7600` `056e:400a` `057c:8503` `0846:9014` `0846:9053` `0b05:17eb` `0b05:180b` `0b05:1833` `0e8d:7632` `2357:0137` `2c4e:0103` | 🟡 | Same chip as the tested one, and HowlDrop for Windows drives them as it does it. Have one? [Test it](#test-yours-and-tell-us). |
| Any other adapter, the Realtek RTL8852BU included | | ❌ | HowlDrop for Windows has a driver for the MT7612U only. Other chips work with the Home Assistant add-on (below). [Test yours](#test-yours-and-tell-us) anyway: the report tells us which driver to write next. |
| **Your PC's or laptop's built-in Wi-Fi** (any brand) | | ❌ | Windows' own Wi-Fi drivers can't do AWDL. Use a USB MT7612U. |

You don't need a Bluetooth adapter: HowlDrop wakes the iPhone with the PC's
own Bluetooth when sending to it (receiving doesn't need Bluetooth at all),
and whatever else is connected to it stays connected.

### On Home Assistant, Raspberry Pi or Linux

Here the radio runs on the machine itself, so **built-in cards can work
too**. The add-on uses the radio helper from
[Owly](https://github.com/t4t5/owly) (formerly omdrop-owl), and most of this
list is from its users' tests. Many built-in
cards can only stay on 2.4 GHz channel 6, where the iPhone comes by only
now and then: they work, but slowly. The MT7612U follows the phone to 5 GHz,
so it's the one we recommend here too.

| Adapter (chip) | | Speed | Notes |
|---|---|---|---|
| MediaTek **MT7612U** (USB) | ✅ | fast (5 GHz): 30 MB in 14 s | Tested here on Home Assistant OS (an Intel NUC). Raspberry Pi kernels have its driver too (untested). |
| MediaTek MT7925 (built-in) | ✅ | 5 GHz | Keeps your Wi-Fi connected while it works ("shared" mode). |
| Realtek RTL8852AE, RTL8852BE, RTL8852CE (built-in) | ✅ | varies | Shared mode, opt-in (the RTL8852CE also works in exclusive mode). |
| Realtek RTL8852BU (USB) | ✅ | 5 GHz | As a second card. |
| Realtek RTL8822BE (built-in) | ✅ | slow (channel 6): 6.4 MB in 110 s | |
| Intel 9260, 9560, AX200, AX201, AX210, AX211, BE200, BE (Panther Lake) | ✅ | slow (channel 6): 2.4 MB in 17 s | Your Wi-Fi drops while HowlDrop is on ("exclusive" mode). |
| Realtek RTL8188FTV (USB) | 🟠 | slow (2.4 GHz only) | Works partly, opt-in. 2.4 GHz only, so even working it's slow. |
| MediaTek MT7921, MT7922 | 🟠 | | Works now and then: the driver sends at 1 Mbit/s and nothing on 5 GHz. A driver fix could change that. |
| MediaTek MT7927 | 🟠 | | Sends, but receiving times out. |
| Realtek RTL8821CE | 🟠 | | Syncs, but no transfer. |
| Qualcomm WCN6855, WCN7850 | 🟠 | | Can't make the listening interface, or hears nothing. Driver limits. |
| Intel Wireless-AC 7265 | ❌ | | Its firmware crashes on every start, and gets no more updates. |
| Intel Wireless-AC 8260 | ❌ | | Tested here: its firmware resets on 5 GHz and no transfer finished on channel 6. It gets no more updates. |
| Intel Wireless-AC 8265 | ❌ | | Its firmware resets about 10 s after starting; the phone was never heard. Same generation as the 8260, no more updates. |
| ESP8266 boards (Wemos D1 mini, NodeMCU) | ❌ | | Can't capture whole frames or send AirDrop's, and has no Bluetooth. (An ESP32-S3 can be a radio for a PC: that's a different project, [omdrop-windows](https://github.com/ZyphrZero/omdrop-windows), 2.4 GHz only.) |

The full per-driver details are in Owly's
[supported hardware](https://github.com/t4t5/owly#supported-hardware)
table and this repo's [hardware notes](hardware-notes.md).

## Test yours and tell us

Got a USB Wi-Fi adapter? Test it in a few minutes and send us how it went,
whether it works or not. Every report helps the next person choose, and
tells us which chips to write a driver for next.

On the Windows PC with the adapter plugged in, open PowerShell (Start, type
`powershell`, Enter; no administrator needed) and paste:

```powershell
irm https://raw.githubusercontent.com/MrSco/howl-drop/main/tools/test-adapter.ps1 | iex
```

It:

1. Finds the adapter and tells you whether HowlDrop for Windows can drive it.
2. If it can, walks you through a photo from the iPhone to the PC and one
   back, with HowlDrop doing the AirDrop (it helps you get HowlDrop running
   if it isn't), and times both.
3. Writes a report to your Desktop and opens a GitHub issue with it filled
   in. Nothing is sent until you press **Create** there.

The report has the adapter's name, USB ID and Windows driver, the Windows
and HowlDrop versions, and how each step went. It has no names (the PC's,
the iPhone's or yours), serial numbers, addresses or file names. Read
[the script](../tools/test-adapter.ps1) first if you like.

On Home Assistant or Linux, a card whose Linux driver Owly's table lists
might work: open a
[Wi-Fi adapter report](https://github.com/MrSco/howl-drop/issues/new?template=adapter-report.yml)
and fill it in by hand, with the `Wi-Fi card:` line from the top of the
add-on's log.
