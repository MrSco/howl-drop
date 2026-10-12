# Install OwlDrop on this Home Assistant OS NUC

The NUC keeps Home Assistant OS. This folder is a local add-on. It does not replace the operating system and it does not patch Intel firmware. The image builds the released omdrop-owl tree (`8c38540267641ba2dcdf16b0ae115ac71f11be94`), whose `iwlwifi` line is exclusive IBSS. Pull request 13 is not checked out. A USB Wi-Fi stick is not used.

Ethernet on the I219-LM stays the uplink. Wi-Fi stays unused in the Home Assistant interface. The add-on takes the host's Intel iwlwifi PCI radio only while it is running. On this NUC that interface is usually `wlp2s0`. The name `wlan0` is not required.

## Copy the folder onto the NUC

1. On the NUC, open Settings, Add-ons, Add-on store, and install either Samba share or Advanced SSH & Web Terminal if you do not already have a way onto the disk.
2. The local add-on directory is `/addons`. The Samba share of that name is the same directory.
3. Copy this whole folder there so the path on the NUC is `/addons/owldrop/` and that directory contains `config.yaml`, `Dockerfile`, `run.sh`, and `owldrop/`. Copy over the existing folder when you are updating.
4. In the add-on store, open the menu and choose Check for updates. That reloads local add-ons. OwlDrop appears under local add-ons. Open OwlDrop and install the 0.1.13 update. That rebuilds the image and Supervisor creates a new container in the host network namespace. Start it after the update finishes. Restarting the previous container keeps the old image.

The first install compiles OWL inside the image. The supervisor build needs a network path to GitHub and takes a while.

## Turn protection mode off

Open the OwlDrop add-on. Protection mode has to be off before it will start. The manifest asks for the host network, the host PID namespace, `NET_ADMIN`, `NET_RAW`, `SYS_ADMIN`, `/dev/net/tun`, the host D-Bus, and AppArmor off. The supervisor refuses those while protection mode is on. Host network is what puts the 8260 in this container. The host PID namespace and `SYS_ADMIN` are how the script enters that namespace when Supervisor left the container on the hassio bridge.

Leave full access off. The add-on does not request it, and it does not bind the Bluetooth USB device.

## Options

`pc_name` and `pc_host` can stay empty.

On the Windows PC, start `owldrop.exe` and leave it open. Allow UDP 47631 and TCP 47632 from the LAN. The add-on reads the PC's name from that beacon and puts the same name on the iPhone share sheet.

Set `pc_host` to the PC's Ethernet address only when the beacon does not reach the NUC. Set `pc_name` only when you want to force the sheet name. The name can be at most 63 characters.

Boot is manual. Start the add-on when you want the window. Stopping it runs `omdrop-discoverable stop`, which leaves the IBSS cell and returns the station interface to a station.

## What the log says first

The log starts with the Wireless-AC 8260. It names the released exclusive profile, BSSID `00:25:00:ff:94:73`, and the fall through to channel 6 after channels 44 and 149. It names firmware `iwlwifi-8000C-36.ucode` and says that join has no report on this card. Then it prints the interface name the host assigned, the driver, the PCI id, and the firmware version `ethtool` read from that interface.

Before the join, the add-on marks the interface unmanaged and stops the host `wpa_supplicant` from holding it. `nmcli` unmanaged alone leaves that process attached. The interface that will be joined is set to type ibss while it is down. The log line `Set <name> to type ibss while it was down.` is that step. The interface is brought up after that. IPv6 is turned off on that cell so the kernel does not answer on the AirDrop BSSID. Before that join, an unset regulatory domain is set to US and the card scans while it is still a station. It scans again after the IBSS interface is up, because taking the station down can close 5 GHz again. An unreadable channel list is not treated as usable. The log line `5220 MHz (channel 44) is usable after the cell scan.` means that lock opened. `disabled or No IR after the cell scan` means it did not. The same join is then tried at 5220 MHz (channel 44), then 5745 MHz (channel 149), then 2437 MHz (channel 6): HT20, fixed frequency, BSSID `00:25:00:ff:94:73`, beacon interval 10000. A frequency the card marks disabled or No IR is skipped, and the log says so. The log line `Joined IBSS on 5220 MHz (channel 44), HT20, BSSID 00:25:00:ff:94:73.` means the phone's channel came up. The same line with 5745 MHz is channel 149. The line with 2437 MHz means 5 GHz was refused and the cell is on channel 6 again. If `awdlibss0` cannot be type ibss, it is deleted first and the same steps run on the station alone. Other interfaces on that phy, including a P2P device NetworkManager left up, are deleted before the bring-up. One IBSS interface can come up only when it is the only interface running on the phy. A join that comes up logs that cell. If the join itself returns operation not supported after the interface is up and the supplicant has released it, the log says so in one sentence. The add-on does not switch to active monitor and does not patch Intel firmware. The unmanaged flag is a runtime `nmcli` on the host bus. A NetworkManager restart clears it until the next start. The add-on does not write the host `/etc`.

The receiver will not create its certificate while it is root. The first start writes a self-signed pair into the add-on data and reuses it after that. `awdl0` needs a link-local address that is not still tentative. The log line `awdl0 IPv6 address is <addr> and it is ready.` is that step. A tentative address is added again without duplicate-address detection. The image also installs the plugin's Apple root at `/opt/share/certs/apple_root_ca.pem`, which is the path the receiver checks. The log line `The AirDrop receiver is listening on awdl0 port 8771.` means that bind succeeded.

Files the phone sends land in the add-on data folder `incoming`, then go to `Downloads\OwlDrop` on the PC. A UTF-8 file in `incoming/text` becomes the PC clipboard.
