#!/bin/bash
# AirDrop on Home Assistant OS: omdrop-owl's radio helper on a Wi-Fi card the
# host kernel already drives, omdrop-plugin's receiver, delivery into
# save_folder, and an optional forwarder to howldrop.exe on a PC.
# Which card and how it runs come from owl-profiles, as on any omdrop-owl
# machine: exclusive mode (an IBSS cell, or an active monitor) on a card
# that is not Home Assistant's uplink, or shared mode on one that is.
set -uo pipefail

LIB=/usr/lib/omdrop
DISCOVERABLE=$LIB/omdrop-discoverable
BLE=$LIB/ble-airdrop-adv.py
RECEIVER=/opt/omdrop-receiver/airdrop-serve.py
BRIDGE=/usr/local/bin/howldrop-bridge
# Where the phone's files and text land: save_folder, in Home Assistant's
# share folder by default, which its Samba share shows to every PC.
INBOX=/share/howldrop
# The receiver writes here; deliver.py moves each finished item to INBOX,
# fetching the file behind a link when the link points at one.
STAGING=/data/staging
DELIVER=/deliver.py
# Who may send: the senders option, as the receiver's Contacts Only files.
SENDERS=/senders.py
SENDERS_CONFIG=${XDG_CONFIG_HOME:-${HOME:-/root}/.config}/omdrop
BSSID=00:25:00:ff:94:73
IFACE=

BLE_PID=
SERVE_PID=
BRIDGE_PID=
DELIVER_PID=
TCPFLOW_PID=
MON_IF=mon0
WATCH_PID=
RADIO=0

say() { printf '%s\n' "$*"; }
# Detail for debugging a radio, printed only with the verbose option on.
VERBOSE=false
vsay() { [ "$VERBOSE" != true ] || say "$@"; }

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

# owl-profiles as the helper reads it (load_profile): this driver's line,
# else the "*" line. Prints status, mode (shared, exclusive or
# active-monitor), mon_mac, reconnect and mt76_pm.
PROFILES=$LIB/owl-profiles
profile_of() {  # DRIVER
    local line
    line=$(awk -v d="$1" '!/^#/ && NF >= 6 && $1 == d { print; exit }' "$PROFILES" 2>/dev/null)
    [ -n "$line" ] || line=$(awk '!/^#/ && NF >= 6 && $1 == "*" { print; exit }' "$PROFILES" 2>/dev/null)
    [ -n "$line" ] || return 1
    printf '%s\n' "$line" | awk '{ print $2, $3, $4, $5, $6 }'
}

# Whether the helper runs this driver's window on 5 GHz (tx_channels): an
# active monitor on the MT7612U does; the MT7921/MT7922's, and every IBSS
# cell measured so far, hold channel 6.
uses_5ghz() {  # DRIVER MODE
    [ "$2" = active-monitor ] && [ "$1" = mt76x2u ]
}

# Interfaces with a default route: Home Assistant's own network.
uplinks() {
    { ip -4 route show default; ip -6 route show default; } 2>/dev/null \
        | awk '{ for (i = 1; i < NF; i++) if ($i == "dev") print $(i + 1) }' | sort -u
}

