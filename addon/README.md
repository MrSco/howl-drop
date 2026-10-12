# Install OwlDrop on this Home Assistant OS NUC

The NUC keeps Home Assistant OS. This folder is a local add-on. It does not replace the operating system and it does not patch Intel firmware. The image builds OWL and the radio helper from this repo (`userspace/`, `patches/`, `pins`), whose `iwlwifi` line is exclusive IBSS. Pull request 13 is not checked out. A USB Wi-Fi stick is not used.

Ethernet on the I219-LM stays the uplink. Wi-Fi stays unused in the Home Assistant interface. The add-on takes the host's Intel iwlwifi PCI radio only while it is running. On this NUC that interface is usually `wlp2s0`. The name `wlan0` is not required.

## Copy the folder onto the NUC

Supervisor builds an add-on from its own folder only, so `tools/stage-addon.sh` assembles a self-contained one at `build/addon/owldrop/` (this folder, plus the radio stack and the `owldrop-bridge` source).

1. On the NUC, open Settings, Add-ons, Add-on store, and install either Samba share or Advanced SSH & Web Terminal if you do not already have a way onto the disk.
2. The local add-on directory is `/addons`. The Samba share of that name is the same directory.
3. From the repo root in Git Bash, stage and copy in one step: `tools/stage-addon.sh //homeassistant/addons/owldrop` (use the NUC's name or IP if `homeassistant` does not resolve). Or run it with no argument and copy `build/addon/owldrop/` there yourself. The path on the NUC is `/addons/owldrop/`.
4. In the add-on store, open the menu and choose Check for updates. That reloads local add-ons. Open OwlDrop and install the update (the version is in `config.yaml`; bump it for every change you deploy, or Supervisor keeps the old image). Start it after the update finishes. Restarting the previous container keeps the old image.

The first install compiles OWL inside the image. The supervisor build needs a network path to GitHub and takes a while. Later builds reuse that layer unless `pins` or `patches/` changed.

## Turn protection mode off

Open the OwlDrop add-on. Protection mode has to be off before it will start. The manifest asks for the host network, the host PID namespace, `NET_ADMIN`, `NET_RAW`, `SYS_ADMIN`, `/dev/net/tun`, the host D-Bus, and AppArmor off. The supervisor refuses those while protection mode is on. Host network is what puts the 8260 in this container. The host PID namespace and `SYS_ADMIN` are how the script enters that namespace when Supervisor left the container on the hassio bridge.

Leave full access off. The add-on does not request it, and it does not bind the Bluetooth USB device.

## Options

`pc_name` and `pc_host` can stay empty. `channels` stays `6`.

On the Windows PC, start `owldrop.exe` and leave it open. Allow UDP 47631 and TCP 47632 from the LAN. The add-on reads the PC's name from that beacon and puts the same name on the iPhone share sheet.

Set `pc_host` to the PC's Ethernet address only when the beacon does not reach the NUC. Set `pc_name` only when you want to force the sheet name. The name can be at most 63 characters.

`channels` is the AirDrop channels to try for the IBSS cell, in order: any of `6`, `44`, `149`, separated by spaces. Leave it at `6`. On this 8260, 0.1.12 and 0.1.13 unlocked 5 GHz and joined channel 44; the log said `Joined IBSS` and the iPhone never listed the PC. On channel 6 (0.1.11) the tile appeared. Do not use 44: on 0.2.3 the cell joined there, OWL rejected the phone's frames, and about 13 s in the 8260's firmware stopped answering and iwlwifi removed and re-probed the card (kernel log in `docs/8260-ibss-join.md`). The add-on now stops itself when that happens.

`unicast_copies` (1–5, default 3) is how many times OWL sends each packet to the phone while the phone is on channel 6 (`owl -U`, `patches/owl-09-fixed-radio-copies.patch`). OWL sends unicast without waiting for an acknowledgement, and an iPhone on 5 GHz Wi-Fi visits channel 6 one slot in sixteen, so a lost packet otherwise waits a whole schedule period. 1 is the original behaviour.

Boot is manual. Start the add-on when you want the window. Stopping it runs `omdrop-discoverable stop`, which leaves the IBSS cell and returns the station interface to a station.

## What the log says first

The log starts with the Wireless-AC 8260. It names the released exclusive profile and BSSID `00:25:00:ff:94:73`, then the channels it will try (`Channels to try, in order: 2437 MHz (channel 6).` by default). Then it prints the interface name the host assigned, the driver, the PCI id, and the firmware version `ethtool` read from that interface.

Before the join, the add-on marks the interface unmanaged and stops the host `wpa_supplicant` from holding it. `nmcli` unmanaged alone leaves that process attached. The interface that will be joined is set to type ibss while it is down. The log line `Set <name> to type ibss while it was down.` is that step. The interface is brought up after that. IPv6 is turned off on that cell so the kernel does not answer on the AirDrop BSSID. Only when `channels` names 44 or 149: before that join, an unset regulatory domain is set to US and the card scans while it is still a station, and again after the IBSS interface is up, because taking the station down can close 5 GHz again. The log line `5220 MHz (channel 44) is usable after the cell scan.` means that lock opened. The join is then tried on each channel in order: HT20, fixed frequency, BSSID `00:25:00:ff:94:73`, beacon interval 10000. A frequency the card marks disabled or No IR is skipped, and the log says so. The log line `Joined IBSS on 2437 MHz (channel 6), HT20, BSSID 00:25:00:ff:94:73.` is the default cell. The add-on then writes that channel to `/etc/omdrop/exclusive-channels` and `1` to `/etc/omdrop/keep-cell`, so `omdrop-discoverable` keeps this cell instead of taking it down to join again, and pins OWL to the same channel. If `awdlibss0` cannot be type ibss, it is deleted first and the same steps run on the station alone. Other interfaces on that phy, including a P2P device NetworkManager left up, are deleted before the bring-up. One IBSS interface can come up only when it is the only interface running on the phy. A join that comes up logs that cell. If the join itself returns operation not supported after the interface is up and the supplicant has released it, the log says so in one sentence. The add-on does not switch to active monitor and does not patch Intel firmware. The unmanaged flag is a runtime `nmcli` on the host bus. A NetworkManager restart clears it until the next start. The add-on does not write the host `/etc`; `/etc/omdrop` is the container's.

The receiver will not create its certificate while it is root. The first start writes a self-signed pair into the add-on data and reuses it after that. `awdl0` needs a link-local address that is not still tentative. The log line `awdl0 IPv6 address is <addr> and it is ready.` is that step. A tentative address is added again without duplicate-address detection. The image also installs the plugin's Apple root at `/opt/share/certs/apple_root_ca.pem`, which is the path the receiver checks. The log line `The AirDrop receiver is listening on awdl0 port 8771.` means that bind succeeded.

Files the phone sends land in the add-on data folder `incoming`, then go to `Downloads\OwlDrop` on the PC. A UTF-8 file in `incoming/text` becomes the PC clipboard.

## After the window is up

Before the receiver starts, `awdl0`'s route gets a 20 ms TCP retransmit floor instead of 200 ms (`TCP on awdl0 resends after 20 ms`). OWL sends each unicast frame once and the phone visits channel 6 only briefly, so lost segments are normal there; the default floor stretched the `/Discover` TLS handshake to seconds and the phone hung up before the answer arrived.

Then the log follows the radio, because OWL's and the announcer's own logs stay inside the container:

- `OWL: ... add peer <mac>` means OWL synced with a phone. None at all means the radio never heard it.
- `<time> TCP <state> [<phone>]:<port>: rto:â€¦ rtt:â€¦ retrans:â€¦` is an AirDrop connection, printed when it changes. High `rto`, `retrans` or `bytes_retrans` is loss on the air.
- `Radio check:` every 30 s, with the peer count, the helper's status, and the announcer's last lines.

A `POST /Discover ... 200` with no tile on the phone, and `retrans` on that connection, means the answer did not reach the phone in time.
