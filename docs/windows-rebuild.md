# iPhone AirDrop to a Windows PC

The pair is an iPhone and a Windows PC. Files and shared text come from the phone to the PC. The PC does not have a Wi-Fi card. The adopted path is a Raspberry Pi beside the phone: the Pi's USB Wi-Fi and Bluetooth run the Linux AirDrop stack, the share sheet shows the Windows PC's name, and the Pi forwards the payload over Ethernet to `owldrop.exe`. Setup is in [pi-bridge.md](pi-bridge.md). The Windows program does not create a Wi-Fi adapter.

The PC alone still cannot answer the sheet.

The iPhone fills the AirDrop sheet only after it finds an `_airdrop._tcp` receiver on Apple's peer-to-peer Wi-Fi (AWDL) and that receiver answers HTTPS `/Discover`. Bluetooth is the wake signal that brings that Wi-Fi link up. With the share sheet open, the phone is already the sender: it queries for receivers on AWDL. A PC with no Wi-Fi radio has no AWDL interface, so the query has nowhere to land and the sheet stays empty. The home LAN is a different network. Sharingd does not browse Ethernet for AirDrop peers. A BLE advert from Windows can wake a nearby Apple device. It does not publish the service the sheet is searching for, and it does not carry the file.

The same limit covers clipboard text shared from the phone. The share sheet sends that text as an AirDrop transfer, which is the AWDL path above. Universal Clipboard is Continuity between Apple devices, and it uses that same peer-to-peer link.

The radio that answers the phone is the Pi's USB stick, in [pi-bridge.md](pi-bridge.md). Ethernet on the PC does not substitute for that radio. [windows-path.md](windows-path.md) is the earlier note on putting a radio in the PC itself.

## What is built

`owldrop.exe`, built from [`owldrop/`](../owldrop/), is the Windows listener. It receives a file into `Downloads\OwlDrop` and sets the clipboard from text sent to TCP 47632. It does not create a Wi-Fi adapter and does not answer the iPhone itself. The Pi program `owldrop-bridge` is built from that same source tree.
