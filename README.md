# howl-drop

iPhone AirDrop to a Windows PC that has no Wi-Fi card. A Home Assistant OS
NUC near the phone is the AirDrop radio. It puts the PC's name (OCCORPC) on
the iPhone share sheet and forwards whatever the phone sends over Ethernet to
`owldrop.exe` on the PC.

```
iPhone ──AWDL + BLE──▶ NUC (HAOS, Intel 8260)     ──Ethernet──▶ Windows PC
                       OwlDrop add-on:                          owldrop.exe
                       OWL + omdrop-discoverable                Downloads\OwlDrop
                       AirDrop receiver                         clipboard
                       owldrop-bridge
```

This is a fork of [t4t5/omdrop-owl](https://github.com/t4t5/omdrop-owl), the
Linux radio backend for [omdrop](https://github.com/brentkearney/omdrop-plugin).
Its Arch/Debian packaging was removed here; upstream still has it.

## Layout

| Path | What |
|---|---|
| `addon/` | The Home Assistant OS local add-on: `config.yaml`, `Dockerfile`, `run.sh`. [addon/README.md](addon/README.md) is the install guide. |
| `owldrop/` | `owldrop.exe` (the Windows listener) and `owldrop-bridge` (the forwarder in the add-on), C++17, one CMake tree. |
| `userspace/` | `omdrop-discoverable`, the radio helper, and `owl-profiles`, its per-driver table. |
| `patches/`, `pins` | OWL's pinned upstream commit and our patches on it; `tools/build-owl.sh` applies them. |
| `tools/` | `build-owl.sh` and `stage.sh` build the radio stack (inside the add-on image); `stage-addon.sh` assembles the add-on folder. |
| `tests/` | `test_exclusive.py` runs the helper's exclusive mode against command doubles, no radio needed. |
| `docs/` | Research notes: why each other path (Pi, D1 Mini, Windows-native, VMs) was or wasn't taken, and the 8260 IBSS findings. |

## Develop

The add-on is the only thing deployed. Edit here, then:

```bash
python -m unittest tests.test_exclusive
```

```bash
tools/stage-addon.sh //homeassistant/addons/owldrop
```

Bump `version` in `addon/config.yaml`, then in Home Assistant: Add-on store,
menu, Check for updates, update OwlDrop, start it, and read its log.

The Windows side:

```bash
cmake -S owldrop -B owldrop/build && cmake --build owldrop/build --config Release --target owldrop
```

Everything that runs on the NUC is a Linux script, so `.gitattributes` keeps
the checkout LF even with `core.autocrlf`; `stage-addon.sh` refuses a CRLF
file.

## Status

- **NUC's Intel 8260 (soldered):** discovery works. OCCORPC shows on the
  sheet after mDNS answers to the phone's queries (sent unicast as well, so
  OWL holds them for the phone), packet copies (`owl -U`) and replies sent
  while the phone is still on our channel (`owl -H`). Tapping the tile never
  finishes: the /Ask connection's SYN-ACK is never acknowledged. Channel 44
  makes the firmware reset the card; an iPhone on 5 GHz Wi-Fi is on channel 6
  one 64 TU slot in sixteen, and on a 2.4 GHz network's channel about as
  rarely when idle. Details in [docs/8260-ibss-join.md](docs/8260-ibss-join.md).
- **Next radio:** a MediaTek MT7612U USB stick (Alfa AWUS036ACM), run as
  t4t5/omdrop-owl pull request 13's active monitor, ported into
  `userspace/omdrop-discoverable` (`exclusive_backend` in `owl-profiles`).
  It can transmit on 44 and 149, where the phone mostly is. The add-on picks
  it when plugged in (`radio: auto`). Untested here yet.
- Upstream (a063387) reports the Intel AX200 working in exclusive mode; the
  NUC6i7KYK can't take one (no Wi-Fi M.2 slot).