wifi_ifaces() {
    local d name
    for d in /sys/class/net/*; do
        [ -d "$d/phy80211" ] || continue
        name=$(basename "$d")
        case "$name" in
            awdl0|mon0|awdlibss0) continue ;;
        esac
        printf '%s\n' "$name"
    done
}

supports_active_monitor() {
    iw phy "$(phy_of "$1")" info 2>/dev/null | grep -q 'Device supports active monitor'
}

# Can IFACE carry the window? Prints "RANK BACKEND STATUS", lower ranks
# better, or "- REASON". BACKEND is ibss (exclusive mode's IBSS cell),
# active-monitor or shared. Exclusive and active-monitor take the card off
# any network, so never Home Assistant's uplink; shared mode stays
# associated, so it needs a card that is connected, which here means the
# uplink. A card whose window runs on 5 GHz, where an iPhone runs its
# transfers, ranks first.
judge_iface() {  # IFACE
    local name=$1 drv prof status mode backend up=no rank
    drv=$(driver_of "$name")
    prof=$(profile_of "$drv") || { echo "- no-profile"; return; }
    read -r status mode _ <<< "$prof"
    uplinks | grep -qx "$name" && up=yes
    case "$mode" in
        exclusive|active-monitor)
            [ "$up" = no ] || { echo "- uplink"; return; }
            backend=ibss
            if [ "$mode" = active-monitor ]; then
                supports_active_monitor "$name" || { echo "- no-active-monitor"; return; }
                backend=active-monitor
            fi
            rank=1
            uses_5ghz "$drv" "$mode" && rank=0
            ;;
        shared)
            [ "$up" = yes ] || { echo "- not-connected"; return; }
            rank=2
            backend=shared
            ;;
        *) echo "- mode"; return ;;
    esac
    if [ "$status" != tested ]; then
        [ "$ALLOW_UNTESTED" = true ] || { echo "- untested"; return; }
        rank=$((rank + 3))
    fi
    echo "$rank $backend $status"
}

why_not() {
    case "$1" in
        no-profile) echo "owl-profiles has no line for its driver" ;;
        uplink) echo "it carries Home Assistant's network, and exclusive mode would take that away" ;;
        no-active-monitor) echo "its profile needs an active monitor, and this driver offers none" ;;
        not-connected) echo "its profile is shared mode, which needs it connected to a Wi-Fi network" ;;
        untested) echo "its profile is untested; turn allow_untested on to try it" ;;
        *) echo "its profile asks for a mode omdrop-owl does not implement" ;;
    esac
}

# The best interface: prints "IFACE BACKEND STATUS", or nothing.
pick_radio() {
    local name verdict rank best='' best_rank=99
    for name in $(wifi_ifaces); do
        verdict=$(judge_iface "$name")
        rank=${verdict%% *}
        [ "$rank" = - ] && continue
        if [ "$rank" -lt "$best_rank" ]; then
            best_rank=$rank
            best="$name ${verdict#* }"
        fi
    done
    [ -z "$best" ] || printf '%s\n' "$best"
}

explain_radios() {
    local name verdict
    say "Wi-Fi interfaces here:"
    for name in $(wifi_ifaces); do
        verdict=$(judge_iface "$name")
        if [ "${verdict%% *}" = - ]; then
            say "  ${name} (driver $(driver_of "$name"), $(bus_id_of "$name")): not usable, $(why_not "${verdict#* }")."
        else
            say "  ${name} (driver $(driver_of "$name"), $(bus_id_of "$name")): usable, ${verdict#* }."
        fi
    done
}

usb_id_of() {
    local dev
    dev=$(readlink -f "/sys/class/net/$1/device" 2>/dev/null) || return 0
    printf '%s:%s' "$(cat "$dev/../idVendor" 2>/dev/null)" "$(cat "$dev/../idProduct" 2>/dev/null)"
}

bus_id_of() {
    case "$(basename "$(readlink -f "/sys/class/net/$1/device/subsystem" 2>/dev/null)" 2>/dev/null)" in
        pci) printf 'PCI %s' "$(pci_of "$1")" ;;
        usb) printf 'USB %s' "$(usb_id_of "$1")" ;;
        *) printf 'bus unknown' ;;
    esac
}

list_ifaces() {
    local d name
    for d in /sys/class/net/*; do
        [ -e "$d" ] || continue
        name=$(basename "$d")
        say "  ${name}: driver $(driver_of "$name"), $(bus_id_of "$name")"
    done
}

# host_network puts this process in the host netns. If Supervisor left the
# container on the hassio bridge, host_pid lets us enter the host netns
# before any radio command runs. The exclusive IBSS profile is unchanged.
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
    local pid
    dbus_remove_iface
    for _ in 1 2 3 4 5; do
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

# A driver can advertise IBSS without listing it in its interface
# combinations (iwlmvm does). mac80211 then allows the bring-up only when this
# netdev is the only one running on the phy. A live P2P device or a second vif makes `ip link set up`
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
        if ! iw phy "$phy" interface add awdlibss0 type ibss 2>/tmp/howldrop-iw.err; then
            [ -s /tmp/howldrop-iw.err ] && cat /tmp/howldrop-iw.err
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
        ip link set awdlibss0 address "$(cat "/sys/class/net/$IFACE/address" 2>/dev/null)" 2>/tmp/howldrop-addr.err || true
        if [ -s /tmp/howldrop-addr.err ]; then
            cat /tmp/howldrop-addr.err
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
        24[1-7][27]) printf '%s\n' "$1 MHz (channel $(( ($1 - 2407) / 5 )))" ;;
        5[0-9][0-9][05]) printf '%s\n' "$1 MHz (channel $(( ($1 - 5000) / 5 )))" ;;
        *) printf '%s\n' "$1 MHz" ;;
    esac
}

# The channels option as frequencies, in its order.
want_freqs() {
    local c out=
    for c in $1; do
        case "$c" in
            36) out="$out 5180" ;;
            44) out="$out 5220" ;;
            149) out="$out 5745" ;;
            6) out="$out 2437" ;;
            *) say "Ignoring channel ${c}: use 6, 36, 44 or 149." >&2 ;;
        esac
    done
    printf '%s\n' "${out# }"
}

wants_5ghz() {
    case " $WANT_FREQS " in
        *" 5220 "*|*" 5745 "*) return 0 ;;
    esac
    return 1
}

# The wanted frequencies, in the channels option's order. A frequency the
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
    for c in $WANT_FREQS; do
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
    local dev=$1 freq=$2 err got
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
    for _ in $(seq 100); do
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
            vsay "${label} is usable ${when}."
        else
            say "${label} is disabled or No IR ${when}."
        fi
    done
}

# Hear a 5 GHz network. Some cards (Intel's) keep those channels
# receive-only until one is heard. The join arguments are not involved.
scan_for_5ghz() {
    local err rc
    if ! iface_is_up "$1"; then
        ip link set "$1" up 2>/dev/null || true
    fi
    say "Scanning once on ${1} so the card can hear a 5 GHz network."
    if command -v timeout >/dev/null 2>&1; then
        err=$(timeout 25 iw dev "$1" scan 2>&1 >/dev/null)
        rc=$?
    else
        err=$(iw dev "$1" scan 2>&1 >/dev/null)
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
    vsay "Regulatory domain: ${country:-unread}."
    if iw reg get 2>/dev/null | grep -q 'self-managed'; then
        vsay "This radio keeps its own regulatory domain."
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
    if wants_5ghz && ! five_ghz_open "$allowed"; then
        say "5 GHz is still closed after the interface was brought up. Scanning on ${dev}."
        scan_for_5ghz "$dev" || true
        allowed=$(allowed_airdrop_freqs "$phy")
        report_5ghz "$allowed" "after the cell scan"
    fi
    if [ -z "$allowed" ]; then
        say "No wanted channel is usable, or the channel list could not be read. Trying channel 6."
        allowed=2437
    fi
    for freq in $WANT_FREQS; do
        printf '%s\n' "$allowed" | grep -qx "$freq" || say "$(freq_name "$freq") will not be tried."
    done
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
    base=${HOWLDROP_KEYS_DIR:-/data/omdrop}
    mkdir -p "$base/keys" || {
        say "Could not create ${base}/keys."
        return 1
    }
    chmod 700 "$base/keys" 2>/dev/null || true
    cert=$base/keys/certificate.self-signed.pem
    key=$base/keys/key.self-signed.pem
    if [ -f "$cert" ] && [ -f "$key" ]; then
        vsay "Using the self-signed AirDrop identity already in the add-on data."
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
        vsay "awdl0 IPv6 address is ${addr} and it is ready."
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
    vsay "awdl0 IPv6 address is ${addr} and it is ready."
    return 0
}

# OWL -F sends each unicast frame once, without retries, and the phone is on
# channel 6 for one slot in sixteen (its sequence on 2026-10-12: 44,44,149,
# 0,0,0,0,44,6,44,149,44,0,0,0,44), so segments are lost and only TCP
# resends them: after 200 ms at least, doubling each time. On 2026-10-12
# that stretched /Discover's TLS handshake to 1-4 s, the phone hung up, and
# no tile appeared. A 20 ms floor on awdl0's route alone resends within the
# phone's next visit. It is a second fe80::/64 route on awdl0, preferred
# by its lower metric: nothing is deleted or replaced, so the kernel's own
# route and every other interface's (the Ethernet uplink's) stay as they are.
# 0.2.1's delete-and-add was refused and hid why, so errors are printed.
fast_awdl_retransmit() {
    local err
    # No quickack: it acknowledges every segment, and a 30 MB upload on 149
    # drew one injected ACK per segment (21,926 for 21,921), each taking the
    # air the phone sends on (2026-10-14). Delayed ACKs halve that.
    if err=$(ip -6 route add fe80::/64 dev awdl0 metric 128 rto_min 20ms 2>&1); then
        vsay "TCP on awdl0 resends after 20 ms instead of 200 ms."
        [ "$VERBOSE" != true ] || ip -6 route show dev awdl0 2>/dev/null | sed 's/^/  awdl0 route: /'
        return 0
    fi
    say "ip -6 route add fe80::/64 dev awdl0 metric 128 rto_min 20ms: ${err}"
    say "TCP on awdl0 keeps the 200 ms retransmit floor. Routes on awdl0:"
    ip -6 route show dev awdl0 2>&1 | sed 's/^/  /'
}

# The host kernel's last radio lines: a firmware crash (Microcode SW error)
# or a failed restart is what takes mon0 away mid-window. dmesg needs
# CAP_SYSLOG or SYS_ADMIN; the host's mount namespace is the fallback.
# What mon0 actually receives over SECS, by frequency, and how much of it
# carries AWDL's BSSID. iw's channel report is not proof: on the MT7612U an
# active monitor alone "accepted" channel changes and kept receiving on
# channel 1 (pull request 13), and on Home Assistant OS iw reported no channel
# at all while the card heard nothing on 44 (2026-10-14).
frames_heard() {  # SECS
    local secs=$1 cap freqs awdl total own senders
    command -v tcpdump >/dev/null 2>&1 || return 0
    cap=$(timeout "$secs" tcpdump -i "$MON_IF" -n -e -l 2>/dev/null)
    total=$(printf '%s\n' "$cap" | grep -c 'MHz')
    awdl=$(printf '%s\n' "$cap" | grep -ci "$BSSID")
    freqs=$(printf '%s\n' "$cap" | grep -oE '[0-9]{4} MHz' | sort | uniq -c | sort -rn | head -n 4 \
        | awk '{ printf "%s%s %s x%s", (NR > 1 ? ", " : ""), $2, $3, $1 }')
    say "Frames on ${MON_IF} in ${secs} s: ${total:-0} (${freqs:-none}); ${awdl:-0} with AWDL's BSSID."
    # Whose: a monitor interface also captures what it sends, and OWL sends
    # all the time, so AWDL frames alone do not prove the phone is heard.
    own=$(cat "/sys/class/net/${MON_IF}/address" 2>/dev/null)
    senders=$(printf '%s\n' "$cap" | grep -i "$BSSID" | grep -oiE 'SA:[0-9a-f:]{17}' | cut -c4- \
        | tr 'A-F' 'a-f' | sort | uniq -c | sort -rn | head -n 5 \
        | awk -v own="$own" '{ printf "%s%s x%s%s", (NR > 1 ? ", " : ""), $2, $1, ($2 == own ? " (us)" : "") }')
    say "AWDL senders: ${senders:-none with a source address}."
}

# The AirDrop TCP conversation on awdl0 (what OWL hands the kernel and what
# the kernel sends back), every 2 s with traffic: segments and new bytes in
# from the phone, its resends, and our ACKs out. Uploads stalled with
# "nothing arrived for 30 s" after 1.1 MB and after 0.46 MB (2026-10-14):
# this says whether the phone went quiet or kept resending what our lost ACKs
# never confirmed. Sequence numbers are tcpdump's, relative per connection.
tcp_flow() {
    command -v tcpdump >/dev/null 2>&1 || return 0
    tcpdump -i awdl0 -n -S -tt -l 'tcp port 8771' 2>/dev/null | awk '
        function flush() {
            if (inp || outp)
                printf "TCP flow t+%ds: in %d seg, +%d new bytes, %d resent | out %d, acked +%d bytes%s\n",
                    bucket * 2 - t0, inp, newb, resent, outp, ackb, (syn ? ", SYN" : "") (fin ? ", FIN" : "")
            inp = outp = newb = resent = ackb = syn = fin = 0
        }
        {
            ts = int($1)
            if (!started) { t0 = int(ts / 2) * 2; started = 1 }
            b = int(ts / 2)
            if (b != bucket) { flush(); bucket = b }
            src = $3; dst = $5; sub(/:$/, "", dst)
            out = (src ~ /\.8771$/)
            if ($0 ~ /Flags \[S/) syn = 1
            if ($0 ~ /Flags \[[^]]*F/) fin = 1
            if (out) {
                outp++
                if (match($0, /ack [0-9]+/)) {
                    ack = substr($0, RSTART + 4, RLENGTH - 4) + 0
                    if (lastack[dst] && ack > lastack[dst]) ackb += ack - lastack[dst]
                    lastack[dst] = ack
                }
            } else {
                inp++
                conn = src
                if (match($0, /seq [0-9]+:[0-9]+/)) {
                    split(substr($0, RSTART + 4, RLENGTH - 4), r, ":")
                    a = r[1] + 0; e = r[2] + 0
                    if (e <= maxseq[conn]) resent++
                    else { newb += e - (a > maxseq[conn] ? a : maxseq[conn]); maxseq[conn] = e }
                }
            }
            fflush()
        }
        END { flush() }'
}

kernel_radio_log() {
    local pattern='iwlwifi|iwlmvm|ieee80211|mac80211|cfg80211|wlp|mon0|awdl'
    local out
    out=$(dmesg 2>/dev/null | grep -iE "$pattern" | tail -n 30)
    [ -n "$out" ] || out=$(nsenter -t 1 -m -- dmesg 2>/dev/null | grep -iE "$pattern" | tail -n 30)
    if [ -n "$out" ]; then
        printf '%s\n' "$out" | sed 's/^/kernel: /'
    else
        say "The host kernel log is not readable from this container."
    fi
}

# The receiver's connections from /proc/net/tcp6, one per line: state (hex),
# peer address, peer port, tx:rx queue, retransmits. Local port 8771 is 2243
# in hex; the listening socket (0A) is left out. Addresses there are four
# 32-bit words in host (little-endian) order. mawk (Debian's awk) has no
# strtonum, hence hex().
tcp6_8771() {
    awk '
        function hex(s,   i, n) { n = 0; s = toupper(s)
            for (i = 1; i <= length(s); i++) n = n * 16 + index("0123456789ABCDEF", substr(s, i, 1)) - 1
            return n }
        function ip6(h,   i, w, out) { out = ""
            for (i = 0; i < 4; i++) { w = substr(h, i * 8 + 1, 8)
                out = out substr(w, 7, 2) substr(w, 5, 2) ":" substr(w, 3, 2) substr(w, 1, 2) (i < 3 ? ":" : "") }
            return tolower(out) }
        $2 ~ /:2243$/ && $4 != "0A" {
            split($3, rem, ":")
            print $4, ip6(rem[1]), hex(rem[2]), $5, hex($7) }' /proc/net/tcp6 2>/dev/null
}

# Our SYN-ACK to an address on awdl0 leaves only once the kernel knows that
# address's MAC. OWL enters each AWDL peer's EUI-64 link-local as a permanent
# neighbour; a phone connecting from any other address needs neighbour
# discovery, a multicast exchange that seldom meets the phone on channel 6,
# and until it does the SYN-ACK waits and the connection stays half-open.
# With one AWDL peer, that peer's MAC is the only one the address can have.
NEIGH_FIXED=" "
neighbor_for() {  # IPV6 (as tcp6_8771 prints it)
    local addr=$1 entry state macs mac
    case "$NEIGH_FIXED" in *" $addr "*) return 0 ;; esac
    entry=$(ip -6 neigh show "$addr" dev awdl0 2>/dev/null)
    state=$(printf '%s\n' "$entry" | awk 'NF { print $NF; exit }')
    case "$state" in
        PERMANENT|REACHABLE|STALE|DELAY|PROBE)
            vsay "Half-open connection from ${addr}: its neighbour entry is ${entry}. The SYN-ACK has a MAC to go to."
            NEIGH_FIXED="$NEIGH_FIXED$addr "
            return 0 ;;
    esac
    macs=$(ip -6 neigh show dev awdl0 nud permanent 2>/dev/null \
        | awk '{ for (i = 1; i < NF; i++) if ($i == "lladdr") print $(i + 1) }' | sort -u)
    say "Half-open connection from ${addr}, which has no usable neighbour entry (${state:-none}). awdl0 neighbours:"
    ip -6 neigh show dev awdl0 2>/dev/null | sed 's/^/  /'
    if [ "$(printf '%s\n' "$macs" | grep -c .)" -ne 1 ]; then
        say "OWL has not entered exactly one peer MAC, so ${addr} is left to neighbour discovery."
        NEIGH_FIXED="$NEIGH_FIXED$addr "
        return 0
    fi
    mac=$macs
    if ip -6 neigh replace "$addr" lladdr "$mac" dev awdl0 nud permanent 2>/dev/null; then
        say "Entered ${addr} as ${mac}, the phone's AWDL MAC, so the SYN-ACK can leave."
    else
        say "Could not enter ${addr} as ${mac}."
    fi
    NEIGH_FIXED="$NEIGH_FIXED$addr "
}

# OWL's and the announcer's logs stay in /run/omdrop-owl inside this
# container, so bring what matters into the add-on log: OWL's peer events
# and errors every 5 s, TCP state of AirDrop connections every second, and a
# summary every 30 s. No "add peer" means the radio never heard the iPhone.
# A /Discover answered 200 with no tile means the answer did not reach the
# phone in time; rtt, rto and retrans say whether that is loss on the air.
# The helper's logs grow for as long as the window is open (OWL runs with
# -v, a few lines a second while a phone is near). Past 20 MB each keeps its
# last 2 MB. Every writer appends, so the next line lands after what stays.
# The add-on's own output goes to Home Assistant's journal, which rotates.
trim_logs() {
    local f size
    for f in /run/omdrop-owl/owl.log /run/omdrop-owl/announce.log /run/omdrop-owl/mdns.log /run/omdrop-owl/log; do
        size=$(stat -c %s "$f" 2>/dev/null || echo 0)
        [ "$size" -gt 20971520 ] || continue
        tail -c 2097152 "$f" > "$f.keep" 2>/dev/null && cat "$f.keep" > "$f"
        rm -f "$f.keep"
        # watch_radio's read positions (bash scoping: it is the caller), so
        # what stays is not printed again.
        case "$f" in
            */owl.log) seen=$(wc -l < "$f") ;;
            */mdns.log) mseen=$(wc -l < "$f") ;;
        esac
        vsay "Trimmed ${f} from $((size / 1048576)) MB to its last 2 MB."
    done
}

