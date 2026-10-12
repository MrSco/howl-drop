# Built-in Wi-Fi experiment

No setup has put the Pi’s built-in radio in a current iPhone AirDrop sheet. The nearest experiment is a Raspberry Pi 4, kernel `5.4.81`, Nexmon firmware `7_45_206`, and upstream OWL with `-N`. It fails at the share sheet: the phone never lists the Pi, and the Pi never receives. A send to a Mac did complete. That report is [opendrop#80](https://github.com/seemoo-lab/opendrop/issues/80) (December 2021). omdrop-owl has no profile and no flags for this chip, including [pull request 13](https://github.com/t4t5/omdrop-owl/pull/13).

## What to build

| Piece | Pin |
|---|---|
| Board | Raspberry Pi 4 Model B. The chip is CYW43455 (`bcm43455c0`), same as a Pi 3 B+. The recorded run is a Pi 4. A Pi 3 Model B has no 5 GHz, so it cannot do channel 149. A Pi 5 is the same CYW43455 on firmware Nexmon does not mark for injection. |
| OS | 32-bit Raspberry Pi OS Buster on the 5.4 line. The report stepped the kernel down to **5.4.81** (`5.4.81-v7l+`, `#1378`, 7 Dec 2020, `raspberrypi/firmware` `919aee0`). The 2020-12-02 image ships 5.4.79. The 2021-01-11 image ships 5.4.83. Stay on that 5.4 line. `apt full-upgrade` from a current archive leaves it. |
| Nexmon firmware | `patches/bcm43455c0/7_45_206/nexmon` only. That is the one `bcm43455c0` row Nexmon marks for frame injection, and it is tied to kernel 5.4. `7_45_154` and `7_45_189` are monitor-only in that table. `7_45_234`, the current Pi OS firmware, is unmarked for monitor and injection. |
| OWL flags | `owl -N -i mon0 -c 6` (or `-c 149`). `-N` is required. It skips the nl80211 active-monitor request, which `brcmfmac` rejects. |
| Bluetooth | A separate USB dongle on `btusb`. Turn the onboard controller off with `dtoverlay=disable-bt`. The wake advert must not run on this combo chip. |

## Recipe

Flash 32-bit Buster and confirm the kernel before installing anything else:

```
uname -r
```

You want `5.4.81-v7l+`. `5.4.79-v7l+` or `5.4.83-v7l+` are the neighboring images on the same line. Hold the kernel so a later upgrade cannot move it:

```
sudo apt-mark hold raspberrypi-kernel raspberrypi-bootloader
```

Do not run `apt full-upgrade`.

Disable the onboard Bluetooth controller. Wi-Fi stays on SDIO. In `/boot/config.txt` on this old image:

```
dtoverlay=disable-bt
```

Reboot, plug in a USB Bluetooth adapter that `btusb` drives, and check that `hciconfig` or `bluetoothctl list` shows that dongle and not the CYW43455. The recorded Pi 4 run did not do this. It is still required here, because the Continuity advert (Apple company id `0x004C`, type `0x05`) has to come from a radio that is not the chip Nexmon is patching.

Build Nexmon’s injection firmware for this exact chip. The steps are the project’s Raspberry Pi section, pointed at `7_45_206` rather than the older monitor-only directories named in that section:

```
sudo apt install raspberrypi-kernel-headers git libgmp3-dev gawk qpdf \
  bison flex make autoconf libtool texinfo
git clone https://github.com/seemoo-lab/nexmon.git
cd nexmon
source setup_env.sh
make
cd patches/bcm43455c0/7_45_206/nexmon
make
make backup-firmware
make install-firmware
cd ../../../utilities/nexutil
make && sudo make install
```

`make` in the patch directory also builds `brcmfmac_5.4.y-nexmon/brcmfmac.ko` on this kernel. Copy that over the stock module (same layout as the 5.10 notes in [nexmon#505](https://github.com/seemoo-lab/nexmon/issues/505)), then:

```
sudo depmod -a
sudo reboot
```

After reboot, `dmesg` should show the `7.45.206` firmware on `brcmfmac`. Bring up the monitor interface the Nexmon README uses:

```
sudo iw phy $(iw dev wlan0 info | awk '/wiphy/ {print "phy"$2}') interface add mon0 type monitor
sudo ifconfig mon0 up
sudo iw dev wlan0 set power_save off
```

`tcpdump -i mon0` should print radiotap frames. If `mon0` never appears, the patched module did not load. Stop. A newer firmware will not grow an injection path.

Set the channel with `nexutil`. Nexmon’s nl80211 channel changes fail (status `-110` in [nexmon#360](https://github.com/seemoo-lab/nexmon/issues/360)), and `-k` takes a chanspec string:

```
sudo nexutil -k6/20
```

The Pi 4 report had to use channel 6 and channel 149 together, via `nexutil` and OWL, or injection returned `unable to inject packet (send: Resource temporarily unavailable)`. If `6/20` does that, set `149/20` instead and use the same number below.

Build upstream [seemoo-lab/owl](https://github.com/seemoo-lab/owl) (`libpcap-dev`, `libev-dev`, `libnl-3-dev`, `libnl-genl-3-dev`, `libnl-route-3-dev`, then cmake). Start it on the monitor interface that is already up:

```
sudo owl -N -i mon0 -c 6 -vv
```

Use `-c 149` when `nexutil` is on `149/20`. Without `-N`, OWL sends `NL80211_MNTR_FLAG_ACTIVE` and exits with `Could not put device in monitor mode`. `-N` does not turn off OWL’s later channel hops; those hops still miss on this driver, which is why `nexutil -k` stays in the recipe. There is no omdrop-owl equivalent. `omdrop-discoverable` matches `mt76x2u` / `iwlwifi` / `mt7925e`. `brcmfmac` is not in `owl-profiles`, and pull request 13’s `active-monitor` backend is the MediaTek stick.

Leave at least 2 ms between injections. On this firmware, tighter injection crashes the chip ([nexmon#505](https://github.com/seemoo-lab/nexmon/issues/505)). OWL’s own action-frame timer is denser than that once a transfer starts.

## Where it stops

1. Kernel not on 5.4, or firmware not `7_45_206`: Nexmon’s table does not claim injection. Nothing after this step is the experiment.
2. `owl` without `-N`: dies on the active-monitor netlink call, before any AWDL frame.
3. Channel set only inside OWL, not with `nexutil`: `pcap_inject` returns `Resource temporarily unavailable`. The Pi 4 report got past this by combining `nexutil` with channels 6 and 149.
4. Past that, the same report saw `add peer` in OWL and completed a send to a Mac. The iPhone AirDrop sheet did not list the Pi. The Pi’s own receive never showed up on another device’s sheet. A follow-up on that thread, on a different card, is the same split: a Mac on macOS 11.6.3 worked, an iPhone 8 on iOS 15.3 did not. The reply on the thread is that the Mac can take the file over ordinary Wi-Fi, and the iPhone sheet needs AWDL that this OWL no longer matches.
5. The USB Bluetooth dongle is what keeps the wake advert off the combo chip. The recorded run did not use one, and adding one has not been shown to make the sheet appear. The sheet is still the step that fails.

The Ethernet forwarder to the Windows PC has nothing to forward. The phone never selects the Pi.
