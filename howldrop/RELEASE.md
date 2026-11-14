`howldrop.exe` puts your Windows PC on the iPhone's AirDrop share sheet, two ways:

- **AirDrop on this PC**: plug in a USB **MT7612U** Wi-Fi stick (e.g. Alfa AWUS036ACM) and turn it on in the tray menu. No Home Assistant needed, and nothing else to install. The first time, it downloads `howldrop-native.zip` below (the radio, about 3.5 MB; checked against the SHA-256 built into the exe) and runs it as you. Windows asks once, and only to put a new stick on its own WinUSB driver.
- **Through Home Assistant**: the HowlDrop add-on forwards what arrives there.

**New in 0.22.0**
- **Sending to the iPhone is about 10× faster** with the USB stick: a 13 MB batch of photos goes in about 9 s instead of 90, and large sends no longer stall partway. The stick now waits for the iPhone to acknowledge each frame and resends what it missed, resends only what TCP actually lost, and calibrates its transmitter the way Linux does, so the fast rates come out clean.
- **Sending speed is Automatic**: up to 54 Mbit/s, stepping down when the iPhone is far away. Settings can still fix it at 12, 54 or HT.
- **Files you send together go as one AirDrop**, and the window has **Cancel** and **Retry** for sends.
- A setting for **where received files go**.
- Keyboard and screen-reader support in the window and the tray.
- A send no longer waits forever when the iPhone leaves while you're deciding whether to accept.
- If sending is still slow, the iPhone may be giving AirDrop only a little of its time: open Control Center and tap the Wi-Fi icon once; AirDrop keeps working.

**New in 0.21.3**
- The tray icon's tooltip keeps showing its text after a notification is clicked, dismissed or times out.

**New in 0.21.2**
- **Several photos at once from the Photos app** now really arrive: Photos can put each one several folders deep, which 0.21.1 still turned away, leaving the iPhone on "Waiting". Edited photos, which all share a name, are kept side by side as `FullSizeRender (1).jpg` and so on.
- When HowlDrop turns a share away, the log says why.

**New in 0.21.1**
- **Several photos at once from the Photos app** now arrive. Photos wraps each one in a folder of its own, and HowlDrop turned the whole share away.
- Taking your time on Accept no longer eats into the iPhone's time to start sending: the wait for the upload starts when you answer.
- Sending to a phone whose details haven't fully arrived yet uses the compression iPhones expect.
- From [Owly](https://github.com/t4t5/owly) 0.4.2 and after.

**New in 0.21.0**
- **No administrator.** The radio now runs as you, with nothing installed: no Program Files folder, no scheduled task, no virtual network adapter. It brings its own network stack instead. Windows asks for permission only once, to put a stick on WinUSB, and not at all for one 0.20.0 already set up. 0.20.0's elevated scheduled task is what Microsoft Defender flagged (`Behavior:Win32/Persistence.A!ml`), and it's gone.
- To clean up after 0.20.0, as administrator: `schtasks /delete /tn "\HowlDrop\AirDrop" /f`, then delete `C:\Program Files\HowlDrop`.

**New in 0.20.0**
- **Native on Windows: no WSL.** HowlDrop now drives the stick itself, from Windows, instead of handing it to a Linux running in WSL: no `wsl --install`, no usbipd-win, no 250 MB Linux part. It starts in under a second. A 14.7 MB video from the iPhone still arrives in about 4 s.
- The stick restarts itself when Windows or a sleep leaves it unanswering, as if you'd unplugged it.
- Only MT7612U sticks: the Realtek RTL8852BU, which ran (untested) in WSL, isn't supported on Windows any more. It still works with the Home Assistant add-on.
- The PC keeps its entry on the share sheet across restarts, so a share sheet left open still reaches it.

**New in 0.19.2**
- **Comes back by itself** after the PC sleeps: the dot goes amber and the PC is back on the share sheet in seconds, instead of staying red until you turned it off and on. When it does go red, it says why.

**New in 0.19.1**
- A transfer that takes under a second reads "621 KB in under a second" instead of an impossible speed, and times count from the upload's start.

**New in 0.19.0**
- **Faster both ways.** The radio is rebuilt on [Owly](https://github.com/t4t5/owly), and the stick now listens 40 MHz wide: a 14.7 MB video from the iPhone arrives in about 3 s. Sending to the iPhone runs at 36 Mbit/s instead of 24: the same video goes in about 16 s instead of 25.
- **Contacts work again**: Apple's certificate authority behind many iPhones' AirDrop identities expired on 21 Oct 2026, and HowlDrop had been asking about (or turning away) your own contacts since.
- A picture you send shows its **preview** on the iPhone once accepted.
- When the iPhone is nearby but its AirDrop isn't taking files (Everyone for 10 Minutes ran out, or it has just finished a transfer), the card **says so** instead of just waiting; the send keeps trying.
- Each finished transfer shows **how long it took** (and the log keeps it). The version is in the tray menu and the window's settings, and HowlDrop is in the **Start menu**, so it's easy to start again after Quit.

Either way: files in `Downloads\HowlDrop` (deleted after 30 days unless you turn that off), text on the clipboard, links (or photos and documents, if you choose) opened as they arrive, and **Send to iPhone** from the window, the tray or Explorer's Send to. The dot on the tray icon shows the PC's radio: amber starting, green on the share sheet, red when it needs you.

**Install**

1. Download `howldrop.exe` below and put it somewhere it can stay, e.g. `%LOCALAPPDATA%\Programs\HowlDrop`. Nothing to install. (`howldrop-native.zip` is fetched by the exe when needed; you don't need to download it.)
2. Double-click it. Windows SmartScreen may say it doesn't recognize the app, since it is not code-signed: **More info**, then **Run anyway**. Allow it through the firewall on private networks.
3. Either turn on **AirDrop on this PC** (click the HowlDrop icon by the clock, then the gear), or, for Home Assistant: right-click the icon, **Copy pairing key**, paste it into the add-on's `pc_key`, turn on `forward_to_pc`, and restart the add-on.

**Send to iPhone**: set the iPhone's AirDrop to Everyone for 10 Minutes, drop files on the window and pick the phone (or the tray menu's Send to iPhone, or Explorer's Send to, iPhone (HowlDrop)), and accept on the phone. With AirDrop on this PC, the PC's Bluetooth wakes the phone (it stays connected to your other devices); through Home Assistant, the add-on needs `send_to_iphone`.

It pairs with the Home Assistant add-on 0.19.2 or 0.20.0. The [Windows app's README](https://github.com/MrSco/howl-drop/blob/main/howldrop/README.md) has the menu, the commands and how pairing works.
