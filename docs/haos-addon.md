# A local add-on on this Home Assistant OS box

**Yes, a local add-on can run the AirDrop userspace on this NUC without replacing Home Assistant OS.** The condition is an add-on you install yourself, with protection mode off, sharing the host network and the host D-Bus, allowed to create a TUN device, and with NetworkManager told to leave `wlan0` alone.

That start does not put the NUC on an iPhone AirDrop sheet. The Wireless-AC 8260 still has no such result, so the add-on has no file or clipboard text to forward to `owldrop.exe`.

Ethernet stays the uplink. Wi-Fi is unused today, which is what exclusive mode needs: the I219-LM keeps Home Assistant and the forwarder on the LAN, and the 8260 is only the AirDrop radio for the window.

## What a local add-on is allowed to take

A folder you add under Settings, Add-ons, is not reviewed by the store. The supervisor still refuses the dangerous options until protection mode is turned off for that add-on. With protection mode off, the manifest can ask for:

| Access | Why this stack needs it |
|---|---|
| `host_network: true` | The container must see the host’s `wlan0` and `phy`. nl80211 and the IBSS cell live in that network namespace. Ethernet is in the same namespace, so `owldrop-bridge` reaches the PC on UDP 47631 and TCP 47632 without a published port. |
| `privileged: NET_ADMIN` and `NET_RAW` | `iw` creates the IBSS cell. OWL injects frames and adds IPv6 neighbors. |
| `devices: ["/dev/net/tun"]` | OWL opens `/dev/net/tun` and creates `awdl0`. The generic x86-64 kernel HAOS builds from `x86_64_defconfig` includes TUN. Add-ons on this OS already open that device node. With host networking, `awdl0` appears on the host, which is where the receiver has to bind. |
| `host_dbus: true` | The wake advert is a BlueZ LE advertisement on the system bus (`LEAdvertisement1`, company id `0x004C`, type `0x05`). HAOS already runs `bluetoothd` for the host. The add-on talks to that daemon. It does not take the USB Bluetooth function away from the host. |
| `apparmor: false` | The default profile blocks the netlink and raw sockets `iw` and OWL use. Turning the profile off is what forces protection mode off. |

`SYS_MODULE` is available in the privilege list and is unnecessary here. The 8260 is already bound by the host kernel, because the interface exists and is simply unused.

`full_access` is Docker’s privileged mode. It is a broader switch than this stack needs, and the docs say not to combine it with a `devices` list. The rows above are the set that matches the calls OWL makes.

## Kernel and firmware

HAOS’s generic x86-64 image builds `iwlwifi` and lists the Wireless-AC 8260 as a supported card. The firmware package is `iwlwifi-8000C`. It was missing on early images (the NUC6 loaded no ucode, API 22 through 36). That blob has shipped since OS 6.3. A current image has `iwlwifi-8000C-36.ucode`, which is the last firmware this card will ever get.

The add-on does not load the module and does not ship the firmware. The host already did both.

## NetworkManager

The supervisor owns NetworkManager and will put a managed Wi-Fi interface back the way the UI left it. The Hass.io access-point add-on already does the counter on this OS: `nmcli device set wlan0 managed no`. The supervisor skips devices that are not managed, so it will not take `wlan0` back as an uplink and will not treat `awdl0` as one if that device is unmanaged too.

Set both unmanaged at the start of the window, from the add-on, over the host bus. A runtime `nmcli` lasts until NetworkManager restarts. A file under `/etc/NetworkManager/conf.d` survives that, and writing it means a host shell (protection mode off and the host PID namespace). The add-on does not mount the host `/etc` by itself.

One leftover: NetworkManager’s maintainer has said that marking a device unmanaged does not always make `wpa_supplicant` release it. If the supplicant is still scanning, the IBSS join fails until that process lets go. That is a start-up race, not a reason the container cannot be built.

## Bluetooth

The 8260’s Bluetooth 4.2 is a USB function on the same module (`btusb`), and HAOS’s `bluetoothd` already has it. `host_dbus` is the path for the wake advert. Legacy LE advertising does not need Bluetooth 5.

If Home Assistant’s own Bluetooth integration is already advertising on that controller, a 4.2 chip often has only one advertising slot. The wake advert then loses the slot. An idle adapter is the case that works. The add-on should not bind the USB device into the container. That would steal the controller from the host daemon.

## What still fails after the container is up

The container can start OWL, hold `wlan0`, register the advert, and run `owldrop-bridge watch` toward the PC. The 8260 is still the card from the earlier note.

`iwlwifi` does not advertise active monitor. Upstream OWL on an 8260 exits on that request, and with the Nexmon `-N` bypass an iPhone on iOS 13.4 never listed the machine ([owl#12](https://github.com/seemoo-lab/owl/issues/12)). The path that does work on Intel is exclusive IBSS, channel 6, BSSID `00:25:00:ff:94:73`, tested on AX201 and AX211 firmware. This add-on would hand the 8260 that same `iwlwifi` profile. Nobody has reported that join on `iwlwifi-8000C-36.ucode`. The cell may be refused, or it may come up and still not appear on a current iPhone.

Until a phone completes a transfer, the inbox stays empty. `owldrop.exe` has nothing to receive. Replacing Home Assistant OS is not what decides that. The firmware does.
