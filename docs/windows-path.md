# Getting real AirDrop frames from a Windows-only machine

The laptop's own Wi-Fi cannot run this. The one path that can still put AWDL frames on the air is a Linux guest in VirtualBox or VMware, with a **USB MediaTek MT7612U** dongle and a **separate USB Bluetooth** adapter passed straight into that guest, running the unmerged active-monitor work in [omdrop-owl pull request 13](https://github.com/t4t5/omdrop-owl/pull/13). That report is one person's bare-metal test, not a VM test, so treat it as fragile. Nothing on Windows itself speaks AWDL.

This note does not repeat [windows-feasibility.md](windows-feasibility.md). Same repo, `t4t5/omdrop-owl` at `8c385402`, plus that open pull request (`4cc937de`, opened 2026-10-11, not merged).

## Recommended path

1. Buy a USB stick whose chip is **MT7612U** and whose USB id is **`0e8d:7612`**. Alfa **AWUS036ACM** is that chip. Alfa **AWUS036ACH** is a Realtek RTL8812AU and will not do this job. Check `0e8d:7612` on the box or with `lsusb` after it is in the guest.
2. Buy a second, ordinary USB Bluetooth adapter that Linux's `btusb` drives. The pull request's send to an iPhone used the BlueZ Continuity advert (`ble-airdrop-adv.py`) as the wake. Wi-Fi-only is not the setup they reported.
3. Install a normal Linux guest (Ubuntu or Debian is enough; the Omarchy bar is optional). Give it a current kernel. The reported success was kernel **7.2.9**. `mt76x2u` has been in-tree since 4.19, so the driver is there; the active-monitor behavior was only reported on 7.2.9.
4. Use **VirtualBox** (Extension Pack, USB 3.0 xHCI controller, a USB filter that captures the dongle) or **VMware Workstation** (USB arbitrator running, USB 3.x controller, VM → Removable Devices → Connect). The guest must own both USB devices. Windows must not keep a driver attached to them.
5. Inside the guest, build **pull request 13**, not the released 0.1.0 package. The release still lists `mt76x2u` as untested and has no active-monitor mode. The pull request adds `exclusive_backend=active-monitor` and a tested `mt76x2u` line.
6. Leave the guest's internet on the virtual Ethernet adapter. This mode does not use an access point. It takes the USB radio off station mode for the window.

What that pull request actually reports, on a machine with no built-in Wi-Fi: images both ways on `mt76x2u`, kernel 7.2.9. Send used BLE wake. Receive of a 261 kB PNG completed `/Discover`, `/Ask`, and `/Upload` with HTTP 200. IBSS on that card is the wrong backend: the author says the driver rejects it, and the write-up also says an IBSS join that did come up then timed out at HTTPS `/Discover`. Active monitor is the path that passed the same send. The radio prefers channel 6, then 44 or 149 if the regulatory domain allows transmit. Set the regulatory domain in the guest (`iw reg get` / `iw reg set`); a USB MediaTek with an unset domain has shipped frames into the driver and never onto the air (upstream OWL, Archer T1U, 2019).

The built-in **MT7925** and **Intel AX** stay on Windows the whole time. VirtualBox and VMware on a Windows host pass through USB devices only. VMware's own staff answer on Workstation is that a motherboard Wi-Fi chip is PCI, and Workstation has no PCI passthrough because Windows does not offer that hook. Those cards are usable only when Linux is installed on the machine (below).

## Why this hypervisor, and not the others

AWDL here is pinned to one channel for the whole window (`-S pin`), which is kinder to a VM than classic OWL channel hopping. It is still a 16 TU airtime protocol with hardware ACKs. USB passthrough that is a real host-controller handoff (VirtualBox, VMware) is the least-bad virtualization. USB-over-IP adds another queue.

