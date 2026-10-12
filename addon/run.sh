#!/bin/bash
# AirDrop receiver and owldrop-bridge on Home Assistant OS.
# Ethernet stays the uplink. The Intel Wireless-AC 8260 is whichever host
# interface is iwlwifi on PCI (often wlp2s0). It is used only for the
# released omdrop-owl iwlwifi exclusive window.
set -uo pipefail

LIB=/usr/lib/omdrop
DISCOVERABLE=$LIB/omdrop-discoverable
BLE=$LIB/ble-airdrop-adv.py
RECEIVER=/opt/omdrop-receiver/airdrop-serve.py
BRIDGE=/usr/local/bin/owldrop-bridge
INBOX=/data/incoming
BSSID=00:25:00:ff:94:73
IFACE=

BLE_PID=
SERVE_PID=
BRIDGE_PID=
RADIO=0

say() { printf '%s\n' "$*"; }

option() {
    python3 - "$1" <<'PY'
import json, sys
path = "/data/options.json"
try:
    with open(path) as handle:
        data = json.load(handle)
except FileNotFoundError:
    data = {}
value = data.get(sys.argv[1], "")
if not isinstance(value, str):
    value = str(value)
print(value.strip())
PY
}

dbus_host() {
    if [ -S /run/dbus/system_bus_socket ]; then
        export DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket
        return 0
    fi
    if [ -S /var/run/dbus/system_bus_socket ]; then
        export DBUS_SYSTEM_BUS_ADDRESS=unix:path=/var/run/dbus/system_bus_socket
        return 0
    fi
    return 1
}

driver_of() {
    basename "$(readlink -f "/sys/class/net/$1/device/driver" 2>/dev/null)" 2>/dev/null || true
}

pci_of() {
    local dev="/sys/class/net/$1/device"
    local vendor device
    vendor=$(cat "$dev/vendor" 2>/dev/null || true)
    device=$(cat "$dev/device" 2>/dev/null || true)
    printf '%s:%s' "${vendor#0x}" "${device#0x}"
}

firmware_of() {
    ethtool -i "$1" 2>/dev/null | awk -F': ' '/^firmware-version:/ { print $2; exit }'
}

# 8260 PCI ids. A lone iwlwifi PCI radio is still accepted when the id
# is a later firmware revision of the same card.
radio_is_8260() {
    case "$(pci_of "$1")" in
        8086:24f3|8086:24f4|8086:24f5|8086:24f6) return 0 ;;
    esac
    return 1
}