watch_radio() {
    local owl=/run/omdrop-owl/owl.log ann=/run/omdrop-owl/announce.log mdns=/run/omdrop-owl/mdns.log
    local seen=0 total tick=0 peers tcp last_tcp='' fault='' mseen=0 mtotal
    vsay "OWL's first lines:"
    [ "$VERBOSE" != true ] || head -n 15 "$owl" 2>/dev/null | sed 's/^/OWL: /'
    seen=$(wc -l < "$owl" 2>/dev/null || echo 0)
    vsay "Interfaces on this phy:"
    [ "$VERBOSE" != true ] || iw dev 2>/dev/null | grep -E 'Interface|type|channel|addr' | sed 's/^[[:space:]]*/  /'
    # Without verbose, OWL's lines are peers coming and going and failures.
    local pattern='add peer|remove peer|error|fail|fatal|refus'
    [ "$VERBOSE" != true ] || pattern='peer|error|fail|warn|fatal|refus'
    while [ -f /tmp/howldrop-running ]; do
        sleep 1
        tick=$((tick + 1))
        # Discover connections last a second or few, so this is per tick.
        # Printed only when it changes: a socket left in FIN_WAIT would
        # otherwise repeat for minutes.
        # A socket is one line, plus an indented detail line when the kernel
        # has socket diagnostics (0.2.5 printed nothing: it assumed one).
        tcp=$(ss -tinH '( sport = :8771 )' 2>/dev/null \
            | awk '/^[^ \t]/ { if (rec) print rec; rec = "TCP " $1 " " $5 ":"; next }
                   { for (i = 1; i <= NF; i++)
                         if ($i ~ /^(rtt|rto|retrans|unacked|lost|bytes_sent|bytes_retrans|bytes_acked):/) rec = rec " " $i }
                   END { if (rec) print rec }')
        # Without ss, the kernel's table still has state, queues and the
        # retransmit count (local port 8771 is 2243 in hex).
        # mawk (Debian's awk) has no strtonum, hence hex().
        [ -n "$tcp" ] || tcp=$(tcp6_8771 | awk '{
                printf "TCP state %s, peer [%s]:%d, tx:rx queue %s, retransmits %d\n", $1, $2, $3, $4, $5 }')
        if [ "$VERBOSE" = true ] && [ -n "$tcp" ] && [ "$tcp" != "$last_tcp" ]; then
            printf '%s %s\n' "$(date +%H:%M:%S)" "$tcp"
        fi
        last_tcp=$tcp
        # A connection stuck half-open (03, SYN_RECV) while others from the
        # phone complete: 0.2.9's /Ask SYN-ACK was retried for 30 s while a
        # /Discover beside it finished in 50 ms.
        # Fed by process substitution, not a pipe, so NEIGH_FIXED survives.
        while read -r peer6; do
            neighbor_for "$peer6"
        done < <(tcp6_8771 | awk '$1 == "03" { print $2 }' | sort -u)
        [ $((tick % 5)) -eq 0 ] || continue
        # The responder's lines: "responding on", then one per answered query.
        # A phone with the share sheet open queries for _airdrop._tcp; no
        # "answered" line means no query reached us.
        mtotal=$(wc -l < "$mdns" 2>/dev/null || echo 0)
        [ "$mtotal" -lt "$mseen" ] && mseen=0
        if [ "$mtotal" -gt "$mseen" ]; then
            [ "$VERBOSE" != true ] || sed -n "$((mseen + 1)),${mtotal}p" "$mdns" | tail -n 5 | sed 's/^/mDNS: /'
            mseen=$mtotal
        fi
        total=$(wc -l < "$owl" 2>/dev/null || echo 0)
        [ "$total" -lt "$seen" ] && seen=0
        if [ "$total" -gt "$seen" ]; then
            # A run of one message (an Intel 8260 on channel 44 logged
            # "unhandled frame" dozens of times a second) is one line and a
            # count, timestamp aside.
            sed -n "$((seen + 1)),${total}p" "$owl" \
                | grep -iE "$pattern" \
                | awk '{ t = $1; $1 = ""; msg = $0 }
                       msg == last { n++; next }
                       { if (n) print "OWL: (" n " more of that)"; print "OWL: " t msg; last = msg; n = 0 }
                       END { if (n) print "OWL: (" n " more of that)" }' \
                | head -n 20
            # Injection failing with "No such device" means mon0 is gone,
            # which nothing here deletes mid-window: the driver did.
            if [ -z "$fault" ] && sed -n "$((seen + 1)),${total}p" "$owl" | grep -q 'unable to inject'; then
                fault=1
                say "OWL cannot send any more. Interfaces now:"
                iw dev 2>/dev/null | grep -E 'Interface|type|channel' | sed 's/^[[:space:]]*/  /'
                kernel_radio_log
                # An Intel 8260's firmware stopped answering on channel 44 (-5
                # on every command) and iwlwifi removed and re-probed the
                # card, taking every interface with it (2026-10-12). Nothing
                # recovers a window from that, so end it plainly.
                if [ ! -d /sys/class/net/mon0 ]; then
                    say "The card was reset under this window (mon0 is gone): its firmware failed, often on a channel it cannot hold. Try the channels option set to 6. Stopping."
                    kill -TERM $$
                fi
            fi
            seen=$total
        fi
        [ $((tick % 600)) -eq 0 ] && trim_logs
        if [ "$VERBOSE" = true ] && [ $((tick % 30)) -eq 0 ]; then
            peers=$(grep -c 'add peer' "$owl" 2>/dev/null)
            say "Radio check: ${peers:-0} peer adds in OWL's log so far ($total lines). Helper: $("$DISCOVERABLE" status 2>/dev/null)."
            frames_heard 2
            tail -n 2 "$ann" 2>/dev/null | sed 's/^/Announce: /'
        fi
    done
}

stop_stack() {
    rm -f /tmp/howldrop-running
    if [ -n "$WATCH_PID" ]; then kill "$WATCH_PID" 2>/dev/null || true; fi
    if [ -n "$BRIDGE_PID" ]; then kill "$BRIDGE_PID" 2>/dev/null || true; fi
    pkill -f "$BRIDGE watch" 2>/dev/null || true
    if [ -n "$DELIVER_PID" ]; then kill "$DELIVER_PID" 2>/dev/null || true; fi
    if [ -n "$TCPFLOW_PID" ]; then kill "$TCPFLOW_PID" 2>/dev/null || true; fi
    pkill -f "tcpdump -i awdl0" 2>/dev/null || true
    pkill -f "tcpdump -i mon0 -s 256" 2>/dev/null || true
    pkill -f "$DELIVER" 2>/dev/null || true
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

say "HowlDrop add-on"
say "It uses a Wi-Fi card the host kernel already drives and loads no driver or firmware of its own."
say "Files and text from the phone are kept in save_folder (default /share/howldrop). howldrop.exe, when a PC runs it, gets them too."
say ""

if ! dbus_host; then
    say "The host D-Bus socket is not in this container. Turn protection mode off and install the add-on with host D-Bus enabled."
    exit 1
fi

case "$(option verbose)" in
    True|true|1|yes) VERBOSE=true ;;
esac
ALLOW_UNTESTED=false
case "$(option allow_untested)" in
    True|true|1|yes) ALLOW_UNTESTED=true ;;
