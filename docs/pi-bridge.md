# iPhone AirDrop to the Windows PC

The Raspberry Pi sits near the iPhone. A USB Wi-Fi stick and the Pi's Bluetooth run the Linux AirDrop stack, and the iOS share sheet shows the Windows PC's name. When a file or shared text arrives, the Pi forwards it over Ethernet to `owldrop.exe`. The PC can be anywhere on the same LAN and does not need a Wi-Fi card. The Windows program does not create a Wi-Fi adapter.

## Hardware

- Raspberry Pi with an Ethernet port, on the same IPv4 network as the PC. The Ethernet cable is how files reach the PC. The USB stick is only the AirDrop radio.
- USB MediaTek MT7612U, USB id `0e8d:7612`. The Alfa AWUS036ACM is that stick.
- The Pi's own Bluetooth, for the AirDrop wake advert.

Do not use the Pi's built-in Broadcom Wi-Fi, and do not install Nexmon. Turn that radio off and leave Bluetooth on. On Raspberry Pi OS that is a line in `/boot/firmware/config.txt` (on older images, `/boot/config.txt`):

```
dtoverlay=disable-wifi
```

Reboot. `lsusb` should show `0e8d:7612`. The stick's driver is `mt76x2u`:

```
basename $(readlink -f /sys/class/net/wlan0/device/driver)
```

That prints `mt76x2u`. If it prints `brcmfmac`, the onboard radio is still the one that came up. `lsmod` should not list `brcmfmac`.

Power Bluetooth on if it is blocked:

```
bluetoothctl show
sudo rfkill unblock bluetooth
```

`Powered: yes` is the state you want. Do not add a separate Bluetooth dongle.

## omdrop-owl pull request 13

The radio package is [t4t5/omdrop-owl](https://github.com/t4t5/omdrop-owl) at [pull request 13](https://github.com/t4t5/omdrop-owl/pull/13). That pull request is the MT7612U support. Its profile line is:

```
mt76x2u tested exclusive station no no active-monitor
```

Exclusive mode takes the USB stick for AirDrop. Ethernet stays up. The stick does not join an access point and does not use IBSS. An active monitor interface answers the phone.

On the Pi:

```
sudo apt install build-essential cmake git pkg-config dpkg-dev \
  libev-dev libpcap-dev libnl-3-dev libnl-genl-3-dev
git clone https://github.com/t4t5/omdrop-owl.git
cd omdrop-owl
git fetch origin pull/13/head:pr-13
git checkout pr-13
tools/build-deb.sh
sudo apt install ./omdrop-owl_*.deb
```

Check the installed profile still names this stick:

```
grep mt76x2u /usr/lib/omdrop/owl-profiles
```

You want `tested`, `exclusive`, and `active-monitor` on that line.

## The AirDrop receiver and Bluetooth

Install [omdrop](https://github.com/brentkearney/omdrop-plugin) (0.16.0 or newer). The command is `bin/omdrop` in that repository. The Omarchy panel is optional; the command is what starts the receiver. It needs Python 3, the `libarchive` Python module (`python3-libarchive-c` on Debian and Raspberry Pi OS), and BlueZ, which you already used for Bluetooth.

```
sudo apt install python3 python3-libarchive-c bluez
git clone https://github.com/brentkearney/omdrop-plugin.git
cd omdrop-plugin
./bin/omdrop setup
sudo ln -sf "$PWD/bin/omdrop" /usr/local/bin/omdrop
```

`omdrop on` starts three things: the receiver, AWDL on the USB stick through `/usr/lib/omdrop/omdrop-discoverable`, and the Bluetooth wake advert that helper runs on the Pi's Bluetooth. Leave visibility on everyone so the iPhone share sheet can list this Pi:

```
omdrop visibility everyone
omdrop on forever
```

`forever` stays up until `omdrop off`. A plain `omdrop on` stops after ten minutes.

The iPhone is the sender. Open the share sheet and choose AirDrop. The tile uses the name from the next section.

## How the Pi learns the PC's name

On the PC, start `owldrop.exe` and leave that window open. It beacons the Windows computer name on UDP port 47631.

Build the forwarder from the OwlDrop source that sits next to `owldrop.exe`:

```
sudo apt install build-essential cmake g++
cmake -S . -B build
cmake --build build --target owldrop-bridge
sudo cp build/owldrop-bridge /usr/local/bin/
```

On the Pi, on the same subnet as the PC:

```
owldrop-bridge name
```

That prints the Windows computer name. Give it to omdrop, which is the name the share sheet shows (`ReceiverComputerName`):

```
omdrop name "$(owldrop-bridge name)"
```

omdrop accepts at most 63 characters. If the command refuses the name, pass a shorter one to `omdrop name`. If more than one OwlDrop PC answers, pick one:

```
owldrop-bridge name --name Office-PC
```

If the hello beacon does not reach the Pi, set the sheet name yourself to the PC's name (`omdrop name "Office-PC"`) and point the bridge at the PC's Ethernet address. `--host` skips the beacon:

```
owldrop-bridge watch --host 192.168.1.20 --inbox "$HOME/OwlDrop/incoming"
```

`owldrop name` restarts the receiver when it is already on, so the sheet picks up the new name.

## Run the forwarder

Tell omdrop to save into the bridge inbox, then leave the bridge running:

```
mkdir -p "$HOME/OwlDrop/incoming/text"
omdrop dir "$HOME/OwlDrop/incoming"
owldrop-bridge watch --inbox "$HOME/OwlDrop/incoming"
```

What the bridge does:

- A file that lands in the inbox is sent to the PC. `owldrop.exe` stores it in `Downloads\OwlDrop`. The PC keeps only the file's base name, so a name that tries to climb out of that folder still lands inside it.
- Shared text is a UTF-8 file in `incoming/text/`. The bridge sends the file's contents and the PC clipboard becomes that text. You can also send one string without the folder: `owldrop-bridge text --host 192.168.1.20 'hello from the phone'`.
- A link the receiver stores as a `.url` file is a file, so it goes to `Downloads\OwlDrop` with everything else in the inbox.
- The bridge waits until a file's size stops changing, then sends it. After the PC accepts it, the Pi copy moves to `incoming/.forwarded/`. A file the PC refuses moves to `incoming/.failed/`. If the PC is not running, the file stays in the inbox and the bridge tries again.

The PC must accept inbound UDP 47631 and TCP 47632 from the LAN. If Windows Firewall asks when `owldrop.exe` starts, allow it on private networks.

`owldrop-bridge file PHOTO.jpg` sends one file the same way. Without `--host`, both commands wait for the PC's hello beacon. `--timeout 8` is the default wait, in seconds.

## What is running

On the PC: `owldrop.exe`, left open.

On the Pi: `omdrop on forever` (AirDrop receiver, USB stick, Bluetooth wake) and `owldrop-bridge watch`.

The iPhone shows the name `owldrop-bridge name` printed. Files end in `Downloads\OwlDrop`. Text from `incoming/text/` replaces the PC clipboard.