find_radio() {
    local d name drv sub chosen="" only="" count=0
    for d in /sys/class/net/*; do
        [ -d "$d/phy80211" ] || continue
        name=$(basename "$d")
        case "$name" in
            awdl0|mon0|awdlibss0) continue ;;
        esac
        drv=$(driver_of "$name")
        [ "$drv" = iwlwifi ] || continue
        sub=$(basename "$(readlink -f "$d/device/subsystem" 2>/dev/null)" 2>/dev/null || true)
        [ "$sub" = pci ] || continue
        count=$((count + 1))
        only=$name
        if [ -z "$chosen" ] && radio_is_8260 "$name"; then
            chosen=$name
        fi
    done
    if [ -n "$chosen" ]; then
        printf '%s\n' "$chosen"
        return 0
    fi
    if [ "$count" -eq 1 ]; then
        printf '%s\n' "$only"
        return 0
    fi
    return 1
}

list_ifaces() {
    local d name
    for d in /sys/class/net/*; do
        [ -e "$d" ] || continue
        name=$(basename "$d")
        say "  ${name}: driver $(driver_of "$name"), PCI $(pci_of "$name")"
    done
}

# host_network puts this process in the host netns. If Supervisor left the
# container on the hassio bridge, host_pid lets us enter the host netns
# before any radio command runs. The exclusive IBSS profile is unchanged.
join_host_netns() {
    local self host
    [ "${OWLDROP_JOINED_HOST_NETNS:-}" = 1 ] && return 0
    [ -e /proc/1/ns/net ] || return 0
    self=$(readlink /proc/self/ns/net 2>/dev/null || true)
    host=$(readlink /proc/1/ns/net 2>/dev/null || true)
    [ -n "$self" ] && [ -n "$host" ] && [ "$self" != "$host" ] || return 0
    command -v nsenter >/dev/null 2>&1 || return 0
    export OWLDROP_JOINED_HOST_NETNS=1
    say "This container is not in the host network namespace. Joining the host network namespace."
    exec nsenter -t 1 -n -- "$0"
    say "Joining the host network namespace failed."
    exit 1
}

iface_is_up() {
    local flags
    flags=$(cat "/sys/class/net/$1/flags" 2>/dev/null || echo 0)
    [ $((flags & 1)) -eq 1 ]
}

wpa_pid() {
    local d comm
    for d in /proc/[0-9]*; do
        # The shell opens a redirect itself. A pid that exits between the
        # glob and that open prints "No such file or directory" on this line.
        [ -r "$d/cmdline" ] || continue
        comm=$(cat "$d/cmdline" 2>/dev/null | tr '\0' ' ' || true)
        case "$comm" in
            *wpa_supplicant*)
                basename "$d"
                return 0
                ;;
        esac
    done
    return 1
}

supplicant_has_iface() {
    local pid=$1
    [ -n "$pid" ] || return 1
    if [ -S "/proc/$pid/root/run/wpa_supplicant/$IFACE" ] \
        || [ -S "/proc/$pid/root/var/run/wpa_supplicant/$IFACE" ]; then
        return 0
    fi
    if [ -r "/proc/$pid/cmdline" ] \
        && cat "/proc/$pid/cmdline" 2>/dev/null | tr '\0' ' ' | grep -q -- "$IFACE"; then
        return 0
    fi
    # Host /run is not visible from here. NetworkManager's supplicant still
    # owns every Wi-Fi interface it was started with.
    if [ ! -d "/proc/$pid/root/run" ]; then
        return 0
    fi
    return 1
}

dbus_remove_iface() {
    timeout 2 python3 - "$IFACE" <<'PY' || true
import sys
try:
    import dbus
except Exception:
    sys.exit(0)
name = sys.argv[1]
try:
    bus = dbus.SystemBus()
    obj = bus.get_object("fi.w1.wpa_supplicant1", "/fi/w1/wpa_supplicant1")
    wpa = dbus.Interface(obj, "fi.w1.wpa_supplicant1")
    wpa.RemoveInterface(wpa.GetInterface(name))
except Exception:
    sys.exit(0)
PY
}

# nmcli unmanaged leaves the host wpa_supplicant attached, and that hold
# makes IBSS setup return EOPNOTSUPP. Drop the interface, then stop the
# process if it still has the radio.
release_supplicant() {
    local n pid
    dbus_remove_iface
    for n in 1 2 3 4 5; do
        pid=$(wpa_pid || true)
        [ -n "${pid:-}" ] || break
        if ! supplicant_has_iface "$pid"; then
            break
        fi
        kill "$pid" 2>/dev/null || true
        sleep 0.2
        nmcli device set "$IFACE" managed no >/dev/null 2>&1 || true
        dbus_remove_iface
    done
}

iface_type() {
    iw dev "$1" info 2>/dev/null | awk '/type / { print $2; exit }'
}

# iw prints the iftype as "IBSS". The set-type argument is "ibss".
type_is_ibss() {
    case "$1" in
        [Ii][Bb][Ss][Ss]) return 0 ;;
        *) return 1 ;;
    esac
}

mark_station_converted() {
    local ipv6
    ipv6=$(sysctl -n "net.ipv6.conf.${IFACE}.disable_ipv6" 2>/dev/null || echo 0)
    printf '%s\n' "$ipv6" > /run/omdrop-owl/converted
}

# iwlmvm advertises IBSS, but its interface combinations do not list it.
# mac80211 allows the bring-up only when this netdev is the only one running
# on the phy. A live P2P device or a second vif makes `ip link set up`
# return EBUSY.
drop_other_ifaces() {
    local keep="${1:-}" phy name d
    phy=$(readlink -f "/sys/class/net/$IFACE/phy80211" 2>/dev/null || true)
    for d in /sys/class/net/*; do
        [ -d "$d/phy80211" ] || continue
        name=$(basename "$d")
        [ "$name" = "$IFACE" ] && continue
        [ -n "$keep" ] && [ "$name" = "$keep" ] && continue
        if [ -n "$phy" ] && [ "$(readlink -f "$d/phy80211")" != "$phy" ]; then
            continue
        fi
        say "Deleting ${name} so ${IFACE} is the only interface."
        if command -v nmcli >/dev/null 2>&1; then
            nmcli device set "$name" managed no >/dev/null 2>&1 || true
        fi
        ip link set "$name" down 2>/dev/null || true
        iw dev "$name" del 2>/dev/null || true
    done
}

# JOIN_IBSS returns EOPNOTSUPP unless the netdev is already type ibss.
# Set that type while the interface is down. The caller brings it up next.
set_ibss_while_down() {
    local err cur
    err=$(ip link set "$1" down 2>&1) || true
    if iface_is_up "$1"; then
        [ -n "$err" ] && printf '%s\n' "$err"
    fi
    err=$(iw dev "$1" set type ibss 2>&1) || {
        printf '%s\n' "$err"
        return 1
    }
    cur=$(iface_type "$1")
    if ! type_is_ibss "$cur"; then
        say "Switching ${1} to IBSS was refused. iw reports type ${cur:-unknown}."
        return 1
    fi
    say "Set ${1} to type ibss while it was down."
    return 0
}

# One delete. A name that already exists must not be created again.
drop_stale_vif() {
    say "Deleting stale awdlibss0 once."
    iw dev awdlibss0 del 2>/dev/null || true
}

# Pick awdlibss0 when it can be created. Leave it down. The type is set
# in ready_cell, while the interface is down, and only then is it brought up.
prepare_cell() {
    local phy
    mkdir -p /run/omdrop-owl
    printf '%s\n' "$IFACE" > /run/omdrop-owl/exclusive
    phy=$(basename "$(readlink -f "/sys/class/net/$IFACE/phy80211")")
    CELL=$IFACE
    if [ ! -d /sys/class/net/awdlibss0 ]; then
        if ! iw phy "$phy" interface add awdlibss0 type ibss 2>/tmp/owldrop-iw.err; then
            [ -s /tmp/owldrop-iw.err ] && cat /tmp/owldrop-iw.err
            if [ -d /sys/class/net/awdlibss0 ]; then
                drop_stale_vif
            fi
            say "A second IBSS interface cannot be used. ${IFACE} becomes the cell."
            return 0
        fi
        if command -v nmcli >/dev/null 2>&1; then
            nmcli device set awdlibss0 managed no >/dev/null 2>&1 || true
        fi
        ip link set awdlibss0 down 2>/dev/null || true
        ip link set "$IFACE" down 2>/dev/null || true
        ip link set awdlibss0 address "$(cat "/sys/class/net/$IFACE/address" 2>/dev/null)" 2>/tmp/owldrop-addr.err || true
        if [ -s /tmp/owldrop-addr.err ]; then
            cat /tmp/owldrop-addr.err
        fi
    fi
    if [ -d /sys/class/net/awdlibss0 ]; then
        rm -f /run/omdrop-owl/converted
        CELL=awdlibss0
        return 0
    fi
    say "A second IBSS interface cannot be used. ${IFACE} becomes the cell."
    return 0
}

# awdlibss0 is gone before the station is switched, so the station is the
# only interface on this phy when its type changes.
station_alone() {
    release_supplicant
    ip link set "$IFACE" down 2>/dev/null || true
    drop_other_ifaces
    if [ -d /sys/class/net/awdlibss0 ]; then
        say "awdlibss0 is still present, so ${IFACE} was not switched."
        return 1
    fi
    CELL=$IFACE
    mark_station_converted
    set_ibss_while_down "$IFACE" || return 1
    # NetworkManager restarts wpa_supplicant, which brings a P2P device back.
    release_supplicant
    drop_other_ifaces
    bring_up "$IFACE"
}

# Set type ibss while down, then bring that interface up. If the second vif
# cannot take the type, delete it and do the same on the station alone.
ready_cell() {
    if [ "$CELL" != "$IFACE" ]; then
        release_supplicant
        drop_other_ifaces "$CELL"
        ip link set "$IFACE" down 2>/dev/null || true
        if set_ibss_while_down "$CELL" && bring_up "$CELL"; then
            return 0
        fi
        say "A second IBSS interface cannot be type ibss. Deleting awdlibss0 so ${IFACE} is the only interface."
        station_alone
        return $?
    fi
    station_alone
}

bring_up() {
    local err keep
    if iface_is_up "$1"; then
        return 0
    fi
    say "Bringing ${1} up before the IBSS join."
    err=$(ip link set "$1" up 2>&1) || true
    if ! iface_is_up "$1"; then
        release_supplicant
        if [ "$1" = "$IFACE" ]; then
            keep=
        else
            keep=$1
        fi
        drop_other_ifaces "$keep"
        err=$(ip link set "$1" up 2>&1) || true
    fi
    if iface_is_up "$1"; then
        return 0
    fi
    [ -n "$err" ] && printf '%s\n' "$err"
    say "${1} stayed down."
    iw dev 2>/dev/null || true
    return 1
}

freq_name() {
    case "$1" in
        5220) printf '%s\n' "5220 MHz (channel 44)" ;;
        5745) printf '%s\n' "5745 MHz (channel 149)" ;;
        2437) printf '%s\n' "2437 MHz (channel 6)" ;;
        *) printf '%s\n' "$1 MHz" ;;
    esac
}

# Released order: channel 44, channel 149, then channel 6. A frequency the
# phy marks disabled, No IR, or radar is not offered. An empty result means
# the list could not be read; it does not mean 5 GHz is usable.
allowed_airdrop_freqs() {
    local phy=$1 ok c
    ok=$(iw phy "$phy" channels 2>/dev/null | awk '
        function mhz(s) { sub(/[^0-9].*/, "", s); return s + 0 }
        /^[[:space:]]*\* [0-9]/ {
            if (f && ok) print f
            f = mhz($2)
            line = tolower($0)
            ok = (line !~ /disabled/ && line !~ /no ir/ && line !~ /radar/)
            next
        }
        {
            line = tolower($0)
            if (line ~ /no ir/ || line ~ /radar/ || line ~ /disabled/) ok = 0
        }
        END { if (f && ok) print f }
    ')
    [ -n "$ok" ] || return 0
    for c in 5220 5745 2437; do
        printf '%s\n' "$ok" | grep -qx "$c" && printf '%s\n' "$c"
    done
}

