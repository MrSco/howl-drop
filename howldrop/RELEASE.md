`howldrop.exe` puts your Windows PC on the iPhone's AirDrop share sheet, two ways:

- **AirDrop on this PC** (new): plug in a USB **MT7612U** Wi-Fi stick (e.g. Alfa AWUS036ACM) and turn it on in the tray menu. No Home Assistant needed. Needs WSL 2 (`wsl --install`) and usbipd-win, which HowlDrop offers to install. The first time, it downloads `howldrop-linux.tar.gz` below (the radio, in a WSL distro of its own; checked against the SHA-256 built into the exe).
- **Through Home Assistant**: the HowlDrop add-on forwards what arrives there.

Either way: a notification for each file or text, files in `Downloads\HowlDrop` (deleted after 30 days unless you turn that off), text on the clipboard, links (or photos and documents, if you choose) opened as they arrive, and **Send to iPhone** from the tray or Explorer's Send to. The dot on the tray icon shows the PC's radio: amber starting, green on the share sheet, red when it needs you.

**Install**

1. Download `howldrop.exe` below and put it somewhere it can stay, e.g. `%LOCALAPPDATA%\Programs\HowlDrop`. Nothing to install. (`howldrop-linux.tar.gz` is fetched by the exe when needed; you don't need to download it.)
2. Double-click it. Windows SmartScreen may say it doesn't recognize the app, since it is not code-signed: **More info**, then **Run anyway**. Allow it through the firewall on private networks.
3. Either turn on **AirDrop on this PC** in the menu (right-click the HowlDrop icon by the clock), or, for Home Assistant: **Copy pairing key**, paste it into the add-on's `pc_key`, turn on `forward_to_pc`, and restart the add-on.

**Send to iPhone**: set the iPhone's AirDrop to Everyone for 10 Minutes, then the tray menu's **Send to iPhone** (Look for devices, then pick one), or right-click a file in Explorer, **Send to**, **iPhone (HowlDrop)**, and accept on the phone. With AirDrop on this PC, the PC's Bluetooth wakes the phone (it stays connected to your other devices); through Home Assistant, the add-on needs `send_to_iphone`.

Use the release that matches your add-on version. The [Windows app's README](https://github.com/MrSco/howl-drop/blob/main/howldrop/README.md) has the menu, the commands and how pairing works.
