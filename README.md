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

- **Sheet:** the add-on runs the IBSS cell on channel 6 (the `channels`
  option). Versions 0.1.12–0.1.13 unlocked 5 GHz and joined channel 44,
  where the log reported a joined cell and the iPhone never listed OCCORPC.
  On channel 6 (0.1.11) the tile appeared.
- **Transfer:** on channel 6 the send then died in the handshake (0.1.11).
  That is the next thing to chase once the tile is back.
- The 8260's firmware (`iwlwifi-8000C-36.ucode`) is closed and final; the
  upstream `iwlwifi` profile was tested on AX201, AX211 and BE, not the 8260.