# IP for AirDrop stays on awdl0. Left enabled, the kernel treats the IBSS
# cell as a network and sends plain IPv6 on AirDrop's BSSID.
silence_cell_ipv6() {
    sysctl -qw "net.ipv6.conf.$1.disable_ipv6=1" 2>/dev/null || true
    say "IPv6 on ${1} is off so the kernel does not answer on the AirDrop BSSID. Addresses stay on awdl0."
}

cell_joined_freq() {
    iw dev "$1" link 2>/dev/null | awk '/^Joined IBSS/ { joined = 1 } joined && /freq:/ { print int($2); exit }'
}

# HT20, fixed BSSID, beacon interval 10000. The frequency is the only word
# that changes. A down interface is brought up once and the same join is
# retried. A join that never reports Joined IBSS is left before the next.
try_join_freq() {
    local dev=$1 freq=$2 err i got
    say "Joining the released exclusive IBSS on $(freq_name "$freq"), HT20, BSSID ${BSSID}."
    err=$(iw dev "$dev" ibss join omdrop "$freq" HT20 fixed-freq "$BSSID" beacon-interval 10000 2>&1) || {
        JOIN_ERR=$err
        printf '%s\n' "$err"
        if printf '%s\n' "$err" | grep -q 'Network is down'; then
            bring_up "$dev" || true
            err=$(iw dev "$dev" ibss join omdrop "$freq" HT20 fixed-freq "$BSSID" beacon-interval 10000 2>&1) || {
                JOIN_ERR=$err
                printf '%s\n' "$err"
                return 1
            }
        else
            return 1
        fi
    }
    for i in $(seq 100); do
        got=$(cell_joined_freq "$dev")
        if [ "$got" = "$freq" ]; then
            JOINED_FREQ=$freq
            return 0
        fi
        sleep 0.1
    done
    iw dev "$dev" ibss leave 2>/dev/null || true
    say "$(freq_name "$freq") did not report Joined IBSS."
    return 1
}

