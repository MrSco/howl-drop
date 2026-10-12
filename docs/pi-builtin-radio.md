# Can the Pi use its built-in wireless instead of the USB stick?

**No.** The onboard chip cannot replace the USB MediaTek MT7612U (`0e8d:7612`, Alfa AWUS036ACM). That holds for the original Raspberry Pi 3, the Pi 3 B+, the Pi 4, and the Pi 5.

Pi 3 versus Pi 4 does change the radio. The original Pi 3 is a 2.4 GHz-only Broadcom. The Pi 3 B+ and the Pi 4 share one dual-band Cypress/Infineon part. The Pi 5 keeps that same part. None of those chips can take the place of the stick.

The bridge still needs real AWDL frames on the air, a radio that injects them and ACKs the phone, and a Bluetooth advert that wakes AirDrop. The path with a both-directions report is [omdrop-owl pull request 13](https://github.com/t4t5/omdrop-owl/pull/13) on the USB stick, with the Pi’s own Bluetooth sending the wake advert, and files and clipboard text forwarded over Ethernet. The stick stays.

## What “works” would have to mean

AirDrop’s sheet is looking for AWDL, not for a Wi-Fi network. [OWL](https://github.com/seemoo-lab/owl) requires a card with active monitor mode: frame injection, and hardware ACKs for unicast addressed to the card. Without those ACKs the phone retransmits and the HTTPS discovery behind the sheet times out. Channels are 6, 44, and 149. Pull request 13’s working report preferred channel 6, then 44 or 149.

Released omdrop-owl has profiles for `mt7925e` and `iwlwifi` only. `brcmfmac` is not in [owl-profiles](https://github.com/t4t5/omdrop-owl/blob/master/userspace/owl-profiles). Pull request 13 adds active monitor for `mt76x2u`. It does not add the Pi’s chip.

Stock `brcmfmac` does not provide monitor mode or injection. Kali’s own write-up of these boards says the Broadcom/Cypress chips do neither until Nexmon patches the firmware ([Kali, 22 July 2025](https://www.kali.org/blog/raspberry-pi-wi-fi-glow-up/)). hcxdumptool’s adapter notes say the same about official `brcmfmac`. There is no mac80211 “Device supports active monitor” flag on this driver. Injection exists only with Nexmon.

Nexmon’s monitor mode is what OWL’s maintainer calls active: the patched firmware ACKs frames for its own MAC, and OWL’s `-N` flag skips the nl80211 active-monitor request that `brcmfmac` rejects ([owl#12](https://github.com/seemoo-lab/owl/issues/12#issuecomment-673651362)). `-N` also skips OWL’s channel setting. That is a firmware behavior inside a fork of the closed Broadcom image, not the `iw` active-monitor mode pull request 13 uses on `mt76x2u`. No omdrop-owl run, and no current-iPhone AirDrop sheet, has confirmed those ACKs on this chip.

## Raspberry Pi 3 Model B

| | |
|---|---|
| Wi-Fi chip | Broadcom BCM43438, which Cypress also calls CYW43438. Nexmon’s firmware target is `bcm43430a1` (Nexmon’s table notes that this part was mislabeled bcm43438 in older rows). |
| Bands | 2.4 GHz 802.11n only. The Pi 3 B+ announcement is the dual-band replacement of this chip. Channels 44 and 149 do not exist here. |
| Monitor and injection | Stock `brcmfmac`: neither. Nexmon marks monitor, radiotap, and injection on firmware `7_45_41_26` and `7_45_41_46` (Raspbian 8 and Stretch). Upstream Nexmon’s Pi build notes stop at kernel 5.15. |
| Active monitor / hardware ACKs | Not via nl80211. OWL treats Nexmon monitor mode as active and requires `-N`. Nothing in that path is an omdrop-owl measurement. |
| OWL / omdrop-owl | OWL’s README still points at a Raspberry Pi 3 as the 2018 OpenDrop demo. That project is unmaintained and says it can be incompatible with later AWDL. omdrop-owl has no `brcmfmac` result. |
| Bluetooth during injection | One combo chip, Bluetooth 4.1 on a UART, Wi-Fi on SDIO, one PCB antenna. The Continuity wake advert is 2.4 GHz, and this radio can only be on 2.4 GHz, so the advert and AWDL would fight over the same antenna and the same patched firmware. |
| Replace the USB stick? | **No.** |

## Raspberry Pi 3 B+ and Raspberry Pi 4

Same Wi-Fi chip on both boards. Raspberry Pi’s [Pi 3 B+ to Pi 4](https://pip.raspberrypi.com/categories/685-app-notes-guides-whitepapers/documents/RP-003475-WP/Transitioning-from-Pi-3-to-Pi-4.pdf) note says wireless on both is a Cypress CYW43455 on SDIO, firmware `brcmfmac43455-sdio`, and that no software change is required for the radio. The Pi 3 B+ uses one Proant PCB antenna for Wi-Fi and Bluetooth together.

| | Pi 3 B+ | Pi 4 |
|---|---|---|
| Wi-Fi chip | Cypress CYW43455 (`bcm43455c0`) | Same CYW43455 (`bcm43455c0`) |
| Bands | 2.4 and 5 GHz, 802.11ac, 1x1 | Same |
| Bluetooth as sold | 4.2 | 5.0 |
| Replace the USB stick? | **No.** | **No.** |

The Bluetooth version label does not add a wake path. The AirDrop wake is a legacy LE advert (Apple company id `0x004C`, Continuity type `0x05`). InternalBlue’s comparison of the 3 B+ and the Pi 4 found the same controller: a newer `.hcd` reports Bluetooth 5.0, and the optional 5.0 features (2 Mb/s PHY, coded PHY, extended advertising) are absent. Nexmon lists one firmware line for “Raspberry Pi B3+/B4”, not two chips.

Nexmon’s own table for `bcm43455c0`:

| Firmware | Where it applies | Monitor | Radiotap | Injection |
|---|---|---|---|---|
| `7_45_154` | Kernel 4.9 / 4.14 / 4.19 | yes | yes | not marked |
| `7_45_189` | Kernel 4.14 / 4.19 / 5.4 | yes | yes | not marked |
| `7_45_206` | Raspberry Pi OS kernel 5.4 | yes | yes | yes |
| `7_45_234` | Current Raspberry Pi OS, and the row that includes the Pi 5 | not marked | not marked | not marked |

The only upstream row that claims injection is an old 5.4-era firmware. On that firmware, [nexmon#505](https://github.com/seemoo-lab/nexmon/issues/505) reports that injections closer than about 2 ms crash the firmware. AWDL’s sync frames and a file transfer are denser than that. [nexmon#360](https://github.com/seemoo-lab/nexmon/issues/360) shows a Pi 4 losing channel control in monitor mode (`brcmf_proto_bcdc_msg` status `-110`).

The published OWL result on this chip is [opendrop#80](https://github.com/seemoo-lab/opendrop/issues/80) (December 2021, Pi 4, kernel pinned to 5.4.81, `bcm43455c0`). Monitor and injection came up, OWL saw peers, and a send to a Mac worked. The Pi did not appear in iPhone AirDrop sheets, and the Pi could not receive. Injection also failed with `Resource temporarily unavailable` unless the channel was coaxed with `nexutil`. A later note on that thread, from a different card, is the same shape: a Mac on macOS 11 worked and an iPhone 8 on iOS 15.3 did not. That is the weak record with iPhones. It is not an omdrop-owl result, and pull request 13 does not cover this driver.

Kali’s July 2025 packages (`brcmfmac-nexmon-dkms`, `firmware-nexmon`) do bring monitor mode and an `aireplay-ng` injection test up on a Pi 4, 64-bit and 32-bit. Kali says that injection test is not always stable and depends on the device. An aircrack-style test is not an AirDrop transfer.

Bluetooth on these boards is the other half of the CYW43455. The datasheet’s cheap layout, which is the one the Pi uses, is a single dual-band antenna shared by Bluetooth and WLAN, with coexistence done in firmware. Wi-Fi is SDIO and Bluetooth is HCI over UART, so the stick design can leave onboard Wi-Fi idle and still advertise from this chip. Injection loads Nexmon’s Wi-Fi firmware onto that same die. The UART controller can still show up in BlueZ. That is not a second radio. The wake advert then depends on a patched firmware whose injection path already crashes, on an antenna the Wi-Fi side is holding. It does not leave Bluetooth usable for the AirDrop wake advert.

## Raspberry Pi 5

The chip is not a new one. The [Pi 5 announcement](https://www.raspberrypi.com/news/introducing-raspberry-pi-5/) says the Infineon CYW43455 is carried over from the Pi 4 unchanged: dual-band 802.11ac and Bluetooth 5.0 / BLE, with a better SDIO link (DDR50) and its own power rail. Nexmon’s `7_45_234` row is “Raspberry Pi B3+/B4/5” and does not mark monitor or injection. Kali’s 2025 packages include the Pi 5 in the same “injection is not always stable” test as the Pi 4. Same combo chip, same shared antenna, same answer: **it cannot replace the USB stick.**

## Decision

| Board | Chip | 2.4 / 5 GHz | Injection | Hardware ACKs the way pull request 13 needs | omdrop-owl result | Bluetooth still a wake advert while this chip injects | Drop the USB stick? |
|---|---|---|---|---|---|---|---|
| Pi 3 Model B | BCM43438 / `bcm43430a1` | 2.4 only | Nexmon, old firmware | Only inside Nexmon, via OWL `-N`; not nl80211 active monitor | None | No, same antenna and same 2.4 GHz radio | No |
| Pi 3 B+ | CYW43455 / `bcm43455c0` | 2.4 and 5 | Nexmon on `7_45_206` only; current `7_45_234` row unmarked | Same Nexmon caveat; iPhone sheet not shown | None | No | No |
| Pi 4 | CYW43455 / `bcm43455c0` | 2.4 and 5 | Same as the 3 B+ | Pi 4 OpenDrop thread: Mac send, no iPhone sheet, no receive | None | No | No |
| Pi 5 | Same CYW43455 | 2.4 and 5 | Same chip and same Nexmon rows; Kali’s test is unstable | No measured AirDrop ACKs | None | No | No |

Keep the USB MT7612U for AWDL and the Pi’s Bluetooth for the wake advert. The onboard Wi-Fi stays out of the injection path.