| Host | Can the guest see a USB Wi-Fi stick? | Use it? |
|---|---|---|
| VirtualBox | Yes, with the Extension Pack and xHCI. The device is detached from Windows. | Yes. First choice. |
| VMware Workstation | Yes, built-in USB 2/3, arbitrator service. Same exclusive attach. | Yes. Equal first choice. |
| QEMU on Windows | Yes, `usb-host` through libusb, if the device is not claimed by Windows. | Same dongles, clumsier attach. No reason to prefer it. |
| Hyper-V | No native "capture this USB device" for a Linux guest. Enhanced Session USB redirection is for Windows guests and does not give a Linux guest a raw Wi-Fi interface. [usbipd-win](https://github.com/dorssel/usbipd-win) can attach the stick over USB/IP (`usbip attach` from the guest). Discrete Device Assignment can hand a whole PCIe device to a guest on server-class hardware with ACS; it is not a way to donate a laptop's in-use Wi-Fi chip. | Only as USB/IP, which is the WSL case below. |
| WSL2 | Yes, and this is the part the earlier note left thin: `usbipd attach --wsl --busid …` does put a USB device on the WSL USB bus. Microsoft documents it. While it is attached, Windows cannot use it. | The stick can show up in `lsusb`. The default WSL kernel still has `CONFIG_MT76x2U` **off** (and `MT7610U`, `ATH9K_HTC`, `RTW88`, `RTW89` off). `iw` will not create `wlan0` until you boot a WSL kernel you built with `mt76x2u` enabled, and you must pin that kernel in `.wslconfig` or the next `wsl --update` drops it. `btusb` is already built, so the Bluetooth stick can attach on the stock kernel. USB/IP plus a custom kernel is a worse copy of the VirtualBox path, and nobody has reported AWDL surviving that extra latency. The internal PCIe radio is still absent. |

A guest that only has the virtual Ethernet adapter has nothing for `omdrop-discoverable` to find. Bridged or NAT networking does not create `phy80211`.

## Which dongles can speak AWDL

"Monitor mode" on a wardriving stick is not enough. OWL needs frames injected as 802.11 with radiotap, and the firmware has to ACK the phone's unicast (active monitor, or an IBSS cell that this helper joins to BSSID `00:25:00:ff:94:73`). omdrop-owl then has to have a profile for that driver.

| Chip | Example | In this project |
|---|---|---|
| **MT7612U** (`mt76x2u`, `0e8d:7612`) | Alfa AWUS036ACM | The only USB device with a both-directions report, and only on unmerged PR 13, active-monitor, channel 6, plus BLE. |
| MT7610U (`mt76x0u`) | TP-Link Archer T1U | Upstream OWL exchanged pings with a Mac on channel 44 in 2019 after the regulatory domain was set. Not an omdrop-owl result, and not a current-iPhone result. |
| AR9271 (`ath9k_htc`) | Alfa AWUS036NHA | 2.4 GHz only. An old OWL report got a Mac to list the peer on channel 6 with `-N`. No 5 GHz, no omdrop-owl profile. |
| AR9280 (`ath9k`) | OWL's original mini-PCIe card | The chip upstream OWL was written on. It is not a USB dongle, so a Windows laptop cannot hand it to a guest. |
| MT7921U | Alfa AWUS036AXML | `mt7921u` advertises active monitor; the mt76 maintainer has called that support buggy. omdrop-owl issue 14: PCIe MT7921 exclusive mode fails. |
| Realtek RTL8812AU / 8822BU | Alfa AWUS036ACH, many "monitor mode" sticks | Injection for aircrack, via an out-of-tree driver, not mac80211 active monitor. The mt76 maintainer's note is that no Realtek driver they know does active monitor. Wrong purchase for OWL. |
| Built-in MT7925, Intel AX201/AX211/BE | the laptop | Works when Linux owns the chip. No Windows hypervisor can see it. |

Released omdrop-owl 0.1.0 will refuse an unknown driver until `/etc/omdrop/allow-untested` is set, and the fallback profile is **shared** mode: the station must be associated, the access point must be on channel 6, 44, or 149, and `ip -4 route show default dev <that-iface>` must succeed. In a VM the default route is the virtual NIC, so shared mode fails even if the dongle is associated. Exclusive / active-monitor is the mode that matches a dongle. That mode, for `mt76x2u`, exists only on the pull request.

## Windows-native AWDL

There is no Windows program that establishes AWDL and transfers with an iPhone.

- [seemoo-lab/owl](https://github.com/seemoo-lab/owl) issue 8, still open, labeled wontfix since 2020. The maintainers said a Windows port is a new `io` and `netutils`, because nl80211 is what sets active monitor and the channel. No such backend was merged.
- [seemoo-lab/opendrop](https://github.com/seemoo-lab/opendrop) states it runs on macOS or on Linux with OWL. Issue 108 (Windows), still open as of 2025, dies at "interface `awdl0` does not have an IPv6 address" under WSL, which is the missing radio.
- [AirDrop Anywhere](https://bakedbean.org.uk/posts/2021-06-airdrop-anywhere-part-4/) (2021, .NET) implements the AirDrop HTTP conversation and then, in the author's words, a proxy that must run on a machine that already has AWDL or OWL. Npcap's raw-802.11 option was considered and set aside: typical Windows adapters do not inject. The series is marked work in progress and does not speak AWDL from Windows.
- Apps that move files over the local network (LocalSend and the rest of that category) are a different product. They do not join AWDL, and an iPhone's AirDrop sheet will not see them.

## Last resort: install Linux

Dual boot, or replacing Windows, is installing Linux. It is the only way the **built-in** card participates.

- MediaTek MT7925 (`mt7925e`): shared mode on the released package. Wi-Fi stays up. The access point has to be on channel 6, 44, or 149.
- Intel AX201 / AX211 / Panther Lake (`iwlwifi`): exclusive mode on the released package. Wi-Fi is down while the machine is visible, then restored. Channel 6. Slow (a 2.4 MB photo was 10–17 s on an AX201).

That is bare metal mac80211, which is the configuration the released package was tested on. A USB dongle is unnecessary there. It is also no longer "running on Windows."