reg_country() {
    iw reg get 2>/dev/null | awk '
        /^global/ { global = 1 }
        global && $1 == "country" { c = $2; sub(/:.*/, "", c); print c; exit }
    '
}

five_ghz_open() {
    local allowed=$1 freq
    for freq in 5220 5745; do
        printf '%s\n' "$allowed" | grep -qx "$freq" && return 0
    done
    return 1
}

phy_of() {
    basename "$(readlink -f "/sys/class/net/$1/phy80211" 2>/dev/null)" 2>/dev/null
}

report_5ghz() {
    local allowed=$1 when=$2 freq label
    if [ -z "$allowed" ]; then
        say "The channel list could not be read ${when}."
        return 0
    fi
    for freq in 5220 5745; do
        label=$(freq_name "$freq")
        if printf '%s\n' "$allowed" | grep -qx "$freq"; then
            say "${label} is usable ${when}."
        else
            say "${label} is disabled or No IR ${when}."
        fi
    done
}

# Hear a 5 GHz network. Intel keeps those channels receive-only until one
# is heard. The join arguments are not involved.
scan_for_5ghz() {
    local err rc
    if ! iface_is_up "$1"; then
        ip link set "$1" up 2>/dev/null || true
    fi
    say "Scanning once on ${1} so the card can hear a 5 GHz network."
    if command -v timeout >/dev/null 2>&1; then
        err=$(timeout 25 iw dev "$1" scan >/dev/null 2>&1)
        rc=$?
    else
        err=$(iw dev "$1" scan >/dev/null 2>&1)
        rc=$?
    fi
    if [ "$rc" -ne 0 ]; then
        [ -n "$err" ] && printf '%s\n' "$err"
        say "The scan on ${1} did not finish."
        return 1
    fi
    return 0
}

