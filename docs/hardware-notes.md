# Hardware and iPhone notes

What this add-on's development measured, on Home Assistant OS (Linux 6.12,
generic x86-64) with one iPhone. The per-driver table the add-on runs
from is `userspace/owl-profiles`; these are the observations behind its lines
and the add-on's defaults.

## The iPhone

- **Transfers run on 5 GHz.** With its Wi-Fi on a 5 or 6 GHz network, an
  idle iPhone's AWDL sequence looks like `6,0,149,0,0,0,0,0,6,0,149,...` and,
  during a transfer, `149,149,149,149,149,149,0,0,6,149,...`. Channel 6 gets
  one or two slots in sixteen. A 1.1 MB photo took 27 s on 6 and about 1 s on
  149; two photos, 4 MB, crossed in about 3 s on 149. A card that can follow
  the phone onto 5 GHz is the difference between usable and not.
- **The pre-window survey can miss the phone on 149** while it is idle, so
  the add-on starts on 5 GHz anyway (149 unless the phone was heard on 44 or
  36) and keeps 6 as the last resort.
- **The chunked terminator sometimes never comes.** The phone sends an
  `/Upload` body chunked, and now and then sends the whole archive, has every
  byte acknowledged, and then sends nothing at all: no `0\r\n\r\n`, no
  resend. A receiver that waits for the terminator answers 408 after its idle
  timeout and the phone reports a failed transfer. A Mac answers once the
  archive is complete; `addon/howl_upload.py` does the same (the DVZip stream
  ends and the CPIO trailer is in). Seen on channels 6 and 149, with 1 MB and
  3 MB photos.
- **Sharing a picture from a browser sends its address**, not the image,
  though the sheet says "PNG Image". `fetch_links` fetches it.

## Cards

- **MediaTek MT7612U (USB `0e8d:7612`, driver `mt76x2u`, e.g. Alfa
  AWUS036ACM): works.** Home Assistant OS ships the driver and firmware
  (`mediatek/mt7662.bin`). It runs as an active monitor (t4t5/omdrop-owl pull
  request 13, ported into `userspace/omdrop-discoverable`), transmits on 44
  and 149 with regulatory domain US, and carried every transfer above on 149.
  It needs no USB passthrough: the host's driver makes the interface and the
  add-on shares the host network.
- **Intel Wireless-AC 8260 (`iwlwifi`, firmware `8000C-36`): not usable.**
  As an IBSS cell it holds only channel 6. On 44 the cell joined, OWL
  rejected the phone's frames (2 stray bytes each), and about 13 s in every
  firmware command returned -5 and iwlwifi removed and re-probed the card,
  taking `mon0` with it. `8000C-36` is the last firmware the card gets. On 6
  the tile appeared, but no transfer ever finished: the phone is there one
  slot in sixteen. Later Intel cards (AX200, AX201, AX211, 9560) work in
  exclusive mode on 6 according to upstream; expect channel-6 speeds.
- **Shared mode** (a card that stays on its network, e.g. MediaTek MT7925)
  is implemented by the helper and selected by the add-on for a card that is
  Home Assistant's Wi-Fi uplink, but has not been tried on Home Assistant OS.

## Home Assistant OS specifics

- NetworkManager's `wpa_supplicant` keeps holding an interface marked
  unmanaged, and an IBSS join then returns `EOPNOTSUPP`; the add-on removes
  the interface from the supplicant over D-Bus first.
- Country `00` (unset) leaves 5 GHz receive-only; the add-on sets `US` when
  the domain is unset. Set the right country in Home Assistant if you are
  elsewhere.
- `iw` may report no channel for a monitor interface at all, and an active
  monitor alone may not retune the radio; the add-on counts frames actually
  received (`verbose`) rather than trusting `iw`.
- The default 200 ms TCP retransmit floor stretched TLS handshakes over AWDL
  to seconds; `awdl0`'s route gets 20 ms.
