# Which Wi-Fi adapter do I need?

**Short answer: get a USB Wi-Fi adapter with a MediaTek MT7612U chip, like the
Alfa AWUS036ACM.** It's the one
HowlDrop is built and tested with, on Windows and on Home Assistant, and it's
the fastest: on Windows a 14.7 MB iPhone video arrives in about 3 seconds, and
goes back to the phone in about 16.

AirDrop needs a Wi-Fi radio that can do things normal Wi-Fi doesn't
(Apple's AWDL: listening to everything on a channel, sending raw frames, and
following the iPhone onto 5 GHz). Most Wi-Fi hardware can't, or its driver
won't let it. That's why the choice matters.

## How to tell which chip you have

A product's name doesn't tell you its chip, and makers change chips between
versions. What counts is the **USB ID** (vendor:product).

- **Windows:** plug it in, open a terminal, run `usbipd list` (after
  installing [usbipd-win](https://github.com/dorssel/usbipd-win)), and read
  the `VID:PID` column. Or: Device Manager → the adapter → Properties →
  Details → Hardware Ids, and look for `VID_xxxx&PID_xxxx`.
- **Linux / Home Assistant:** `lsusb` for USB adapters, `lspci -nn` for
  built-in cards.

HowlDrop for Windows recognizes a stick by its USB ID: the MT7612U's
`0e8d:7612` (Alfa AWUS036ACM) and every other ID WSL's MT7612U driver
takes, and the RTL8852BU's. The list is `kSticks` in
[howldrop/src/wsl_text.hpp](../howldrop/src/wsl_text.hpp).

## The lists

✅ works, tested · 🟡 might work, not tested (or not supported yet) ·
🟠 tested, doesn't work now, could some day · ❌ can't work (hardware or
Windows limits)

### On Windows: AirDrop on this PC

The Windows app runs the radio in Linux inside WSL, and WSL can only borrow
**USB** devices. So on Windows it has to be a USB adapter.

| Adapter (chip) | USB ID | | Notes |
|---|---|---|---|
| MediaTek **MT7612U** (Alfa AWUS036ACM) | `0e8d:7612` | ✅ | Tested both ways: a 14.7 MB video from the iPhone in 3 s, to it in 16 s. Runs on 5 GHz. |
| MT7612U with another USB ID: Netgear A6210 (`0846:9053`), ASUS USB-AC55 (`0b05:17eb`), and the others WSL's `mt76x2u` driver takes | `045e:02e6` `045e:02fe` `0471:2126` `0471:7600` `056e:400a` `057c:8503` `0846:9014` `0846:9053` `0b05:17eb` `0b05:180b` `0b05:1833` `0e8d:7632` `2357:0137` `2c4e:0103` | 🟡 | Same chip, driver and settings as the tested one, and HowlDrop for Windows uses them as it does it. Not tried yourself? Tell us how it went. |
| Realtek **RTL8852BU** | `0bda:b832` `35bc:0108` `0586:3428` `0b05:1a62` `0bda:a85b` `0bda:b83a` `0bda:b852` `0bda:b85a` `0db0:6931` `0db0:f0c8` `2001:3327` `3574:6121` `35bc:0100` `7392:6822` | 🟡 | HowlDrop for Windows runs it (its firmware is in the Linux part), marked **untested**: in Owly's exclusive mode it worked on Linux, but nobody has tried it in WSL yet. The tray says "untested" beside the share sheet name. |
| MediaTek MT7921AU, MT7925 (USB versions) | | 🟡 | WSL has drivers, untested. Their built-in (PCIe) twins: the MT7925 works on Linux; the MT7921 only unreliably. |
| Realtek RTL8812AU, RTL8814AU, RTL8821AU, RTL8821CU, RTL8822BU, RTL8822CU | | 🟡 | WSL has drivers (rtw88), untested for AirDrop. |
| **Your PC's or laptop's built-in Wi-Fi** (any brand) | | ❌ | Windows' own Wi-Fi drivers can't do AWDL, and WSL can't borrow a built-in card. Use a USB adapter. |
| Wi-Fi 6/6E/7 USB adapters with no Linux driver | | ❌ | Nothing in WSL can drive them. If WSL has no driver for the chip, it can't work. |
| 2.4 GHz-only adapters (e.g. MT7601U, RTL8188/RTL8192CU, RTL8187, ZD1211) | | ❌ | The iPhone moves transfers to 5 GHz. On 2.4 GHz (channel 6) it only stops by now and then, so transfers crawl or fail: a 1.1 MB photo took 27 s there, and about 1 s on 5 GHz. |

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

## Something not listed?

If it's a USB adapter whose chip WSL has a driver for (the 🟡 rows), it might
work. Open an issue with its USB ID, and with `radio.log` and `radio.log.1`
from `%LOCALAPPDATA%\HowlDrop` if HowlDrop tried it. Your report helps the
next person, whether it works or not.
