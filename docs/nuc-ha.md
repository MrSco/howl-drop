# Can the Skull Canyon NUC replace the Pi?

**No, while this machine runs Home Assistant OS.** The condition that decides it is a normal Debian or Ubuntu host, with Home Assistant beside that host. On Home Assistant OS the AirDrop stack cannot be installed. On Debian or Ubuntu the NUC has the right ports, and its Wi-Fi driver is the one omdrop-owl already uses for Intel, but this particular firmware has no AirDrop result.

The NUC still has to sit in radio range of the phone. Ethernet to the Windows PC is the forward path, and `owldrop.exe` is already the listener.

## The radio in the NUC6i7KYK

Intel’s specification for the NUC6i7KYK lists one wireless module and one Ethernet chip:

| Part | What it is |
|---|---|
| Wi-Fi | Intel Dual Band Wireless-AC 8260. 802.11ac, 2.4 and 5 GHz, 2x2. Driver `iwlwifi`, firmware family `iwlwifi-8000C`. The last firmware generation for the 8260 is Core33, `iwlwifi-8000C-36.ucode`. |
| Bluetooth | Bluetooth 4.2 on that same module. Linux sees it as a USB function (`btusb`), separate from the PCIe Wi-Fi function. |
| Ethernet | Intel I219-LM, Gigabit. This is the cable to the PC. |

The 8260 is not an AX201. Both use `iwlwifi`. The firmware is not the same file, and the 8260’s line is frozen.

## What omdrop-owl actually does on Intel

Pull request 13 does not apply. That change is active monitor for the USB MT7612U (`mt76x2u`). The Intel line in released omdrop-owl is exclusive mode, keyed by the driver name `iwlwifi`, and the notes name an AX201, an AX211, and Panther Lake. The AX200 is marked untested. The 8260 is not named.

Exclusive mode exists because associated Intel firmware delivers no AWDL frames. The helper does not use active monitor. It builds an IBSS cell on the station MAC, joined to AWDL’s BSSID `00:25:00:ff:94:73`, HT20, channel 6. The cell will not start on 5 GHz, so 44 and 149 are out. A 2.4 MB photo on an AX201 took 10–17 seconds. Wi-Fi to an access point is down for that window. Bluetooth on the same module sends the wake advert. That is the result to compare against.

Active monitor is a different flag, and `iwlwifi` does not advertise it. Upstream OWL sets `NL80211_MNTR_FLAG_ACTIVE` and exits with “operation not supported” on this driver. [owl#12](https://github.com/seemoo-lab/owl/issues/12) is that failure on an Intel Dual Band Wireless-AC 8260: the daemon would start only with `-N`, and an iPhone on iOS 13.4 neither saw the machine nor was seen by it. `-N` is the Nexmon bypass. It is the wrong flag here.

The driver does advertise IBSS, including HT IBSS, for every `iwlmvm` device, which includes the 8260. So the AX recipe is the one this card would be handed: same `iwlwifi` profile, channel 6, BSSID `00:25:00:ff:94:73`. Nobody has reported that join on `iwlwifi-8000C-36.ucode`. The firmware cannot pick up later AX fixes. Treat a sheet on this NUC as untested, not as the AX201 result.

Bluetooth 4.2 is enough for the legacy Continuity advert (company id `0x004C`, type `0x05`). The AX201 send used the card’s own Bluetooth while Wi-Fi was exclusive. The 8260 is the same split, PCIe Wi-Fi plus USB Bluetooth, and that pairing is also untested on this module.

## Home Assistant OS, and the other installs

Do not guess which one this NUC is. These are the four common cases:

| Install | Can it run OWL / omdrop-owl? |
|---|---|
| **Home Assistant OS** (the appliance image) | No. The host is a read-only system with no apt. Add-ons are containers. The supervisor owns NetworkManager, so it will fight an IBSS takeover of `wlan0`. An OS update replaces the host. There is no supported place to build OWL, libnl, and the BlueZ advert script. |
| **Home Assistant Supervised** on Debian | Yes, on the Debian host. Home Assistant stays under the supervisor. The radio packages are ordinary Debian packages and a root helper. Exclusive mode takes Wi-Fi only. Ethernet stays up for Home Assistant and for the forwarder. |
| **Home Assistant Container** on Debian or Ubuntu | Yes. The host is a normal Linux install. The container does not need the Wi-Fi. |
| **Home Assistant Core** (a venv) on Debian or Ubuntu | Yes. Same as a normal Linux install. |

Home Assistant OS is the image people write straight onto a NUC. That case cannot be the middleman without replacing the operating system with Debian or Ubuntu and running Home Assistant as Supervised, Container, or Core.

On a Debian or Ubuntu host, use the released omdrop-owl `iwlwifi` profile, not pull request 13. Exclusive mode drops Wi-Fi for the window, so the NUC’s LAN has to be the I219-LM. If Home Assistant’s only uplink is Wi-Fi, Home Assistant goes offline while the phone can see the machine.

## Forwarding, and where the box sits

`owldrop.exe` on the PC beacons the computer name on UDP port 47631. On the NUC, `owldrop-bridge name` reads that beacon and `omdrop name` puts it on the sheet. `owldrop-bridge watch` sends files and clipboard text to the PC. The PC must accept UDP 47631 and TCP 47632 from the LAN. Files land in `Downloads\OwlDrop`. Text from the inbox replaces the PC clipboard.

AWDL is the room the phone is in. A Skull Canyon in a closet, a rack, or another floor will not appear on the sheet even if the software is right. The Ethernet cable is what reaches the PC. The NUC does not need to be next to the PC.