# Country 00 leaves 5 GHz receive-only. Set US before trusting the channel
# list. An unreadable list is not treated as usable.
allow_5ghz() {
    local phy country allowed err
    phy=$(phy_of "$1")
    country=$(reg_country)
    say "Regulatory domain: ${country:-unread}."
    if iw reg get 2>/dev/null | grep -q 'self-managed'; then
        say "This radio keeps its own regulatory domain."
    fi
    case "${country:-00}" in
        00|99|"")
            say "The regulatory domain is unset, which leaves 5 GHz receive-only. Setting it to US."
            err=$(iw reg set US 2>&1) || printf '%s\n' "$err"
            sleep 0.5
            ;;
    esac
    allowed=$(allowed_airdrop_freqs "$phy")
    if five_ghz_open "$allowed"; then
        report_5ghz "$allowed" "before the scan"
        return 0
    fi
    report_5ghz "$allowed" "before the scan"
    scan_for_5ghz "$1" || true
    allowed=$(allowed_airdrop_freqs "$phy")
    report_5ghz "$allowed" "after the station scan"
}

join_released() {
    local dev=$1 phy freq allowed
    phy=$(phy_of "$dev")
    silence_cell_ipv6 "$dev"
    iw dev "$dev" ibss leave 2>/dev/null || true
    allowed=$(allowed_airdrop_freqs "$phy")
    if ! five_ghz_open "$allowed"; then
        say "5 GHz is still closed after the interface was brought up. Scanning on ${dev}."
        scan_for_5ghz "$dev" || true
        allowed=$(allowed_airdrop_freqs "$phy")
        report_5ghz "$allowed" "after the cell scan"
    fi
    if [ -z "$allowed" ]; then
        say "The channel list could not be read. Trying channel 6."
        allowed=2437
    fi
    if ! printf '%s\n' "$allowed" | grep -qx 5220; then
        say "5220 MHz (channel 44) will not be tried."
    fi
    if ! printf '%s\n' "$allowed" | grep -qx 5745; then
        say "5745 MHz (channel 149) will not be tried."
    fi
    JOINED_FREQ=
    for freq in $allowed; do
        try_join_freq "$dev" "$freq" && return 0
    done
    return 1
}

explain_join_failure() {
    if printf '%s\n' "${JOIN_ERR:-}" | grep -q 'Operation not supported'; then
        say "The IBSS join itself returned operation not supported after ${1} was up and wpa_supplicant had released it."
    else
        say "The IBSS cell did not start."
    fi
}

# The receiver refuses to create a self-signed pair while it is root
# (self-signed-missing). The pair lives in the add-on data, which persists.
ensure_identity() {
    local base cert key tmp err
    base=${OWLDROP_KEYS_DIR:-/data/omdrop}
    mkdir -p "$base/keys" || {
        say "Could not create ${base}/keys."
        return 1
    }
    chmod 700 "$base/keys" 2>/dev/null || true
    cert=$base/keys/certificate.self-signed.pem
    key=$base/keys/key.self-signed.pem
    if [ -f "$cert" ] && [ -f "$key" ]; then
        say "Using the self-signed AirDrop identity already in the add-on data."
        return 0
    fi
    if [ -e "$cert" ] || [ -e "$key" ]; then
        say "The AirDrop identity in the add-on data is only half a pair, so the receiver will not start."
        return 1
    fi
    tmp=$(mktemp -d)
    err=$(openssl req -newkey rsa:2048 -nodes -x509 -days 365 \
        -subj "/CN=${PC_NAME}" \
        -keyout "$tmp/key.pem" -out "$tmp/certificate.pem" 2>&1) || {
        printf '%s\n' "$err"
        rm -rf "$tmp"
        say "Could not create the self-signed AirDrop identity."
        return 1
    }
    chmod 600 "$tmp/key.pem" "$tmp/certificate.pem" || true
    if ! install -m 600 "$tmp/key.pem" "$key" \
        || ! install -m 600 "$tmp/certificate.pem" "$cert"; then
        rm -f "$key" "$cert"
        rm -rf "$tmp"
        say "Could not install the self-signed AirDrop identity."
        return 1
    fi
    rm -rf "$tmp"
    say "Created the self-signed AirDrop identity. The receiver runs as root and will not create one itself."
    return 0
}

linklocal_from_mac() {
    python3 - "$1" <<'PY'
import sys
parts = [int(x, 16) for x in sys.argv[1].split(":")]
parts[0] ^= 0x02
bits = parts[:3] + [0xFF, 0xFE] + parts[3:]
words = [f"{bits[i] << 8 | bits[i + 1]:x}" for i in range(0, 8, 2)]
print("fe80::" + ":".join(words))
PY
}

# Print "address ready" or "address tentative". A ready address wins.
awdl_linklocal() {
    ip -6 addr show dev awdl0 scope link 2>/dev/null | awk '
        $1 == "inet6" {
            addr = $2
            sub(/\/.*/, "", addr)
            state = "ready"
            for (i = 1; i <= NF; i++) if ($i == "tentative") state = "tentative"
            if (state == "ready") { print addr, state; done = 1; exit }
            if (!saved) saved = addr
        }
        END { if (!done && saved) print saved, "tentative" }
    '
}

