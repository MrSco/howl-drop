# Changelog

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
