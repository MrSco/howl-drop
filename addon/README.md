# HowlDrop

AirDrop from an iPhone to Home Assistant. While the add-on runs, **HowlDrop**
appears on the iPhone's share sheet; photos, videos, files, links and text
sent to it land in `/share/howldrop`, which the Samba share add-on shows to
every PC on the network.

- Needs a USB Wi-Fi adapter such as the MediaTek MT7612U (Alfa AWUS036ACM),
  near the phone, and Bluetooth on the host.
- Only the people you list can send, if you want (Apple-verified, like a
  Mac set to Contacts Only).
- Each drop can notify your phone through the Home Assistant app, and fires
  an event for automations.
- Old files are deleted after 30 days, or never.

See the Documentation tab for setup and every option.