# A link-local that is still tentative cannot be reached. Duplicate-address
# detection does that on awdl0; the released window clears it by adding the
# same address again with nodad. Leave a ready address alone.
ensure_awdl_ipv6() {
    local addr state
    if [ ! -d /sys/class/net/awdl0 ]; then
        say "awdl0 is not present, so the AirDrop receiver has nothing to bind."
        return 1
    fi
    read -r addr state <<EOF
$(awdl_linklocal)
EOF
    if [ "${state:-}" = ready ] && [ -n "${addr:-}" ]; then
        say "awdl0 IPv6 address is ${addr} and it is ready."
        return 0
    fi
    sysctl -qw net.ipv6.conf.awdl0.accept_dad=0 2>/dev/null || true
    if [ -n "${addr:-}" ]; then
        say "awdl0 IPv6 address ${addr} is still tentative. Adding it again without duplicate-address detection."
        ip -6 addr del "${addr}/64" dev awdl0 2>/dev/null || true
    else
        if [ -w /proc/sys/net/ipv6/conf/awdl0/disable_ipv6 ]; then
            printf '0\n' > /proc/sys/net/ipv6/conf/awdl0/disable_ipv6 || true
        fi
        ip link set awdl0 up 2>/dev/null || true
        addr=$(linklocal_from_mac "$(cat /sys/class/net/awdl0/address)")
        say "awdl0 has no IPv6 address. Adding link-local ${addr} without duplicate-address detection."
    fi
    ip -6 addr add "${addr}/64" dev awdl0 scope link nodad 2>/dev/null || true
    read -r addr state <<EOF
$(awdl_linklocal)
EOF
    if [ "${state:-}" != ready ] || [ -z "${addr:-}" ]; then
        say "awdl0 still has no usable IPv6 address, so the AirDrop receiver will not bind."
        return 1
    fi
    say "awdl0 IPv6 address is ${addr} and it is ready."
    return 0
}

# Keep a cell this script already joined. exclusive_up would take the
# interface down and join again, which is the ENETDOWN failure.
keep_joined_cell() {
    python3 - "$DISCOVERABLE" <<'PY' || true
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
marker = "A cell already joined by the add-on"
if marker in text:
    sys.exit(0)
anchor = '  echo "$sta" > "$RUN/exclusive"\n'
insert = anchor + '''  # A cell already joined by the add-on (channel 6, interface up) stays up.
  # Tearing it down here put the join on a down interface (ENETDOWN).
  if iw dev "$IBSS" link 2>/dev/null | grep -q '^Joined IBSS'; then
    iw dev "$IBSS" link | awk '/freq:/ {print int($2); exit}'
    return 0
  fi
  if iw dev "$sta" link 2>/dev/null | grep -q '^Joined IBSS'; then
    iw dev "$sta" link | awk '/freq:/ {print int($2); exit}'
    return 0
  fi
'''
if anchor not in text:
    sys.exit(0)
text = text.replace(anchor, insert, 1)
recover = '''  if [ -e "$RUN/converted" ]; then
    cmd_stop recovered >/dev/null
    [ -e "$RUN/converted" ] && die "Wi-Fi ($(cat "$RUN/exclusive")) is still in ad-hoc mode from the last window and couldn\'t be switched back. Restarting the computer fixes it." 4
  fi
'''
keep = '''  if [ -e "$RUN/converted" ]; then
    if ! iw dev "$(cat "$RUN/exclusive" 2>/dev/null)" link 2>/dev/null | grep -q '^Joined IBSS'; then
      cmd_stop recovered >/dev/null
      [ -e "$RUN/converted" ] && die "Wi-Fi ($(cat "$RUN/exclusive")) is still in ad-hoc mode from the last window and couldn\'t be switched back. Restarting the computer fixes it." 4
    fi
  fi
'''
if recover in text:
    text = text.replace(recover, keep, 1)
up = '  ip link set "$cell" up || return 1\n'
wait = up + '''  local uptry
  for uptry in $(seq 50); do
    [ $(( $(cat "/sys/class/net/$cell/flags" 2>/dev/null || echo 0) & 1 )) -eq 1 ] && break
    ip link set "$cell" up 2>/dev/null || true
    sleep 0.05
  done
'''
if up in text:
    text = text.replace(up, wait, 1)
path.write_text(text)
PY
}