esac

# The interface option names the card; empty or auto picks the best one here.
WANT_IFACE=$(option interface)
IFACE=
BACKEND=
CARD_STATUS=
case "${WANT_IFACE:-auto}" in
    auto)
        read -r IFACE BACKEND CARD_STATUS <<< "$(pick_radio)"
        ;;
    *)
        if [ ! -d "/sys/class/net/${WANT_IFACE}/phy80211" ]; then
            say "The interface option is ${WANT_IFACE}, and no Wi-Fi interface here has that name."
            explain_radios
            exit 1
        fi
        VERDICT=$(judge_iface "$WANT_IFACE")
        if [ "${VERDICT%% *}" = - ]; then
            say "${WANT_IFACE} cannot carry the AirDrop window: $(why_not "${VERDICT#* }")."
            exit 1
        fi
        IFACE=$WANT_IFACE
        read -r _ BACKEND CARD_STATUS <<< "$VERDICT"
        ;;
esac
if [ -z "$IFACE" ]; then
    say "No Wi-Fi interface here can carry the AirDrop window."
    explain_radios
    self_ns=$(readlink /proc/self/ns/net 2>/dev/null || true)
    host_ns=$(readlink /proc/1/ns/net 2>/dev/null || true)
    if [ -n "$self_ns" ] && [ -n "$host_ns" ] && [ "$self_ns" != "$host_ns" ]; then
        say "This container is still not in the host network namespace. Turn protection mode off so host_network, host_pid, and SYS_ADMIN apply, then update this add-on so Supervisor creates a new container."
    elif ! compgen -G '/sys/class/net/*/phy80211' >/dev/null; then
        say "No Wi-Fi card is visible. Plug one in (a USB adapter needs no passthrough: the host's driver makes the interface), or check that Home Assistant OS has a driver for it."
    fi
    exit 1
fi

DRV=$(driver_of "$IFACE")
FW=$(firmware_of "$IFACE")
say "Interface ${IFACE}: driver ${DRV}, $(bus_id_of "$IFACE"), firmware ${FW:-unread}."
case "$BACKEND" in
    active-monitor)
        say "Profile: exclusive mode with an active monitor that acknowledges the phone itself; ${IFACE} is a passive monitor for the window. It can follow the phone onto 5 GHz."
        ;;
    ibss)
        say "Profile: exclusive mode, an IBSS cell on AWDL's BSSID ${BSSID}, HT20, fixed frequency. ${IFACE} leaves any network for the window."
        ;;
    shared)
        say "Profile: shared mode. ${IFACE} stays on its Wi-Fi network and AWDL runs beside it on that network's channel, which should be 6, 44 or 149. Not yet tried on Home Assistant OS."
        ;;
