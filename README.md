# howl-drop

AirDrop from an iPhone to Home Assistant OS. A Home Assistant add-on puts
**HowlDrop** on the iPhone's share sheet; whatever the phone sends lands in
Home Assistant's share folder, where any PC on the network can open it. That
is the recommended setup and the default: nothing runs on any PC.

Optionally, a Windows PC that runs `howldrop.exe` also gets each file in its
Downloads and shared text on its clipboard (the add-on's `forward_to_pc`).

```
iPhone ──AWDL + BLE──▶ Home Assistant OS, HowlDrop add-on ──▶ /share/howldrop (Samba)
                       OWL + omdrop-discoverable             └─▶ howldrop.exe on a PC (optional)
                       AirDrop receiver
```

The radio is any Wi-Fi card [omdrop-owl](https://github.com/t4t5/omdrop-owl)
has a profile for (`userspace/owl-profiles`), driven by the host's own kernel
driver: the add-on loads no driver or firmware. A card that is not Home
Assistant's network uplink runs exclusive mode for as long as the add-on runs;
one that is the uplink can run shared mode. A USB MediaTek MT7612U (Alfa
AWUS036ACM) is the tested choice: it follows the iPhone onto 5 GHz, where
transfers run. See [docs/hardware-notes.md](docs/hardware-notes.md).

Install and options: [addon/README.md](addon/README.md).

This is a fork of [t4t5/omdrop-owl](https://github.com/t4t5/omdrop-owl), the
Linux radio backend for [omdrop](https://github.com/netmojo-systems/omdrop-plugin),
with the add-on, delivery, and the Windows listener added and its distribution
packaging removed (upstream has it).

## Layout

| Path | What |
|---|---|
| `addon/` | The add-on: `config.yaml`, `Dockerfile`, `run.sh`, and the Python it ships (`deliver.py`, `senders.py`, `patch_receiver.py`). |
| `howldrop/` | `howldrop.exe` (the Windows listener) and `howldrop-bridge` (the forwarder in the add-on), C++17, one CMake tree. |
| `userspace/` | `omdrop-discoverable`, the radio helper, and `owl-profiles`, its per-driver table. |
| `patches/`, `pins` | OWL's pinned upstream commit and the patches applied to it; `tools/build-owl.sh` applies them. |
| `tools/` | `build-owl.sh` and `stage.sh` build the radio stack (inside the add-on image); `stage-addon.sh` assembles the add-on folder; `build-howldrop.ps1` builds `howldrop.exe`; `make-icons.py` draws the add-on's icon and logo. |
| `tests/` | The helper, the add-on's radio choice, delivery and the allow list, against command doubles and local sockets: no radio needed. |

## Develop

```bash
tools/run-tests.sh
```

(or `python -m unittest discover -s tests` without bats). Then stage the
add-on into Home Assistant's `/addons` share and bump `version` in
`addon/config.yaml` for every deploy, or Supervisor keeps the old image:

```bash
tools/stage-addon.sh //homeassistant/addons/howldrop
```

In Home Assistant: Add-on store, menu, Check for updates, update HowlDrop,
start it, and read its log.

The optional `howldrop.exe` builds with CMake, Ninja and a MinGW-w64 g++
(Strawberry Perl ships all three):

```powershell
tools\build-howldrop.ps1 -Run
```

What runs in the add-on is Linux shell and Python, so `.gitattributes` keeps
the checkout LF even with `core.autocrlf`; `stage-addon.sh` refuses a CRLF
file.