stop_stack() {
    rm -f /tmp/owldrop-running
    if [ -n "$BRIDGE_PID" ]; then kill "$BRIDGE_PID" 2>/dev/null || true; fi
    if [ -n "$SERVE_PID" ]; then kill "$SERVE_PID" 2>/dev/null || true; fi
    if [ -n "$BLE_PID" ]; then kill "$BLE_PID" 2>/dev/null || true; fi
    if [ "$RADIO" = 1 ]; then
        say "Stopping the AirDrop window and giving ${IFACE} back."
        "$DISCOVERABLE" stop || true
        RADIO=0
    fi
}

trap 'stop_stack; exit 0' TERM INT

join_host_netns

say "OwlDrop add-on"
say "Home Assistant OS stays installed. This add-on does not load iwlwifi and does not replace the firmware blob."
say ""
say "Wi-Fi on this NUC is the Intel Wireless-AC 8260. Ethernet on the I219-LM stays the uplink."
say "The radio profile is the released omdrop-owl iwlwifi line: exclusive mode, IBSS, BSSID ${BSSID}."
say "This window tries 5220 MHz (channel 44) and 5745 MHz (channel 149) first, then 2437 MHz (channel 6). Each join is HT20, fixed frequency, BSSID ${BSSID}, beacon interval 10000 TU."
say "A current iPhone spends most of its AirDrop time on channels 44 and 149 and only glances at channel 6. A cell left on channel 6 is why the tile appears and the send then dies in the handshake."
say "The Intel cards this profile was tested on refuse a 5 GHz cell. This 8260 is not an AX201 or an AX211, and that 5 GHz join has no report on iwlwifi-8000C-36.ucode. If 44 and 149 are refused, the window joins channel 6 and says so."
say "Until a phone finishes a transfer, ${INBOX} stays empty and owldrop.exe has nothing to receive."
say "Pull request 13 and a USB Wi-Fi stick are not used."
say ""

if ! dbus_host; then
    say "The host D-Bus socket is not in this container. Turn protection mode off and install the add-on with host D-Bus enabled."
    exit 1
fi

IFACE=$(find_radio || true)
if [ -z "$IFACE" ]; then
    say "No Intel iwlwifi PCI interface is visible in this network namespace."
    say "Interfaces here:"
    list_ifaces
    self_ns=$(readlink /proc/self/ns/net 2>/dev/null || true)
    host_ns=$(readlink /proc/1/ns/net 2>/dev/null || true)
    if [ -n "$self_ns" ] && [ -n "$host_ns" ] && [ "$self_ns" != "$host_ns" ]; then
        say "This container is still not in the host network namespace. Turn protection mode off so host_network, host_pid, and SYS_ADMIN apply, then update this add-on so Supervisor creates a new container."
    elif ! compgen -G '/sys/class/net/*/phy80211' >/dev/null; then
        say "No wireless phy is visible. The 8260 stays on the host until this container shares the host network namespace. Protection mode has to be off, and this version has to be installed so Supervisor recreates the container with host_network."
    else
        say "A wireless interface is visible, and it is not the Intel iwlwifi PCI radio. This add-on will not use another radio."
    fi
    exit 1
fi

DRV=$(driver_of "$IFACE")
PCI=$(pci_of "$IFACE")
FW=$(firmware_of "$IFACE")
say "Interface ${IFACE}: driver ${DRV:-unknown}, PCI ${PCI:-unknown}, firmware ${FW:-unread}."
say "The host names this radio. wlp2s0 is typical for the 8260. The name wlan0 is not required."
say "The firmware file a current Home Assistant OS image already has for this card is iwlwifi-8000C-36.ucode."

if [ "$DRV" != iwlwifi ]; then
    say "${IFACE} is ${DRV:-missing}, so this window will not start. The released profile this add-on runs is the iwlwifi exclusive profile."
    exit 1
fi

PC_HOST=$(option pc_host)
PC_NAME=$(option pc_name)
if [ -z "$PC_NAME" ]; then
    say "Listening for owldrop.exe on UDP 47631."
    if [ -n "$PC_HOST" ]; then
        say "pc_host is set, so the forwarder will use ${PC_HOST}. The share-sheet name still comes from the beacon unless pc_name is set."
    fi
    if ! PC_NAME=$("$BRIDGE" name --timeout 20); then
        say "No OwlDrop beacon arrived. Start owldrop.exe on the Windows PC and allow UDP 47631 and TCP 47632 from this LAN, or set pc_name to that PC's name."
        exit 1
    fi
fi
if [ "${#PC_NAME}" -gt 63 ]; then
    say "The PC name is longer than 63 characters. The AirDrop sheet uses the first 63."
    PC_NAME=${PC_NAME:0:63}
fi
say "The iPhone share sheet will show: ${PC_NAME}"

mkdir -p /etc/omdrop "$INBOX/text" /data/omdrop/keys /run/omdrop-owl || {
    say "Could not create ${INBOX}."
    exit 1
}
printf '%s\n' "$IFACE" > /etc/omdrop/infra-iface

