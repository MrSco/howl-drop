#!/usr/bin/env python3
"""Seed each fuzz target's corpus from tests/vectors/airdrop-vectors.json,
and the AWDL ones with frames built here.

Usage: seed.py <corpus dir>
"""

import base64
import gzip
import json
import os
import sys

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
corpus = sys.argv[1]
with open(os.path.join(root, "tests", "vectors", "airdrop-vectors.json")) as f:
    vectors = json.load(f)


def b64(s):
    return base64.b64decode(s)


def seed(target, name, data):
    os.makedirs(os.path.join(corpus, target), exist_ok=True)
    with open(os.path.join(corpus, target, name), "wb") as f:
        f.write(data)


requests = []
for group in ("discover", "ask", "upload"):
    for i, v in enumerate(vectors[group]):
        request = b64(v["request_b64"])
        requests.append(request)
        seed("http", f"{group}-{i}", request)
        if "body_b64" in v:
            seed("plist", f"{group}-{i}", b64(v["body_b64"]))
for i, v in enumerate(vectors["cpio"]):
    archive = b64(v["input_b64"])
    seed("cpio", f"cpio-{i}", archive)
    seed("gzip", f"cpio-{i}", gzip.compress(archive, mtime=0))
for i, v in enumerate(vectors["dvzip"]):
    seed("dvzip", f"dvzip-{i}", b64(v["input_b64"]))
# A whole connection: /Discover, /Ask and /Upload, after a byte the target
# uses to choose read sizes.
discover, ask, upload = requests[0], requests[2], requests[len(vectors["discover"]) + len(vectors["ask"])]
seed("receiver", "finder", b"\x40" + discover + ask + upload)


# AWDL: an iPhone-like MIF and a data frame, each after its 2-byte length,
# with a first byte for the node's presence.
import struct

PHONE = bytes([0x86, 0x08, 0x04, 0x8A, 0x2B, 0xF4])
US = bytes([2, 0, 0, 0, 0, 1])
BSSID = bytes([0x00, 0x25, 0x00, 0xFF, 0x94, 0x73])


def header(fc, dst, src):
    return struct.pack("<HH", fc, 0) + dst + src + BSSID + struct.pack("<H", 1 << 4)


def tlv(kind, value):
    return struct.pack("<BH", kind, len(value)) + value


chanseq = bytes([15, 3, 0, 3, 0xFF, 0xFF]) + bytes([6, 0x51] * 16)
sync = bytes([6]) + struct.pack("<H", 40) + bytes([6, 0]) + struct.pack("<HHHHHH", 16, 110, 0x1800, 16, 16, 0) + bytes([3, 3, 3, 3]) + PHONE + bytes([4, 0]) + struct.pack("<HH", 400, 400) + chanseq + bytes(2)
election2 = PHONE + PHONE + struct.pack("<IIIIIII", 3, 0, 600, 600, 0, 0, 3)
mif = header(0x00D0, b"\xff" * 6, PHONE) + bytes([127, 0x00, 0x17, 0xF2, 8, 0x10, 3, 0]) + bytes(8)
mif += tlv(4, sync) + tlv(18, chanseq + bytes(3)) + tlv(24, election2) + tlv(16, bytes([3, 6]) + b"iPhone" + b"\xc0\x0c") + tlv(21, bytes([0x34, 2])) + bytes(2)
packet = bytes([0x60, 0, 0, 0, 0, 0, 59, 64]) + bytes(32)
data = header(0x0008, US, PHONE) + bytes([0xAA, 0xAA, 3, 0, 0x17, 0xF2, 8, 0, 3, 4, 0, 0, 0, 0, 0x86, 0xDD]) + packet
frames = b"".join(struct.pack("<H", len(f)) + f for f in (mif, data))
seed("awdl", "mif-data", b"\x01" + frames)
seed("awdl", "mif-data-shared", b"\x00" + frames)
radiotap_header = struct.pack("<BBHI", 0, 0, 9, 1 << 2) + bytes([24])
seed("radiotap", "mif", radiotap_header + mif)
