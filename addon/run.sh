#!/bin/bash
# AirDrop on Home Assistant OS, and the Linux part of howldrop.exe in WSL
# (wsl-run.sh runs this): howldrop-agent brings the radio up on a Wi-Fi card
# the host kernel already drives (Owly's radio, in the mode the card's
# driver needs), receives into a staging folder and sends what is put in
# the outbox; deliver.py moves what arrives into save_folder; the panel
# shows it; howldrop-bridge forwards it to a PC running howldrop.exe and
# relays that PC's sends to the iPhone.
set -uo pipefail

AGENT=/usr/local/bin/howldrop-agent
BRIDGE=/usr/local/bin/howldrop-bridge
# Where the phone's files and text land: save_folder, in Home Assistant's
# share folder by default, which its Samba share shows to every PC.
INBOX=/share/howldrop
# The agent writes here; deliver.py moves each finished item to INBOX,
# fetching the file behind a link when the link points at one.
STAGING=/data/staging
DELIVER=/deliver.py
# The sidebar panel (ingress), and who sent each file for it.
PANEL=/panel.py
HISTORY=/data/drops.jsonl
# send_to_iphone: what the agent says about each send, and the latest scan
# for nearby devices (JSON for the panel, a .txt twin for the relay), and
# the file whose appearance asks for a scan.
SEND_STATUS=/data/send-status.json
NEARBY=/data/send-nearby.json
SCAN_REQUEST=${HOWLDROP_SCAN_REQUEST:-/data/send-scan.request}
# howldrop.exe running this in WSL (wsl-run.sh) gives its own outbox
# (HOWLDROP_OUTBOX), reads the agent's events (HOWLDROP_EVENTS=stdout), has
# no use for the panel (HOWLDROP_PANEL=off), sends the Bluetooth wake from
# Windows (HOWLDROP_BLE=off), and may answer asks (HOWLDROP_ASK_DIR).

AGENT_PID=
DELIVER_PID=
PANEL_PID=

say() { printf '%s\n' "$*"; }

option() {
    python3 - "$1" <<'PY'
import json, sys
try:
    with open("/data/options.json") as handle:
        data = json.load(handle)
except FileNotFoundError:
    data = {}
value = data.get(sys.argv[1], "")
if isinstance(value, list):
    value = ",".join(str(v).strip() for v in value if str(v).strip())
elif isinstance(value, bool):
    value = "true" if value else "false"
elif not isinstance(value, str):
    value = str(value)
print(value.strip())
PY
}

is_true() {
    case "$1" in True|true|1|yes|on) return 0 ;; esac
    return 1
}

# host_network puts this process in the host netns. If Supervisor left the
# container on the hassio bridge, host_pid lets us enter the host netns
# before any radio command runs.
join_host_netns() {
    local self host
    [ "${HOWLDROP_JOINED_HOST_NETNS:-}" = 1 ] && return 0
    [ -e /proc/1/ns/net ] || return 0
    self=$(readlink /proc/self/ns/net 2>/dev/null || true)
    host=$(readlink /proc/1/ns/net 2>/dev/null || true)
    [ -n "$self" ] && [ -n "$host" ] && [ "$self" != "$host" ] || return 0
    command -v nsenter >/dev/null 2>&1 || return 0
    export HOWLDROP_JOINED_HOST_NETNS=1
    say "This container is not in the host network namespace. Joining the host network namespace."
    # execfail: a failed exec returns here instead of ending the script.
    shopt -s execfail
    # shellcheck disable=SC2093
    exec nsenter -t 1 -n -- "$0"
    say "Joining the host network namespace failed."
    exit 1
}

# The host's system bus: NetworkManager lets go of the card through it, and
# BlueZ sends the Bluetooth wake.
dbus_host() {
    local s
    for s in /run/dbus/system_bus_socket /var/run/dbus/system_bus_socket; do
        if [ -S "$s" ]; then
            export DBUS_SYSTEM_BUS_ADDRESS=unix:path=$s
            return 0
        fi
    done
    return 1
}

stop_stack() {
    rm -f /tmp/howldrop-running
    # The agent's TERM gives the card back; wait for it to finish.
    if [ -n "$AGENT_PID" ]; then
        kill -TERM "$AGENT_PID" 2>/dev/null || true
        wait "$AGENT_PID" 2>/dev/null || true
    fi
    pkill -f "$BRIDGE watch" 2>/dev/null || true
    pkill -f "$BRIDGE relay" 2>/dev/null || true
    pkill -f "tcpdump -Z root" 2>/dev/null || true
    if [ -n "$DELIVER_PID" ]; then kill "$DELIVER_PID" 2>/dev/null || true; fi
    pkill -f "$DELIVER" 2>/dev/null || true
    if [ -n "$PANEL_PID" ]; then kill "$PANEL_PID" 2>/dev/null || true; fi
    pkill -f "$PANEL" 2>/dev/null || true
}