esac
mkdir -p /etc/omdrop
if [ "$CARD_STATUS" = tested ]; then
    rm -f /etc/omdrop/allow-untested
else
    say "This card's profile is untested; allow_untested is on. Reports of how it goes help: github.com/t4t5/omdrop-owl, README, \"Adding your card\"."
    echo 1 > /etc/omdrop/allow-untested
fi

PC_HOST=$(option pc_host)
PC_NAME=$(option pc_name)
CHANNELS=$(option channels)
# Empty or auto: a card whose window runs on 5 GHz (uses_5ghz) starts on 149,
# where an iPhone runs its transfers (every fast one here ran there), then 44,
# 36 and 6; the rest hold 6. Shared mode runs on its network's channel.
# No survey for the phone first: it can't tell one Apple device from another.
# One heard a Mac on 44 and started there, where the iPhone (6 and 149) never
# came, and the tile never appeared; another missed the iPhone on 149
# altogether (2026-10-14). It also took 20 s of every start.
case "${CHANNELS// /}" in
    ""|auto)
        if uses_5ghz "$DRV" "$BACKEND"; then
            CHANNELS="149 44 36 6"
        else
            CHANNELS=6
        fi
        ;;
esac
WANT_FREQS=$(want_freqs "$CHANNELS")
if [ -z "$WANT_FREQS" ]; then
    say "The channels option has no channel this add-on can use. Using channel 6."
    WANT_FREQS=2437
