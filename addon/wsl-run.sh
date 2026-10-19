#!/bin/bash
# HowlDrop on a Windows PC: this image as a WSL 2 distro, which howldrop.exe
# downloads, imports and starts as its child (wsl_win.cpp). The PC's USB
# Wi-Fi stick reaches it through usbipd-win; WSL's own kernel drives it
# (mt76x2u ships as a module). Then run.sh, as in the add-on.
#
#   wsl-run.sh --name NAME --save DIR --outbox DIR --scan-request FILE
#              [--receive everyone|ask|contacts] [--senders LIST] [--ask-dir DIR]
#              [--send-to NAME] [--verbose]
#   wsl-run.sh --stop
#
# DIR and FILE are the PC's own folders, as /mnt/c/... paths: files land in
# the tray's inbox once (no second copy), and the tray drops what it sends
# into the outbox. howldrop.exe reads this script's output: log lines, and
# events (@howldrop, then tab-separated fields; announce.event):
#   @howldrop  stick  waiting      ready for usbipd to attach the stick
#   @howldrop  stick  ready IFACE  the stick has a Wi-Fi interface
#   @howldrop  stick  failed WHY   it attached but the driver gave up
#   @howldrop  up IFACE            the window is up (on the share sheet)
# and from deliver.py and outbox.py: received, sending, sent, nearby; from the
# receiver (howl_ask.py): ask, asked, incoming, receiving, stored, failed.
#
# Who may send (--receive): everyone (accepted as they come), ask (contacts
# in --senders, comma-separated phone numbers or emails, are accepted; anyone
# else waits for Accept or Decline on the PC, answered as a file in
# --ask-dir), or contacts (only --senders see the PC; senders.py).
set -uo pipefail

FIRMWARE_SRC=/usr/lib/howldrop/firmware
# WSL's kernel loads firmware in its own system layer, which does not see
# this distro's /lib/firmware; /mnt/wsl is shared by every WSL namespace.
FIRMWARE_DIR=/mnt/wsl/howldrop-firmware
FIRMWARE_PATH=/sys/module/firmware_class/parameters/path
STICK_WAIT_S=90

say() { printf '%s\n' "$*"; }
event() {
    local IFS=$'\t'
    printf '@howldrop\t%s\n' "$*"
}

if [ "${1:-}" = --stop ]; then
    # run.sh's TERM trap gives the card back and stops OWL.
    pkill -TERM -x run.sh 2>/dev/null || true
    for _ in $(seq 1 25); do
        pgrep -x run.sh >/dev/null 2>&1 || exit 0
        sleep 1
    done
    pkill -KILL -x run.sh 2>/dev/null || true
    exit 0
fi

NAME=HowlDrop
SAVE=
OUTBOX=
SCAN=
SEND_TO=
VERBOSE=false
RECEIVE=everyone
SENDERS=
ASK_DIR=
while [ $# -gt 0 ]; do
    case "$1" in
        --name) NAME=$2; shift 2 ;;
        --save) SAVE=$2; shift 2 ;;
        --outbox) OUTBOX=$2; shift 2 ;;
        --scan-request) SCAN=$2; shift 2 ;;
        --send-to) SEND_TO=$2; shift 2 ;;
        --receive) RECEIVE=$2; shift 2 ;;
        --senders) SENDERS=$2; shift 2 ;;
        --ask-dir) ASK_DIR=$2; shift 2 ;;
        --verbose) VERBOSE=true; shift ;;
        *) say "wsl-run.sh: unknown argument $1"; exit 2 ;;
    esac
done
if [ -z "$SAVE" ] || [ -z "$OUTBOX" ] || [ -z "$SCAN" ]; then
    say "wsl-run.sh: --save, --outbox and --scan-request are required"
    exit 2
fi
case "$RECEIVE" in
    everyone|contacts) ;;
    ask) [ -n "$ASK_DIR" ] || { say "wsl-run.sh: --receive ask needs --ask-dir"; exit 2; } ;;
    *) say "wsl-run.sh: --receive is everyone, ask or contacts"; exit 2 ;;
esac
if [ "$RECEIVE" = contacts ] && [ -z "$SENDERS" ]; then
    say "Contacts only, but no contacts are set: everyone can send until some are."
    RECEIVE=everyone
fi

say "HowlDrop for Windows: the radio in WSL ($(uname -r))."

