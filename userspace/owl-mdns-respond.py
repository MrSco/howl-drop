#!/usr/bin/env python3
"""omdrop-awdl's awdl-mdns-respond.py, framed for OWL's awdl0.

The responder answers a peer's mDNS queries for our host, _airdrop._tcp and
our instance with the full announcement (PTR, SRV, TXT, AAAA). omdrop-awdl's
helper runs it for the whole window; omdrop-owl's never did, so a phone
learned of us only from the unsolicited announcements. In exclusive mode an
iPhone on 5 GHz Wi-Fi is on our channel one slot in sixteen (8260, channel
6, 2026-10-12), and an announcement lands only if it happens to go out in
that slot. A query arrives in that slot, so an answer sent at once is still
there for the phone.

It frames its replies Apple-SNAP, for the Broadcom firmware. OWL's awdl0
takes Ethernet II and adds the AWDL encapsulation itself, which is why the
announcer runs with --plain. This replaces wrap() with the plain framing and
runs the responder unmodified; it already reads plain queries.

  owl-mdns-respond.py --iface awdl0 --port 8771 --flags 137
"""
import importlib
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
adv = importlib.import_module('awdl-airdrop-adv')
snap_wrap = adv.wrap


def plain_wrap(mac, payload, seq, snap=True):
    return snap_wrap(mac, payload, seq, snap=False)


# The responder imports the same module object and looks wrap up per reply.
adv.wrap = plain_wrap
importlib.import_module('awdl-mdns-respond').main()