fi
if [ "$BACKEND" != shared ]; then
    say "Channels to try, in order: $(for f in $WANT_FREQS; do freq_name "$f"; done | paste -sd, - | sed 's/,/, /g')."
fi
SAVE_FOLDER=$(option save_folder)
case "${SAVE_FOLDER:-/share/howldrop}" in
    /*) INBOX=${SAVE_FOLDER:-/share/howldrop} ;;
    *) say "save_folder must be an absolute path such as /share/howldrop; using /share/howldrop." ;;
esac
FETCH=$(option fetch_links)
case "$FETCH" in
    False|false|0|no) FETCH=false ;;
    *) FETCH=true ;;
esac
FETCH_MB=$(option fetch_max_mb)
case "$FETCH_MB" in
    ""|*[!0-9]*) FETCH_MB=100 ;;
esac
# Files older than this many days are deleted from save_folder; 0 keeps them.
KEEP_DAYS=$(option delete_after_days)
case "$KEEP_DAYS" in
    ""|*[!0-9]*) KEEP_DAYS=30 ;;
esac
CAPTURE=$(option debug_capture)
case "$CAPTURE" in
    True|true|1|yes) CAPTURE=true ;;
    *) CAPTURE=false ;;
esac
# Off unless asked for: the share folder is the way to use this, and a PC
# running howldrop.exe is an extra.
FORWARD=$(option forward_to_pc)
case "$FORWARD" in
    True|true|1|yes) FORWARD=true ;;
    *) FORWARD=false ;;
esac
# The sheet name: pc_name (HowlDrop by default). Left empty with
# forward_to_pc on, it is the name a running howldrop.exe announces, so the
# sheet shows that PC; otherwise HowlDrop.
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
# The Mac model we report (ReceiverModelName). An iPhone draws a plain contact
# silhouette under Other Devices whatever it is: Macmini9,1 looked the same as
# MacBookPro18,3 (iOS, 2026-10-14). A contact photo needs an Apple-issued
# identity, which this add-on doesn't have.
MODEL=MacBookPro18,3
say "The iPhone share sheet will show: ${PC_NAME}"

mkdir -p "$INBOX/text" "$STAGING/text" /data/omdrop/keys /run/omdrop-owl || {
    say "Could not create ${INBOX}."
    exit 1
}
printf '%s\n' "$IFACE" > /etc/omdrop/infra-iface

TYPE=$(iface_type "$IFACE")
if [ "$BACKEND" = ibss ] && type_is_ibss "$TYPE"; then
    say "${IFACE} is still an IBSS cell from an earlier window. Switching it back to a station before the profile runs."
    ip link set "$IFACE" down || true
    iw dev "$IFACE" set type managed || true
    ip link set "$IFACE" up || true
fi
iw dev mon0 del 2>/dev/null || true

# Exclusive mode: the card leaves NetworkManager and wpa_supplicant for the
# window. Shared mode keeps both; the helper injects beside the association.
if [ "$BACKEND" != shared ]; then
    if wants_5ghz; then
        allow_5ghz "$IFACE"
    fi
    vsay "Marking ${IFACE} unmanaged so NetworkManager leaves it alone for the window."
    nmcli device disconnect "$IFACE" >/dev/null 2>&1 || true
    nmcli device set "$IFACE" managed no || true
    vsay "A NetworkManager restart clears that runtime flag. The flag is set again the next time this add-on starts. This container does not write the host's /etc."
    vsay "Stopping host wpa_supplicant from holding ${IFACE}."
    release_supplicant
fi

rm -f /etc/omdrop/exclusive-channels /etc/omdrop/keep-cell /etc/omdrop/tx-rate /etc/omdrop/aggregation
# aggregation: accept the phone's Block Ack session requests,
# so it may send its frames in aggregated bursts (A-MPDU) rather than one by
# one. An iPhone asked 61 times in a 30 MB upload and, unanswered, sent all
# 21,941 frames singly (2026-10-14). Whether the card acknowledges a burst
# with a Block Ack is up to its hardware: if it can't, transfers fail.
# auto: on for the MT7612U, where it made a 30 MB upload 3.3x faster (47 s
# to 14 s, 2026-10-14), off for cards it hasn't been tried on.
AGGREGATION=$(option aggregation)
case "$AGGREGATION" in
    on|True|true|1|yes) AGGREGATION=on ;;
    off|False|false|0|no) AGGREGATION=off ;;
    *) if [ "$DRV" = mt76x2u ] && [ "$BACKEND" = active-monitor ]; then AGGREGATION=on; else AGGREGATION=off; fi ;;
esac
if [ "$AGGREGATION" = on ]; then
    echo 1 > /etc/omdrop/aggregation
    say "Aggregation is on: the phone sends in bursts (Block Ack sessions, frames reordered for TCP)."
else
    vsay "Aggregation is off: the phone sends one frame at a time."
fi
if [ "$BACKEND" = ibss ]; then
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
    # The helper keeps this cell instead of taking it down to join again
    # (ENETDOWN), and pins OWL to the channel it is on.
    case "$JOINED_FREQ" in
        5220) echo 44 > /etc/omdrop/exclusive-channels ;;
        5745) echo 149 > /etc/omdrop/exclusive-channels ;;
        2437) echo 6 > /etc/omdrop/exclusive-channels ;;
    esac
    echo 1 > /etc/omdrop/keep-cell
elif [ "$BACKEND" = active-monitor ]; then
    # Our data frames (TCP ACKs) at 24 Mbit/s rather than OWL's 12 where the
    # window runs on 5 GHz: half the airtime each. A third of the phone's
    # frames to an MT7612U on 149 were retries whose first try it never
    # received, and a radio that is sending is not receiving (2026-10-14).
    if uses_5ghz "$DRV" "$BACKEND"; then
        echo 24 > /etc/omdrop/tx-rate
    fi
    # The helper makes the active monitor itself and tunes it to the first of
    # these the card may transmit on (exclusive_monitor_up).
    for f in $WANT_FREQS; do
        case "$f" in
            5[0-9][0-9][05]) printf '%s ' $(( (f - 5000) / 5 )) ;;
            *) printf '%s ' $(( (f - 2407) / 5 )) ;;
        esac
    done > /etc/omdrop/exclusive-channels
    echo >> /etc/omdrop/exclusive-channels
    vsay "The helper will tune the active monitor to the first of: $(cat /etc/omdrop/exclusive-channels)that the card may transmit on."
fi
say "Starting the AirDrop window on ${IFACE} (until this add-on stops)."
if ! "$DISCOVERABLE" start 0; then
    say "The AirDrop window did not finish starting (${BACKEND} on ${IFACE})."
    stop_stack
    exit 1
fi
RADIO=1

CHAN=$(awk '/ on ch/ { chan=$0 } END { print chan }' /run/omdrop-owl/log 2>/dev/null || true)
vsay "Radio log: ${CHAN:-the helper wrote no channel line}"
if [ "$VERBOSE" != true ] || [ "$BACKEND" = shared ]; then
    :
elif [ "$BACKEND" = active-monitor ]; then
    say "Active monitor:"
    iw dev mon0 info 2>/dev/null | grep -E 'type|channel|addr' | sed 's/^[[:space:]]*/  /'
