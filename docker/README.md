# HowlDrop without Home Assistant OS

The add-on's image also runs under plain Docker. Use it on a Raspberry Pi
with Raspberry Pi OS, or on any Linux machine, with or without Home
Assistant somewhere else. It is the same image, settings and behavior; only
the Home Assistant parts are optional.

A Raspberry Pi running **Home Assistant OS** doesn't need this folder:
install the add-on as usual. HAOS on a Pi 3, 4 or 5 carries the MT7612U
driver, and the add-on store has the aarch64 image.

## What it needs

- 64-bit Linux: Raspberry Pi OS (64-bit) on a Pi 4 or Pi 5, or x86-64. The
  Pi OS kernel includes the MT7612U driver (`mt76x2u`).
- A second Wi-Fi card for HowlDrop, such as an MT7612U USB adapter (Alfa
  AWUS036ACM), on a USB 3 port. The Pi's own Wi-Fi or Ethernet stays your
  network connection. The cards the add-on supports are the cards this route
  supports.
- Bluetooth, with `bluetoothd` running (the Pi's own works).
- NetworkManager, the default since Raspberry Pi OS Bookworm. HowlDrop takes
  the card from NetworkManager while it runs and gives it back when it stops.
  On an older system with dhcpcd, add `denyinterfaces wlan1` (your card's
  name) to `/etc/dhcpcd.conf` so nothing else holds it.
- Docker with the Compose plugin.

## Install

From a checkout of this repo:

```bash
sh docker/install.sh
```

Or download `install.sh` from this folder, read it, and run it. It sets up
`~/howldrop` (or the folder you name), pulls the image for this machine
(`ghcr.io/mrsco/howldrop-aarch64` or `-amd64`), and starts it. Run it as a
user in the `docker` group, or with `sudo`. With `sudo`, give it a folder,
or it uses root's home.

Then open the share sheet on the iPhone: HowlDrop appears once the log says
the window is up.

```bash
cd ~/howldrop
docker compose logs -f
```

## Using it

| | |
|---|---|
| Files and text | `~/howldrop/howldrop` (text in `text/`). Share it with Samba to reach it from other computers. |
| Settings | `~/howldrop/data/options.json`, the add-on's options with the same names and defaults ([the add-on documentation](../addon/DOCS.md#options)). Then `docker compose restart`. |
| The panel | `http://<this machine>:47640`. By default only this machine may open it. `HOWLDROP_PANEL_ALLOW` in `compose.yaml` takes addresses and networks, e.g. `192.168.1.0/24`. Anyone it lets in can see and delete what was sent. |
| Home Assistant | Optional. For the `howldrop_received` event and the `notify` option with Home Assistant Container or Core on any machine, set `HOWLDROP_HA_URL` (e.g. `http://homeassistant.local:8123/api`) and `HOWLDROP_HA_TOKEN` (a long-lived access token from your Home Assistant profile, Security tab) in `compose.yaml`. |
| Windows PC | `forward_to_pc` and `howldrop.exe` work as with the add-on. |
| Update | `docker compose pull && docker compose up -d`, or run `install.sh` again (your options are kept). |
| Stop | `docker compose down`. The card goes back to NetworkManager. |

The container has the same rights the add-on asks Home Assistant for: the
host network and PID namespace, `NET_ADMIN`, `NET_RAW`, `SYS_ADMIN`,
`/dev/net/tun`, the host D-Bus, and AppArmor off. It loads no driver or
firmware: the host kernel drives the card.