trap 'stop_stack; exit 0' TERM INT

join_host_netns

say "HowlDrop $(cat /etc/howldrop-version 2>/dev/null || echo add-on)"
say "It uses a Wi-Fi card the host kernel already drives. Files and text from the phone are kept in save_folder (default /share/howldrop)."

if ! dbus_host && [ "${HOWLDROP_BLE:-on}" != off ]; then
    say "The host D-Bus socket is not in this container: Bluetooth and NetworkManager can't be reached."
fi

SAVE_FOLDER=$(option save_folder)
case "${SAVE_FOLDER:-/share/howldrop}" in
    /*) INBOX=${SAVE_FOLDER:-/share/howldrop} ;;
    *) say "save_folder must be an absolute path such as /share/howldrop; using /share/howldrop." ;;
esac
# What is AirDropped to the iPhone: save_folder/send, kept afterwards in its
# sent/ and failed/. howldrop.exe's own outbox holds copies, deleted once tried.
SEND_DIR=${HOWLDROP_OUTBOX:-$INBOX/send}
FETCH=$(option fetch_links)
FETCH_MB=$(option fetch_max_mb)
case "$FETCH_MB" in ""|*[!0-9]*) FETCH_MB=100 ;; esac
# Files older than this many days are deleted from save_folder; 0 keeps them.
KEEP_DAYS=$(option delete_after_days)
case "$KEEP_DAYS" in ""|*[!0-9]*) KEEP_DAYS=30 ;; esac
SEND=false
is_true "$(option send_to_iphone)" && SEND=true

# The panel in the sidebar: recent drops in save_folder. Started now, so it
# answers while the radio comes up. Only the Supervisor's ingress proxy may
# use it (panel.py).
PANEL_ARGS=(--dest "$INBOX" --history "$HISTORY")
[ "$SEND" != true ] || PANEL_ARGS+=(--outbox "$SEND_DIR" --send-status "$SEND_STATUS" --nearby "$NEARBY"
                                     --scan-request "$SCAN_REQUEST")
if [ "${HOWLDROP_PANEL:-on}" != off ]; then
    python3 "$PANEL" "${PANEL_ARGS[@]}" 2>&1 | sed -u 's/^/Panel: /' &
    PANEL_PID=$!
fi

# Off unless asked for: the share folder is the way to use this, and a PC
# running howldrop.exe is an extra.
FORWARD=false
is_true "$(option forward_to_pc)" && FORWARD=true
PC_HOST=$(option pc_host)
PC_NAME=$(option pc_name)
# The sheet name: pc_name (HowlDrop by default). Left empty with
# forward_to_pc on, it is the name a running howldrop.exe announces, so the
# sheet shows that PC.
PC_BEACON=
if [ -z "$PC_NAME" ] && [ "$FORWARD" = true ]; then
    say "Listening for howldrop.exe on UDP 47631 for up to 10 s, for the share-sheet name."
    if PC_BEACON=$("$BRIDGE" name --timeout 10 2>/dev/null) && [ -n "$PC_BEACON" ]; then
        PC_NAME=$PC_BEACON
    else
        PC_BEACON=
        say "No howldrop.exe answered. The sheet shows HowlDrop; set pc_name to choose the name."
    fi
fi
PC_NAME=${PC_NAME:-HowlDrop}
if [ "${#PC_NAME}" -gt 63 ]; then
    say "The PC name is longer than 63 characters. The AirDrop sheet uses the first 63."
    PC_NAME=${PC_NAME:0:63}
fi

mkdir -p "$INBOX/text" "$STAGING" /data/owly || { say "Could not create ${INBOX}."; exit 1; }
touch /tmp/howldrop-running

# Who may send: the receive option (howldrop.exe's), else Contacts Only
# when senders are set and everyone when not.
SENDERS=$(option senders)
RECEIVE=$(option receive)
if [ -z "$RECEIVE" ]; then
    if [ -n "$SENDERS" ]; then RECEIVE=contacts; else RECEIVE=everyone; fi
fi

AGENT_ARGS=(--name "$PC_NAME" --staging "$STAGING" --identity /data/owly
            --receive "$RECEIVE" --senders "$SENDERS"
            --interface "$(option interface)" --channels "$(option channels)"
            --aggregation "$(option aggregation)")
[ -z "${HOWLDROP_ASK_DIR:-}" ] || AGENT_ARGS+=(--ask-dir "$HOWLDROP_ASK_DIR")
is_true "$(option allow_untested)" && AGENT_ARGS+=(--allow-untested)
is_true "$(option verbose)" && AGENT_ARGS+=(--verbose)
[ "${HOWLDROP_EVENTS:-}" != stdout ] || AGENT_ARGS+=(--events)
if [ "$SEND" = true ]; then
    mkdir -p "$SEND_DIR"
    AGENT_ARGS+=(--outbox "$SEND_DIR" --send-status "$SEND_STATUS" --nearby "$NEARBY"
                 --scan-request "$SCAN_REQUEST" --to "$(option send_to)")
    [ -z "${HOWLDROP_OUTBOX:-}" ] || AGENT_ARGS+=(--discard)
    [ "${HOWLDROP_BLE:-on}" = off ] || AGENT_ARGS+=(--ble)
fi

# Delivery first, so an item the agent stores is moved as soon as it is
# whole. Anything left in staging by an earlier run goes too.
DELIVER_ARGS=(--staging "$STAGING" --dest "$INBOX" --max-mb "$FETCH_MB" --keep-days "$KEEP_DAYS"
              --options /data/options.json --history "$HISTORY")
is_true "${FETCH:-true}" || DELIVER_ARGS+=(--no-fetch)
python3 "$DELIVER" "${DELIVER_ARGS[@]}" 2>&1 | sed -u 's/^/Deliver: /' &
DELIVER_PID=$!

"$AGENT" "${AGENT_ARGS[@]}" &
AGENT_PID=$!

# The PC is optional. The forwarder leaves every file in save_folder
# (--keep), sends only what arrives from now on, and keeps looking for a PC
# when none answers or the one it had goes away (--wait). It filters by name
# only when that name came from a PC's own beacon. Restarted if it exits.
# The PC's pairing key (pc_key), for the forwarder and the iPhone relay:
# given to them in their environment, not their arguments.
PC_KEY=$(option pc_key)
pc_key_ok() {
    [[ $(printf '%s' "$PC_KEY" | tr -d ' -' | tr 'A-F' 'a-f') =~ ^[0-9a-f]{32}$ ]]
}
if [ "$FORWARD" = true ]; then
    BRIDGE_ARGS=(watch --inbox "$INBOX" --keep --wait --timeout 10)
    [ -z "$PC_BEACON" ] || BRIDGE_ARGS+=(--name "$PC_BEACON")
    [ -z "$PC_HOST" ] || BRIDGE_ARGS+=(--host "$PC_HOST")
    if [ -z "$PC_KEY" ]; then
        say "pc_key is empty: howldrop.exe takes files only from senders with its pairing key. On the PC, right-click the HowlDrop icon, Copy pairing key, and paste it into pc_key."
    elif ! pc_key_ok; then
        say "pc_key is not a pairing key (8 groups of 4 digits and letters a-f). Copy it again from the HowlDrop icon's menu on the PC. Not forwarding until it is fixed."
        FORWARD=false
    fi
fi
if [ "$FORWARD" = true ]; then
    say "Forwarding new files to howldrop.exe whenever a PC runs it; they stay in ${INBOX} either way. Text goes to that PC's clipboard."
    (
        while [ -f /tmp/howldrop-running ]; do
            HOWLDROP_PC_KEY=$PC_KEY "$BRIDGE" "${BRIDGE_ARGS[@]}" 2>&1 | sed -u 's/^/Forward: /'
            [ -f /tmp/howldrop-running ] || break
            sleep 5
        done
    ) &
fi
# The relay: a paired PC sends here (tray, Explorer's Send to, `howldrop
# iphone`), into the send folder. Not under howldrop.exe in WSL: that PC
# writes into SEND_DIR itself.
if [ "$SEND" = true ] && [ -z "${HOWLDROP_OUTBOX:-}" ]; then
    if pc_key_ok; then
        (
            while [ -f /tmp/howldrop-running ]; do
                HOWLDROP_PC_KEY=$PC_KEY "$BRIDGE" relay --inbox "$SEND_DIR" \
                    --nearby "${NEARBY%.json}.txt" --scan-request "$SCAN_REQUEST" 2>&1 | sed -u 's/^/Relay: /'
                [ -f /tmp/howldrop-running ] || break
                sleep 5
            done
        ) &
    else
        say "A PC can send to the iPhone too once pc_key is set (its HowlDrop icon's menu, Copy pairing key)."
    fi
fi

# debug_capture: what the radio and the AirDrop TCP carried, as pcap files
# in save_folder/debug, a ring of 20 MB files, the oldest overwritten. -Z
# root: Debian's tcpdump otherwise drops to its own user after the first
# file and dies at the next one.
if is_true "$(option debug_capture)"; then
    mkdir -p "$INBOX/debug"
    (
        sleep 20
        tcpdump -Z root -i owlymon0 -s 256 -C 20 -W 8 -w "$INBOX/debug/owlymon0.pcap" >/dev/null 2>&1 &
        tcpdump -Z root -i awdl0 -s 256 -C 20 -W 4 -w "$INBOX/debug/awdl0.pcap" 'tcp port 8771' >/dev/null 2>&1 &
    ) &
    say "debug_capture: writing pcap files to ${INBOX}/debug once the window is up (owlymon0 up to 160 MB, awdl0 up to 80 MB)."
fi

wait "$AGENT_PID"
CODE=$?
AGENT_PID=
stop_stack
exit "$CODE"
