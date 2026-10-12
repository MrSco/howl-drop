#!/usr/bin/env python3
"""Answer a phone's mDNS queries for our AirDrop identity on OWL's awdl0.

omdrop-awdl's awdl-mdns-respond.py answers queries for our host,
_airdrop._tcp and our instance with the full announcement (PTR, SRV, TXT,
AAAA). omdrop-owl's helper never ran it, so a phone learned of us only from
unsolicited announcements, and in exclusive mode an iPhone on 5 GHz Wi-Fi is
on our channel one slot in sixteen (8260, channel 6, 2026-10-12).

Two things differ here, both because of how OWL sends:

- Framing. The responder frames its replies Apple-SNAP, for the Broadcom
  firmware. OWL's awdl0 takes Ethernet II and adds the AWDL encapsulation
  itself (the announcer's --plain).
- Unicast. OWL -F sends multicast at once, in any slot, so a multicast
  answer usually leaves after the phone has gone back to 44 or 149: on
  0.2.6 a phone queried three times, every query was answered, and it never
  went on to resolve our host. Unicast to the phone is held until the phone
  is on our channel and sent owl -U times. RFC 6762 6.7 lets a responder
  answer a querier directly; iOS takes it from an on-link address. Each
  query gets that unicast answer and the multicast burst as before.

The query parsing and record building are the responder's and the
announcer's, imported unmodified.

  owl-mdns-respond.py --iface awdl0 --port 8771 --flags 137
"""
import argparse
import importlib
import os
import socket
import struct
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
adv = importlib.import_module('awdl-airdrop-adv')
ident = importlib.import_module('awdl_identity')
respond = importlib.import_module('awdl-mdns-respond')

ETH_P_ALL = 0x0003
MDNS_MAC = b'\x33\x33\x00\x00\x00\xfb'


def query(frame):
    """(src mac, src IPv6, dns bytes) for an Ethernet II IPv6/UDP/5353 frame, else None."""
    if len(frame) < 62 or frame[12:14] != b'\x86\xdd':
        return None
    ip6 = frame[14:54]
    if ip6[6] != 17 or struct.unpack('!H', frame[56:58])[0] != 5353:
        return None
    return frame[6:12], ip6[8:24], frame[62:]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--iface', default='awdl0')
    ap.add_argument('--port', type=int, default=8771)
    ap.add_argument('--flags', type=int, default=0x88)
    ap.add_argument('--host', default=f'{ident.awdl_host()}.local')
    ap.add_argument('--service-id', default=None)
    ap.add_argument('--min-gap', type=float, default=0.1, help='seconds between replies')
    ap.add_argument('--burst', type=int, default=5, help='multicast frames per reply')
    a = ap.parse_args()

    mac = bytes.fromhex(open(f'/sys/class/net/{a.iface}/address').read().strip().replace(':', ''))
    sid = a.service_id or mac.hex()
    ll = adv.eui64_ll(mac)
    mcast6 = socket.inet_pton(socket.AF_INET6, 'ff02::fb')
    # As the responder builds it: the announcement, plus an NSEC naming the
    # types we have, so a query for an A record stops waiting (RFC 6762 6.1).
    msg = adv.announcement(sid, a.host, a.port, a.flags, ll)
    nsec = adv.rr(a.host, 47, adv.CACHE_FLUSH, 120, adv.dname(a.host) + b'\x00\x06' + bytes([0, 0, 0, 0x08, 0, 0x01]))
    ancount = struct.unpack('!H', msg[6:8])[0] + 1
    msg = msg[:6] + struct.pack('!H', ancount) + msg[8:] + nsec
    multicast = MDNS_MAC + mac + b'\x86\xdd' + adv.udp6(ll, mcast6, msg)
    ours = {a.host.lower().rstrip('.'), '_airdrop._tcp.local', f'{sid}._airdrop._tcp.local'.lower()}

    s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(ETH_P_ALL))
    s.bind((a.iface, 0))
    print(f'{time.time():.3f} responding on {a.iface} for {sorted(ours)} -> '
          f'{socket.inet_ntop(socket.AF_INET6, ll)}:{a.port}, unicast and multicast', flush=True)
    last = 0.0
    replies = 0
    while True:
        got = query(s.recv(4096))
        if not got:
            continue
        src_mac, src6, dns = got
        if src_mac == mac:
            continue
        hit = [q for q in respond.questions(dns) if q in ours]
        if not hit:
            continue
        now = time.time()
        if now - last < a.min_gap:
            continue
        try:
            # Unicast first: OWL queues it for the phone's next visit.
            s.send(src_mac + mac + b'\x86\xdd' + adv.udp6(ll, src6, msg))
            for i in range(a.burst):
                s.send(multicast)
                if i + 1 < a.burst:
                    time.sleep(0.02)
        except OSError as e:
            print(f'{now:.3f} send error: {e}', flush=True)
            continue
        last = now
        replies += 1
        print(f'{now:.3f} {time.strftime("%H:%M:%SZ", time.gmtime(now))} query from {src_mac.hex(":")} '
              f'({socket.inet_ntop(socket.AF_INET6, src6)}) for {hit[0]} -> answered, unicast and multicast ({replies})',
              flush=True)


if __name__ == '__main__':
    main()