# Firmware for the stick and the wireless regulatory database (without it
# the country stays 00 and 5 GHz is receive-only: the iPhone never sees us).
# Debian's wireless-regdb keeps the upstream-signed copy, the one WSL's
# kernel trusts, as regulatory.db-upstream.
copy_first() {
    local dest=$1 src
    shift
    for src in "$@"; do
        [ -f "$src" ] && { cp -f "$src" "$FIRMWARE_DIR/$dest"; return 0; }
    done
    say "No $dest in this image."
}
mkdir -p "$FIRMWARE_DIR"
cp -f "$FIRMWARE_SRC"/mt7662*.bin "$FIRMWARE_DIR"/ 2>/dev/null || say "No stick firmware in $FIRMWARE_SRC."
copy_first regulatory.db /lib/firmware/regulatory.db-upstream /lib/firmware/regulatory.db
copy_first regulatory.db.p7s /lib/firmware/regulatory.db.p7s-upstream /lib/firmware/regulatory.db.p7s
current=$(cat "$FIRMWARE_PATH" 2>/dev/null || true)
if [ -z "$current" ] || [ "$current" = "$FIRMWARE_DIR" ]; then
    printf '%s' "$FIRMWARE_DIR" > "$FIRMWARE_PATH" 2>/dev/null \
        || say "Could not point the kernel's firmware search at $FIRMWARE_DIR."
else
    say "The kernel's firmware path is already $current (set by something else); leaving it."
fi

# The driver, before the stick arrives, so its probe finds the firmware.
# Scatter-gather off: the setting the first working run had over usbipd.
modprobe mt76-usb disable_usb_sg=1 2>/dev/null || say "modprobe mt76-usb failed."
modprobe mt76x2u 2>/dev/null || say "modprobe mt76x2u failed: this WSL kernel may lack the driver."
modprobe tun 2>/dev/null || true
iw reg reload 2>/dev/null || true

# run.sh finds NetworkManager and bluetoothd on the system D-Bus; here there
# are neither, but it needs a bus to ask.
mkdir -p /run/dbus
if [ ! -S /run/dbus/system_bus_socket ]; then
    rm -f /run/dbus/pid
    dbus-daemon --system --fork || say "dbus-daemon did not start."
fi

stick_iface() {
    local d drv
    for d in /sys/class/net/*; do
        [ -d "$d/phy80211" ] || continue
        drv=$(basename "$(readlink -f "$d/device/driver" 2>/dev/null)" 2>/dev/null)
        [ "$drv" = mt76x2u ] && { basename "$d"; return 0; }
    done
    return 1
}

IFACE=$(stick_iface || true)
if [ -z "$IFACE" ]; then
    event stick waiting
    before=$(dmesg 2>/dev/null | wc -l)
    for _ in $(seq 1 "$STICK_WAIT_S"); do
        IFACE=$(stick_iface || true)
        [ -n "$IFACE" ] && break
        if dmesg 2>/dev/null | tail -n +"$((before + 1))" | grep -q 'probe with driver mt76x2u failed'; then
            dmesg | tail -n +"$((before + 1))" | grep -E 'mt76|usb 1-' | tail -8
            event stick failed "the stick stopped answering while starting; unplug it and plug it back in"
            exit 3
        fi
        sleep 1
    done
fi
if [ -z "$IFACE" ]; then
    event stick failed "no MT7612U stick arrived (is it plugged in and attached with usbipd?)"
    exit 3
fi
event stick ready "$IFACE"

mkdir -p /data "$SAVE" "$OUTBOX"
python3 - "$NAME" "$SAVE" "$SEND_TO" "$VERBOSE" "$RECEIVE" "$SENDERS" > /data/options.json <<'PY'
import json, sys
name, save, send_to, verbose, receive, senders = sys.argv[1:7]
json.dump({
    "pc_name": name, "save_folder": save, "notify": [],
    # senders.py: Contacts Only from these, or (receive ask) the contacts
    # accepted without asking.
    "senders": [s.strip() for s in senders.split(",") if s.strip()] if receive != "everyone" else [],
    "receive": receive,
    "fetch_links": True, "fetch_max_mb": 100,
    # howldrop.exe deletes old files from its inbox itself, on both routes.
    "delete_after_days": 0,
    "forward_to_pc": False, "pc_host": "", "pc_key": "",
    "send_to_iphone": True, "send_to": send_to,
    "interface": "auto", "allow_untested": False, "channels": "", "aggregation": "auto",
    "verbose": verbose == "true", "debug_capture": False,
}, sys.stdout, indent=2)
PY

# No Bluetooth in WSL: howldrop.exe sends the AirDrop wake from Windows'.
export HOWLDROP_EVENTS=stdout HOWLDROP_OUTBOX="$OUTBOX" HOWLDROP_SCAN_REQUEST="$SCAN" HOWLDROP_PANEL=off \
    HOWLDROP_BLE=off
if [ "$RECEIVE" = ask ]; then
    mkdir -p "$ASK_DIR"
    rm -f "$ASK_DIR"/*.accept "$ASK_DIR"/*.decline 2>/dev/null || true
    export HOWLDROP_ASK_DIR="$ASK_DIR"
fi
exec /run.sh
