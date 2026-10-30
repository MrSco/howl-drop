# Changelog

## 0.19.0

- **Not yet tried on Home Assistant OS.** This release was tested as howldrop.exe's Linux part (WSL, an MT7612U, an iPhone), which runs the same image; the add-on starts it differently (the host's Bluetooth, its own options). If it doesn't come up for you, 0.18.0 is the version to go back to, and an issue with the log helps.
- **A new radio stack.** OWL and the Python receiver and sender are gone: HowlDrop runs on [Owly](https://github.com/t4t5/owly) 0.4 (Rust), the rewrite of omdrop-owl, in one process, `howldrop-agent`. The card is chosen from Owly's driver table (`crates/owly-radio/drivers.toml`) instead of `owl-profiles`. Options keep their names; `channels` takes 6, 44 and 149 (not 36).
- On 5 GHz the card listens 40 MHz wide, as the iPhone sends: with an MT7612U a 14.7 MB video arrives in about 3 s.
- Contacts work again: Apple's Application Integration CA expired on 21 Oct 2026, and the check refused iPhones whose certificates it issued. The log says why a sender isn't taken for a contact.
- Sending to an iPhone: a picture goes with a preview, which the iPhone shows once it's accepted; when the phone is there but its AirDrop refuses (Everyone for 10 Minutes ran out, or it has just finished a transfer), the log and the PC say so, and the send keeps trying. New option `tx_rate`, the rate files go to the phone at.
- The log gives each transfer's size, time and speed, and the radio's counters every 10 s.
- `debug_capture` no longer saves senders' certificates.
- For howldrop.exe's Linux part only: the RTL8852BU's firmware, beside the MT7612U's (one linux-firmware commit, checked by hash).

## 0.18.0

- Look for devices asks again, up to three times, when nothing answers: a phone that has just finished a transfer can be out of reach for a few seconds.
- For **howldrop.exe 0.18.0** only (AirDrop on a Windows PC): the receiver can hold an incoming AirDrop for Accept or Decline on the PC, tell contacts from others in Everyone mode, and report an upload's progress. None of it is on in the add-on.

## 0.17.0

- Nothing changes for the add-on itself. This image is also the Linux part of **howldrop.exe 0.17.0**'s new AirDrop on this PC (a Windows PC with a USB MT7612U stick, no Home Assistant), so it carries the stick's firmware, the wireless regulatory database and `kmod` for that, which the add-on never uses.
- A failed send to the iPhone says why in words ("not accepted on the iPhone in time") instead of a sender log line.

## 0.16.0

- Up to date with omdrop-owl 0.3.0 (upstream): its fix for a radiotap field OWL read uninitialized on receive, channel lists read the way newer `iw` prints them (and No IR or radar channels skipped), and new card records (Intel BE200 tested; Realtek RTL8852BE in shared mode; several recorded as failing). The add-on's receive reordering is unchanged, rebuilt on top.

## 0.15.1

- The relay a PC sends through starts (0.14.0 and 0.15.0 logged "Relay: unknown command relay").

## 0.15.0

- Pick which device to send to: **Look for devices** in the panel, or in the PC's tray menu (Send to iPhone), lists the AirDrop devices nearby that are listening (an iPhone set to Everyone for 10 Minutes, a Mac with AirDrop open). Choose one, then send. Needs **howldrop.exe 0.15.0** on the PC.

## 0.14.0

- Send to iPhone from the HowlDrop panel (upload a file and watch it go), or from a paired Windows PC: the tray menu, Explorer's Send to, or `howldrop iphone FILE`. Needs `send_to_iphone`, `pc_key` for the PC, and **howldrop.exe 0.14.0**.
- New `send_to` option: the iPhone's name, so only that phone gets what you send.
- Quieter `Send:` log lines.

## 0.13.1

- Sending to an iPhone: the sender makes its identity first (it stopped with "identity error: self-signed-missing").

## 0.13.0

- Experimental: AirDrop to your iPhone. Turn on `send_to_iphone`, set the phone to Everyone for 10 Minutes, and copy a file into the share's `send` folder. See the Documentation tab.

## 0.12.0

- Pairing with the Windows PC: new `pc_key` option. `howldrop.exe` now accepts files only from a sender with its pairing key, and the add-on sends only to a PC that proves it has the key. Every transfer is signed. **Update `howldrop.exe` too, then copy its key (right-click the tray icon, Copy pairing key) into `pc_key`.**
- `howldrop.exe` no longer sends what you copy to other PCs unless you turn that on in its menu.

## 0.11.5

- The panel shows edited iPhone photos too (FullSizeRender.heic, with an HDR gain map): HEIC is now read with pillow-heif's current libheif instead of Debian's.
- The panel's filter buttons line up with the photos on a wide screen.

## 0.11.4

- Forwarding to `howldrop.exe` works: the Windows app reset any transfer whose data paused, so files from the add-on never arrived. Get the new `howldrop.exe` too.
- A new file that takes the name of one deleted from the share is forwarded (it was skipped as already sent).

## 0.11.3

- When forwarding to a PC fails, the log says why (timed out, refused, broke while sending) instead of only "could not reach the PC".

## 0.11.2

- The panel shows iPhone HEIC photos in every browser, as JPEG, with thumbnails. Download still gives the original.

## 0.11.1

- Runs without Home Assistant OS too: Docker Compose on Raspberry Pi OS or any Linux (`docker/` in the repository). There, a long-lived token reaches a Home Assistant elsewhere, and the panel can be opened from the addresses you allow.

## 0.11.0

- HowlDrop panel in the sidebar: recent drops with sender, photo thumbnails, text to copy, links to open, download and delete.
- `howldrop.exe` for Windows is now a tray app: a notification for each drop, and links (or photos and documents, if you choose) open as they arrive.

## 0.10.0

- Each drop fires a `howldrop_received` event (sender, file, kind, size, link) for automations.
- New `notify` option: a notification per drop, to the Home Assistant app or Home Assistant's own notifications. Text shows its first lines; a link opens when tapped.
- Documentation moved to the Documentation tab.

## 0.9.17

- Starts on channel 149 straight away. The survey for the phone's channel is gone: it could follow another Apple device nearby to the wrong channel, and it took 20 s.

## 0.9.16

- Frames are put back in order for TCP during aggregated transfers (fewer duplicate acknowledgements and resends).
- `aggregation` is `auto` (on for the MT7612U), `on` or `off`.

## 0.9.15

- Aggregation: the phone sends in bursts. A 30 MB video took 14 s instead of 47 s on an MT7612U.

## 0.9.14

- The radio listens 40 MHz wide on 5 GHz, as the phone sends.

## 0.9.13

- Fewer, faster acknowledgements from the add-on during uploads.

## 0.9.12

- Installs download a prebuilt image instead of compiling on your machine.

## 0.9.11

- Installable from GitHub as an add-on repository; every option has a description.

## 0.9.9

- Shared text goes to `text/` in the save folder.

## 0.9.8

- Contacts Only (`senders`) works on Home Assistant OS: the sender's Apple certificate is read correctly on OpenSSL 3.0.

## 0.9.4

- `delete_after_days`: files are deleted from the save folder 30 days after they arrive (0 keeps them), checked daily. The radio's logs are trimmed.

## 0.9.1

- Renamed HowlDrop. The share folder is the default; forwarding to `howldrop.exe` is optional and off.

## 0.8.0

- Works with any Wi-Fi card omdrop-owl supports, not only the MT7612U; `verbose` option.