elif iw dev awdlibss0 link 2>/dev/null | grep -q 'Joined IBSS'; then
    iw dev awdlibss0 link
elif iw dev "$IFACE" link 2>/dev/null | grep -q 'Joined IBSS'; then
    say "The station itself is the cell."
    iw dev "$IFACE" link
else
    say "The helper returned, and no interface is reporting Joined IBSS yet."
fi
nmcli device set awdl0 managed no 2>/dev/null || true

touch /tmp/howldrop-running
vsay "Bluetooth wake: company id 0x004C, type 0x05, through the host bluetoothd on D-Bus."
vsay "A Bluetooth 4.2 controller often has one advertising slot. If Home Assistant is already advertising on it, registration fails and the log says REGISTER FAILED. The window still stays up."
(
    while [ -f /tmp/howldrop-running ]; do
        if [ "$VERBOSE" = true ]; then
            python3 "$BLE" --hashes 0000000000000000 --seconds 3600 --adapter hci0 || true
        else
            python3 "$BLE" --hashes 0000000000000000 --seconds 3600 --adapter hci0 2>&1 \
                | grep --line-buffered -iE 'fail|error|refus' | sed -u 's/^/Bluetooth: /' || true
        fi
        [ -f /tmp/howldrop-running ] || break
        sleep 5
    done
) &
BLE_PID=$!

