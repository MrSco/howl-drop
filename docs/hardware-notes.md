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
- **Which channel, without asking:** an idle iPhone is hard to hear on 149,
  and other Apple devices nearby use AWDL too: a survey before the window
  once missed the phone on 149 and once heard a Mac on 44 and started there,
  where the iPhone never came. The add-on starts on 149 with no survey; the
  `channels` option overrides it.
- **The chunked terminator sometimes never comes.** The phone sends an
  `/Upload` body chunked, and now and then sends the whole archive, has every
  byte acknowledged, and then sends nothing at all: no `0\r\n\r\n`, no
  resend. A receiver that waits for the terminator answers 408 after its idle
  timeout and the phone reports a failed transfer. Seen here on channels 6
  and 149, with 1 MB and 3 MB photos, and upstream on an Intel card. Since
  omdrop-plugin 0.16.2 the receiver stores an upload whose body is already a
  complete archive after 3 s of quiet (its pull requests 41 and 47).
- **Sharing a picture from a browser sends its address**, not the image,
  though the sheet says "PNG Image". `fetch_links` fetches it.

## Cards

- **MediaTek MT7612U (USB `0e8d:7612`, driver `mt76x2u`, e.g. Alfa
  AWUS036ACM): works.** Home Assistant OS ships the driver and firmware
  (`mediatek/mt7662.bin`). It runs as an active monitor (omdrop-owl's
  `active-monitor` mode, from its pull requests 13 and 24), transmits on 44
  and 149 with regulatory domain US, and carried every transfer above on 149.
  Upstream holds active monitors to channel 6, because the MT7921/MT7922
  send no injected unicast on 5 GHz; this fork runs the MT7612U's on 149
  first, and tunes the station monitor too (offered upstream as
  t4t5/omdrop-owl pull request 54).
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
