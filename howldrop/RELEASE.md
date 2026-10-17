`howldrop.exe` gets what your iPhone AirDrops to the HowlDrop Home Assistant add-on: a notification for each file or text, files in `Downloads\HowlDrop`, text on the clipboard, and links (or photos and documents, if you choose) opened as they arrive.

**Install**

1. Download `howldrop.exe` below and put it somewhere it can stay, e.g. `%LOCALAPPDATA%\Programs\HowlDrop`. Nothing to install.
2. Double-click it. Windows SmartScreen may say it doesn't recognize the app, since it is not code-signed: **More info**, then **Run anyway**. Allow it through the firewall on private networks.
3. Right-click the HowlDrop icon by the clock, **Copy pairing key**, and paste it into the add-on's `pc_key` option. Turn on `forward_to_pc`, and restart the add-on.

**Send to iPhone** (add-on 0.15.0 or later, `send_to_iphone` on): the tray menu's **Send to iPhone** (Look for devices, then pick one), or right-click a file in Explorer, **Send to**, **iPhone (HowlDrop)**. Set the iPhone's AirDrop to Everyone for 10 Minutes and accept there.

Use the release that matches your add-on version. The [Windows app's README](https://github.com/MrSco/howl-drop/blob/main/howldrop/README.md) has the menu, the commands and how pairing works.