if ! ensure_identity || ! ensure_awdl_ipv6; then
    stop_stack
    exit 1
fi
fast_awdl_retransmit
# Delivery first, so an item the receiver saves is moved as soon as it is
# whole. Anything left in staging by an earlier run goes too.
DELIVER_ARGS=(--staging "$STAGING" --dest "$INBOX" --max-mb "$FETCH_MB" --keep-days "$KEEP_DAYS")
[ "$FETCH" = true ] || DELIVER_ARGS+=(--no-fetch)
python3 "$DELIVER" "${DELIVER_ARGS[@]}" 2>&1 | sed -u 's/^/Deliver: /' &
DELIVER_PID=$!
# Before the receiver starts: it reads visibility once, when it builds its
# TLS context. A failure here stops the add-on rather than open it to everyone.
if ! python3 "$SENDERS" --config "$SENDERS_CONFIG"; then
    say "Could not apply the senders option."
    stop_stack
    exit 1
fi
vsay "AirDrop receiver on awdl0, saving into ${STAGING}; finished items move to ${INBOX}."
# Without verbose the receiver logs from INFO up: each connection, Ask and
# upload, not every request's headers (patch_receiver.py).
RECEIVER_LOG=INFO
[ "$VERBOSE" != true ] || RECEIVER_LOG=DEBUG
# debug_capture also keeps each sender's TLS client certificate (the
# receiver's OMDROP_PEERCERT), in save_folder/debug/peercerts: a Contacts Only
# refusal for "the validation record does not belong to the presented
# certificate" turns on what that certificate is.
if [ "$CAPTURE" = true ]; then
    export OMDROP_PEERCERT="$INBOX/debug/peercerts"
fi
HOWLDROP_LOG_LEVEL=$RECEIVER_LOG PYTHONWARNINGS=ignore::DeprecationWarning \
python3 "$RECEIVER" --iface awdl0 --port 8771 --name "$PC_NAME" --model "$MODEL" \
    --outdir "$STAGING" --keys "${HOWLDROP_KEYS_DIR:-/data/omdrop}" &
SERVE_PID=$!
sleep 2
if ! kill -0 "$SERVE_PID" 2>/dev/null; then
    say "The AirDrop receiver exited."
    stop_stack
    exit 1
fi
say "The AirDrop receiver is listening on awdl0 port 8771."

# The PC is optional. The forwarder leaves every file in save_folder
# (--keep), sends only what arrives from now on, and keeps looking for a PC
# when none answers or the one it had goes away (--wait). It filters by name
# only when that name came from a PC's own beacon. Restarted if it exits; the
# AirDrop window never waits on it.
if [ "$FORWARD" = true ]; then
    BRIDGE_ARGS=(watch --inbox "$INBOX" --keep --wait --timeout 10)
    if [ -n "$PC_BEACON" ]; then
        BRIDGE_ARGS+=(--name "$PC_BEACON")
    fi
    if [ -n "$PC_HOST" ]; then
        BRIDGE_ARGS+=(--host "$PC_HOST")
    fi
    say "Forwarding new files to howldrop.exe whenever a PC runs it; they stay in ${INBOX} either way. Text goes to that PC's clipboard."
    (
        while [ -f /tmp/howldrop-running ]; do
            "$BRIDGE" "${BRIDGE_ARGS[@]}" 2>&1 | sed -u 's/^/Forward: /'
            [ -f /tmp/howldrop-running ] || break
            sleep 5
        done
    ) &
    BRIDGE_PID=$!
else
    vsay "forward_to_pc is off. Files and text stay in ${INBOX}."
fi

say "Window is up on ${IFACE}. Stopping this add-on gives the card back."
# debug_capture: what the radio and the AirDrop TCP carried, as pcap files in
# save_folder/debug. mon0 keeps each frame's first 256 bytes (radiotap,
# 802.11, LLC, IPv6, TCP headers), so a stalled upload can be followed frame
# by frame: two photo uploads stopped 3 KB from the end with every byte ACKed
# and the phone still sending (2026-10-14). A ring of 20 MB files, the oldest
# overwritten, for the whole window. -Z root: Debian's tcpdump otherwise
# drops to its own user after the first file and dies at the next one, which
# left one minute of capture and missed the transfer (0.6.2).
if [ "$CAPTURE" = true ]; then
    mkdir -p "$INBOX/debug"
    tcpdump -Z root -i mon0 -s 256 -C 20 -W 8 -w "$INBOX/debug/mon0.pcap" >/dev/null 2>&1 &
    tcpdump -Z root -i awdl0 -s 256 -C 20 -W 4 -w "$INBOX/debug/awdl0.pcap" 'tcp port 8771' >/dev/null 2>&1 &
    say "debug_capture: writing pcap files to ${INBOX}/debug (mon0 up to 160 MB, awdl0 up to 80 MB, newest kept)."
fi
if [ "$VERBOSE" = true ]; then
    frames_heard 3
    tcp_flow &
    TCPFLOW_PID=$!
fi
watch_radio &
WATCH_PID=$!
wait "$SERVE_PID" || true
stop_stack
exit 0
