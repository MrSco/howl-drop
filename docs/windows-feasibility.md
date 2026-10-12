# Can omdrop-owl run on Windows?

**No.** AirDrop on the laptop's own Wi-Fi is not possible here, either natively or under WSL. Stay on Linux. A native port is a new radio stack, and the cards this project supports do not expose the operations it needs on Windows. WSL can compile the userspace and still cannot see the built-in radio.

Read-only look at [t4t5/omdrop-owl](https://github.com/t4t5/omdrop-owl) at `8c385402` (`Publish omdrop-owl to AUR`, tag lineage 0.1.0). Nothing in the tree was changed.

## What it is

omdrop-owl is the **radio half** of [omdrop](https://github.com/brentkearney/omdrop-plugin): AirDrop between an iPhone and a Linux laptop whose Wi-Fi is not Apple's Broadcom chip. It does not speak AirDrop by itself. It brings up AWDL (Apple Wireless Direct Link), then omdrop's sender and receiver use that link.

It builds three things into `/usr/lib/omdrop`:

| Piece | Source | Role |
|---|---|---|
| `owl` | [jedbillyb/owl](https://github.com/jedbillyb/owl) at `832d70f8`, plus `patches/owl-*.patch` | Userspace AWDL daemon. Injects 802.11 frames on a monitor interface and creates a TAP device `awdl0`. |
| `omdrop-discoverable`, `owl-profiles` | this repo | Root helper. Creates the monitor vif (and, on Intel, an IBSS cell), starts OWL, announces `_airdrop._tcp`. |
| `send-to-peer`, `airdrop-send.py`, `ble-airdrop-adv.py`, `awdl-airdrop-adv.py`, `awdl-mdns-respond.py`, and small Python modules | [omdrop-awdl](https://github.com/brentkearney/omdrop-awdl) at `534f91a5`, copied unmodified | Transfer, Apple Continuity BLE wake, mDNS on `awdl0`. |

Runtime dependencies from the Arch `PKGBUILD` and the Debian package script: bash, Python, iproute2, iw, NetworkManager, polkit/pkexec, libpcap, libev, libnl, BlueZ, python-dbus, python-gobject, python-zeroconf, python-ifaddr, Pillow, keyutils. Build needs git, cmake, gcc, and those three libraries' headers. Packaging is pacman (`makepkg`) and a `.deb`. CI builds inside an Arch container. The UI is the Omarchy plugin, turned on from the bar, and is a separate repo.

Supported radios are Linux mac80211 drivers, tested on Linux only: MediaTek `mt7925e` in shared mode (Wi-Fi stays up), Intel `iwlwifi` (AX201, AX211, Panther Lake) in exclusive mode (Wi-Fi is dropped for the window). Shared mode needs the AP on channel 6, 44, or 149.

## OS assumptions that matter

The helper is a Linux radio state machine. These are load-bearing, not packaging details:

- **mac80211 / nl80211.** It finds a station by walking `/sys/class/net/*/phy80211` and drives it with `iw`: add a monitor interface, set its MAC, and on Intel create an IBSS (`awdlibss0`) joined to AWDL's BSSID `00:25:00:ff:94:73` as HT20 with a 10 s beacon. Some AX211s cannot add that vif, so the station itself is switched to IBSS and switched back on stop.
- **Monitor injection while the firmware still ACKs.** Shared mode injects from a monitor vif that wears the station MAC; the station ACKs the phone. Exclusive mode exists because associated Intel firmware delivers no AWDL frames. OWL opens that interface with libpcap (`DLT_IEEE802_11_RADIO`) and injects radiotap frames.
- **Linux TAP and netlink.** OWL opens `/dev/net/tun`, `TUNSETIFF` with `IFF_TAP`, then uses libnl for `nl80211` (channel, interface type) and permanent IPv6 neighbors on `awdl0`. The non-Apple half of `daemon/io.c` and `daemon/netutils.c` is behind `#ifndef __APPLE__` and includes `linux/if_tun.h` and `linux/nl80211.h`. libnl is required off macOS. The macOS side is CoreWLAN, which is Apple's own Wi-Fi framework, not a portable backend.
- **NetworkManager and polkit.** `awdl0`, `mon0`, and `awdlibss0` are marked unmanaged. The helper uses `nmcli` to drop and restore the station. omdrop starts it through `pkexec`. The post-install hook reloads NetworkManager via `systemctl`.
- **BlueZ.** `ble-airdrop-adv.py` registers an LE advertisement on `org.bluez` over D-Bus (Apple company id `0x004C`, Continuity type `0x05`). That advert is what wakes a Mac to publish AirDrop.
- **`AF_PACKET`.** The mDNS announcer and responder open raw packet sockets on `awdl0`.
- **Other Linux surfaces.** `/proc/<pid>/cmdline`, `/run/omdrop-owl`, `/etc/omdrop`, `flock`, `sysctl` for IPv6 DAD, mt76 debugfs under `/sys/kernel/debug/ieee80211/.../mt76` for runtime PM. Profiles are keyed by the kernel driver name (`iwlwifi`, `mt7925e`).

GPU and audio are unused. There is no Windows, WSL, or systemd-unit code in this repo. OWL's own README says virtual machines and WSL are unsupported because the daemon needs the Wi-Fi card, with USB or PCIe passthrough as the only exception it names. Upstream [seemoo-lab/owl#8](https://github.com/seemoo-lab/owl/issues/8) is a Windows request labeled wontfix: a Windows port means new `io` and `netutils` implementations, and nl80211 has no counterpart.

## What a native Windows port would break

The userspace AirDrop logic (HTTP `/Discover`, `/Ask`, `/Upload`, identity, hashes) could move. The radio path cannot be recompiled onto Windows.

| Layer | What it does today | On Windows |
|---|---|---|
| Virtual interfaces | `iw phy interface add` monitor and IBSS on the same phy | The WLAN API and NDIS miniports do not offer a second monitor vif, an IBSS with a chosen BSSID, or a per-vif MAC. Npcap's monitor mode covers a few older chipsets. Intel and MediaTek Windows drivers for the tested cards do not. |
| Frame injection and ACKs | libpcap inject on a mac80211 monitor iface; firmware ACKs | No radiotap injection API on these drivers. Active monitor mode (hardware ACKs) is the reason exclusive mode exists, and Windows has no equivalent. |
| OWL daemon | `/dev/net/tun`, libnl, nl80211, netlink neighbors | libnl is Linux-only. Npcap can stand in for libpcap capture. A TAP would be Wintun or TAP-Windows plus a new neighbor-table path. That is a daemon rewrite, and it still has no radio to bind. |
| Helper | bash, `iw`, `ip`, `nmcli`, sysfs, debugfs, pkexec | The whole script is the Linux state machine. WLAN AutoConfig does not stand in for NetworkManager's unmanaged-device model. |
| BLE wake | BlueZ `LEAdvertisement1` | WinRT `BluetoothLEAdvertisementPublisher` is a different stack, with tighter payload and address control. The Continuity advert has to be reimplemented and may still be rejected. |
| mDNS on `awdl0` | `AF_PACKET` | Windows has no `AF_PACKET`. |
| Package and UI | pacman, deb, polkit policy, NetworkManager `conf.d`, Omarchy bar | No installer, no service, no UI. The plugin is a Linux bar, not part of this repo. |

**How hard.** Packaging, paths, and the Python transfer tools are the small part. The port that would make AirDrop work is a new 802.11 backend plus a new BLE backend, against firmware that on Linux already refuses this traffic unless mac80211 is allowed to build extra vifs. The cards in the support table (MT7925, AX201, AX211, Intel BE) are the ones Windows will not open up. Treat a native port as a different project with a high chance of never passing frames on the hardware this repo exists for.

## What WSL would and would not solve

WSL2 is a Hyper-V VM. The host Wi-Fi stays a Windows NDIS device. The guest gets a virtual Ethernet (`eth0` under NAT or mirrored networking). `iw` finds no `phy80211`, and OWL fails where it always has in WSL: netlink is up, nl80211 has no wireless phy ("Failed to connect to generic netlink" / no nl80211 family once there is no cfg80211 device).

Microsoft's WSL 6.6 kernel config does build `CFG80211`, `MAC80211`, and `IWLWIFI`/`IWLMVM` as modules, and it builds `BT` and `BT_HCIBTUSB`. Those modules do not attach the laptop card. The Intel and MediaTek Wi-Fi functions are PCIe, and WSL has no PCIe passthrough, so `iwlwifi` loads against an empty bus. `CONFIG_MT7921E`, `MT7921U`, and the other mt76 USB/PCIe options are off, as are `ATH9K_HTC` and `RTW88`/`RTW89`. A module that is not built, or that has no device, does not create `wlan0`.

| | WSL |
|---|---|
| Compiling OWL, running the bash helper and Python tools | Works, inside the distro, once libev, libpcap, and libnl are installed. |
| systemd, NetworkManager, polkit, BlueZ *inside the guest* | Can be installed. They manage guest interfaces only. |
| Built-in Wi-Fi (MT7925, AX201, AX211, the whole support table) | Stays on Windows. Monitor vif, IBSS, injection, and firmware ACKs do not cross the VM boundary. |
| Built-in Bluetooth | Also stays on Windows, so the BlueZ Continuity advert has no controller. `usbipd-win` can sometimes attach the USB Bluetooth function of a combo card; that removes it from Windows and still leaves Wi-Fi behind. |
| GUI | This repo has no GUI. WSLg does not move the radio. The Omarchy bar does not run on Windows. |
| Filesystem | The scripts expect `/usr/lib/omdrop`, `/sys`, `/proc`, and `/run`. A checkout on `/mnt/c` does not provide those. |
| USB Wi-Fi passthrough | The only shape OWL's README allows. It needs `usbipd-win`, a dongle whose mac80211 driver is actually built (the default WSL kernel omits the mt76 and Atheros USB drivers), and a second Bluetooth controller in the guest for the wake advert. The tested machines are PCIe cards, so this does not cover them. It is a Linux box attached to Windows, not Windows support. |

## Recommendation

**Possible: no, for the machine this project is for.** Use Linux on the metal, which is the only place mac80211 owns the MT7925 or Intel card.

**WSL vs native:** neither. WSL does not solve the device. A native port does not have an API to port to. The blockers, in order, are:

1. Windows drivers for the supported cards will not create a monitor or IBSS interface, inject 802.11, or ACK AWDL frames.
2. OWL's Linux I/O is nl80211 plus `/dev/net/tun`. Upstream already declined a Windows backend.
3. The BLE wake path is BlueZ, and the announcement path is `AF_PACKET` on `awdl0`.
4. WSL2 never receives the built-in PCIe radio, and its kernel config omits the USB Wi-Fi drivers that could have been a passthrough exception.