TYPE=$(iface_type "$IFACE")
if type_is_ibss "$TYPE"; then
    say "${IFACE} is still an IBSS cell from an earlier window. Switching it back to a station before the profile runs."
    ip link set "$IFACE" down || true
    iw dev "$IFACE" set type managed || true
    ip link set "$IFACE" up || true
fi
iw dev mon0 del 2>/dev/null || true

allow_5ghz "$IFACE"

say "Marking ${IFACE} unmanaged so the supervisor leaves it out of the uplink."
nmcli device disconnect "$IFACE" >/dev/null 2>&1 || true
nmcli device set "$IFACE" managed no || true
say "A NetworkManager restart clears that runtime flag. The flag is set again the next time this add-on starts. This container does not write the host's /etc."
say "Stopping host wpa_supplicant from holding ${IFACE}."
release_supplicant

if ! prepare_cell || ! ready_cell; then
    "$DISCOVERABLE" stop >/dev/null 2>&1 || true
    nmcli device set "$IFACE" managed yes >/dev/null 2>&1 || true
    exit 1
fi
if ! join_released "$CELL"; then
    if ! iw dev "$CELL" link 2>/dev/null | grep -q '^Joined IBSS'; then
        explain_join_failure "$CELL"
        "$DISCOVERABLE" stop >/dev/null 2>&1 || true
        nmcli device set "$IFACE" managed yes >/dev/null 2>&1 || true
        exit 1
    fi
    JOINED_FREQ=$(cell_joined_freq "$CELL")
fi
say "Joined IBSS on $(freq_name "${JOINED_FREQ:-unknown}"), HT20, BSSID ${BSSID}."

keep_joined_cell
say "Starting the released exclusive window (until this add-on stops)."
if ! "$DISCOVERABLE" start 0; then
    say "The IBSS cell was joined, and the released window did not finish starting."
    stop_stack
    exit 1
fi
RADIO=1

CHAN=$(awk '/ on ch/ { chan=$0 } END { print chan }' /run/omdrop-owl/log 2>/dev/null || true)
say "Radio log: ${CHAN:-the helper wrote no channel line}"
if iw dev awdlibss0 link 2>/dev/null | grep -q 'Joined IBSS'; then
    iw dev awdlibss0 link
elif iw dev "$IFACE" link 2>/dev/null | grep -q 'Joined IBSS'; then
    say "The station itself is the cell."
    iw dev "$IFACE" link
else
    say "The helper returned, and no interface is reporting Joined IBSS yet."
fi
nmcli device set awdl0 managed no 2>/dev/null || true

touch /tmp/owldrop-running
say "Bluetooth wake: company id 0x004C, type 0x05, through the host bluetoothd on D-Bus. The USB function stays on the host."
say "A Bluetooth 4.2 controller often has one advertising slot. If Home Assistant is already advertising on it, registration fails and the log says REGISTER FAILED. The window still stays up."
(
    while [ -f /tmp/owldrop-running ]; do
        python3 "$BLE" --hashes 0000000000000000 --seconds 3600 --adapter hci0 || true
        [ -f /tmp/owldrop-running ] || break
        sleep 5
    done
) &
BLE_PID=$!

if ! ensure_identity || ! ensure_awdl_ipv6; then
    stop_stack
    exit 1
fi
say "AirDrop receiver on awdl0, saving into ${INBOX}."
python3 "$RECEIVER" --iface awdl0 --port 8771 --name "$PC_NAME" --model MacBookPro18,3 \
    --outdir "$INBOX" --keys "${OWLDROP_KEYS_DIR:-/data/omdrop}" &
SERVE_PID=$!
sleep 2
if ! kill -0 "$SERVE_PID" 2>/dev/null; then
    say "The AirDrop receiver exited."
    stop_stack
    exit 1
fi
say "The AirDrop receiver is listening on awdl0 port 8771."

BRIDGE_ARGS=(watch --inbox "$INBOX" --name "$PC_NAME")
if [ -n "$PC_HOST" ]; then
    BRIDGE_ARGS+=(--host "$PC_HOST")
fi
say "Forwarding files to owldrop.exe. Text placed in ${INBOX}/text becomes the PC clipboard."
"$BRIDGE" "${BRIDGE_ARGS[@]}" &
BRIDGE_PID=$!
sleep 1
if ! kill -0 "$BRIDGE_PID" 2>/dev/null; then
    say "owldrop-bridge exited. The PC has to be answering on TCP 47632."
    stop_stack
    exit 1
fi

say "Window is up. Stop this add-on to leave the cell and return ${IFACE} to a station."
wait "$BRIDGE_PID" "$SERVE_PID" || true
stop_stack
exit 0
