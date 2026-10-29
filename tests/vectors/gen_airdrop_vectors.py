#!/usr/bin/env python3
"""Generate airdrop-vectors.json: byte-level vectors for normal AirDrop transfers.

Usage: gen_airdrop_vectors.py [out.json]

The vectors are reconstructed, not captured. Message shapes follow
docs/SPEC.md: the current /Ask form from the Finder capture documented in the
Protocol Prying artifact ([RD], its RESEARCH_DOCUMENT.md), and the legacy form
that older senders ("legacy senders") send. Every value is fixed, so the output
only changes when this script does.
"""

import base64
import hashlib
import json
import os
import plistlib
import struct
import sys
import zlib

MTIME = 1700000000
UID, GID = 501, 20
TRANSFER_ID = "A3F34E01-0000-4000-8000-000000000001"
TRANSFER_ID_2 = "A3F34E01-0000-4000-8000-000000000002"
TRANSFER_ID_3 = "A3F34E01-0000-4000-8000-000000000003"
SENDER_ID = "486ac2a94ac6"
DVZIP_PIECE = 0x10000

HELLO = b"hello world\n"
VCARD = b"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Test Contact\r\nEND:VCARD\r\n"


def b64(data):
    return base64.b64encode(data).decode()


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def png_1x1():
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    ihdr = struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)
    idat = zlib.compress(b"\x00\xff\x00\x00", 9)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


PNG = png_1x1()


def lines(count):
    return b"".join(b"AirDrop vector line %06d\n" % i for i in range(count))


# ── Archives ──────────────────────────────────────────────────

def odc_entry(name, data, mode, ino, nlink=1, uid=UID, gid=GID, mtime=MTIME):
    name = name.encode() + b"\0"
    return (b"070707" + b"%06o" % 0 + b"%06o" % ino + b"%06o" % mode + b"%06o" % uid
            + b"%06o" % gid + b"%06o" % nlink + b"%06o" % 0 + b"%011o" % mtime
            + b"%06o" % len(name) + b"%011o" % len(data) + name + data)


def odc_archive(files, root=True):
    """files: [(name, data)]. Layout per UP-2a: '.', './<name>'..., trailer."""
    out = bytearray()
    ino = 1
    if root:
        out += odc_entry(".", b"", 0o40755, ino, nlink=3)
        ino += 1
    for name, data in files:
        out += odc_entry(f"./{name}", data, 0o100644, ino)
        ino += 1
    out += odc_entry("TRAILER!!!", b"", 0, 0, nlink=0, uid=0, gid=0, mtime=0)
    return bytes(out)


def newc_entry(name, data, mode, ino, nlink=1, uid=UID, gid=GID, mtime=MTIME):
    name = name.encode() + b"\0"
    fields = [ino, mode, uid, gid, nlink, mtime, len(data), 0, 0, 0, 0, len(name), 0]
    hdr = b"070701" + b"".join(b"%08x" % f for f in fields)
    pad = lambda n: b"\0" * (-n % 4)
    return hdr + name + pad(len(hdr) + len(name)) + data + pad(len(data))


def newc_archive(files):
    out = bytearray(newc_entry(".", b"", 0o40755, 1, nlink=3))
    for i, (name, data) in enumerate(files, start=2):
        out += newc_entry(f"./{name}", data, 0o100644, i)
    out += newc_entry("TRAILER!!!", b"", 0, 0, nlink=0, uid=0, gid=0, mtime=0)
    return bytes(out)


def dvzip(data, store=False):
    """DVZ-1/DVZ-2: 64 KiB pieces, each its own zlib stream behind a BE length.
    With store, a piece zlib doesn't shrink goes as it is, behind a length
    with its top bit set, as Apple senders do."""
    out = bytearray()
    for off in range(0, len(data), DVZIP_PIECE):
        piece = data[off:off + DVZIP_PIECE]
        c = zlib.compress(piece, 6)
        if store and len(c) >= len(piece):
            out += struct.pack(">I", len(piece) | 0x80000000) + piece
        else:
            out += struct.pack(">I", len(c)) + c
    return bytes(out)


def gzip_member(data):
    """data as one gzip member: mtime 0, no name, OS 255 (unknown). Built
    here, not with gzip.compress, whose OS byte depends on the Python version."""
    c = zlib.compressobj(9, zlib.DEFLATED, -zlib.MAX_WBITS)
    return (b"\x1f\x8b\x08\x00" + struct.pack("<I", 0) + b"\x02\xff" + c.compress(data) + c.flush()
            + struct.pack("<II", zlib.crc32(data), len(data) & 0xFFFFFFFF))


def noise(n):
    """n bytes zlib can't shrink, as a photo's are."""
    out = bytearray()
    i = 0
    while len(out) < n:
        out += hashlib.sha256(struct.pack(">I", i)).digest()
        i += 1
    return bytes(out[:n])


def dvzip_decode(data):
    out, off, chunks = bytearray(), 0, []
    while off < len(data):
        (n,) = struct.unpack(">I", data[off:off + 4])
        if n & 0x80000000:
            n &= 0x7FFFFFFF
            out += data[off + 4:off + 4 + n]
        else:
            d = zlib.decompressobj()
            out += d.decompress(data[off + 4:off + 4 + n])
            assert d.eof and not d.unused_data, "chunk is not one complete zlib stream"
        chunks.append(n)
        off += 4 + n
    return bytes(out), chunks


# ── Plists and HTTP ───────────────────────────────────────────

def bplist(obj):
    return plistlib.dumps(obj, fmt=plistlib.FMT_BINARY, sort_keys=True)


def ask_ios18(files, transfer_id=TRANSFER_ID):
    return {
        "TransferID": {"id": transfer_id},
        "TransferType": {"files": {}},
        "SenderComputerName": "Test Mac",
        "BundleID": "com.apple.finder",
        "SenderModelName": "MacBook Pro",
        "SenderID": SENDER_ID,
        "ConvertMediaFormats": False,
        "Files": [{
            "FileName": name,
            "FileType": uti,
            "FileSize": len(data),
            "FileBomPath": f"./{name}",
            "FileIsDirectory": False,
            "ShouldConvertMediaFormats": False,
        } for name, uti, data in files],
        "Items": [],
    }


def ask_legacy(files):
    return {
        "SenderComputerName": "Test Mac",
        "BundleID": "com.apple.finder",
        "SenderModelName": "MacBookPro14,1",
        "SenderID": SENDER_ID,
        "ConvertMediaFormats": False,
        "Files": [{
            "FileName": name,
            "FileType": uti,
            "FileBomPath": f"./{name}",
            "FileIsDirectory": False,
            "ConvertMediaFormats": 0,
        } for name, uti, _ in files],
    }


def ask_expect(plist):
    tid = plist.get("TransferID")
    return {
        "transfer_id": tid["id"] if tid else None,
        "files": [{
            "name": f["FileName"],
            "bom_path": f["FileBomPath"],
            "type": f["FileType"],
            "size": f.get("FileSize"),
            "is_directory": f["FileIsDirectory"],
        } for f in plist.get("Files", [])],
        "items": plist.get("Items", []),
    }


def http(path, headers, body, chunked):
    head = f"POST {path} HTTP/1.1\r\n" + "".join(f"{k}: {v}\r\n" for k, v in headers)
    if chunked:
        head += "Transfer-Encoding: chunked\r\n\r\n"
        payload = (b"%x\r\n" % len(body) + body + b"\r\n" if body else b"") + b"0\r\n\r\n"
    else:
        head += f"Content-Length: {len(body)}\r\n\r\n"
        payload = body
    return head.encode() + payload


FINDER_HEADERS = [("User-Agent", "AirDrop/1.0"), ("Content-Type", "application/octet-stream"),
                  ("Connection", "keep-alive")]
LEGACY_HEADERS = FINDER_HEADERS + [("Accept", "*/*"), ("Accept-Language", "en-us"),
                                   ("Accept-Encoding", "br, gzip, deflate")]


def expect_files(files):
    return [{"path": f"./{name}", "mode": 0o100644, "data_b64": b64(data)} for name, data in files]


# ── Vector groups ─────────────────────────────────────────────

def discover_vectors():
    body = bplist({})
    return [
        {"name": "empty request, Finder framing (chunked)", "covers": ["HTTP-2", "HTTP-2a", "HTTP-3"],
         "source": "reconstructed from [RD]",
         "request_b64": b64(http("/Discover", FINDER_HEADERS, body, chunked=True)),
         "body_b64": b64(body), "expect": "accept"},
        {"name": "empty request, legacy sender framing (Content-Length)", "covers": ["HTTP-2", "HTTP-2a", "HTTP-3"],
         "source": "reconstructed from a legacy sender",
         "request_b64": b64(http("/Discover", LEGACY_HEADERS, body, chunked=False)),
         "body_b64": b64(body), "expect": "accept"},
    ]


HELLO_FILES = [("hello.txt", "public.plain-text", HELLO)]
MIXED_FILES = [("contact.vcf", "public.vcard", VCARD), ("pixel.png", "public.png", PNG),
               ("empty.dat", "public.data", b"")]
# A photo: bytes zlib can't shrink, past two DVZip pieces.
PHOTO_FILES = [("IMG_0001.jpg", "public.jpeg", b"\xff\xd8\xff\xe0" + noise(150000))]

ASKS = {
    "ios18-hello": ask_ios18(HELLO_FILES),
    "ios18-three-files": ask_ios18(MIXED_FILES, TRANSFER_ID_2),
    "legacy-hello": ask_legacy(HELLO_FILES),
    "ios18-photo": ask_ios18(PHOTO_FILES, TRANSFER_ID_3),
}


def ask_vectors():
    out = []
    for name, plist, source, covers in [
        ("ios18-hello", ASKS["ios18-hello"], "reconstructed from the [RD] Finder capture", ["ASK-7"]),
        ("ios18-three-files", ASKS["ios18-three-files"], "reconstructed from the [RD] Finder capture", ["ASK-7"]),
        ("legacy-hello", ASKS["legacy-hello"], "reconstructed from the legacy form (§6.4)", ["§6.4"]),
        ("ios18-photo", ASKS["ios18-photo"], "reconstructed from an iPhone's /Ask for a JPEG [OWY]", ["ASK-7"]),
    ]:
        body = bplist(plist)
        out.append({"name": name, "covers": covers, "source": source,
                    "request_b64": b64(http("/Ask", FINDER_HEADERS, body, chunked=True)),
                    "body_b64": b64(body), "expect": ask_expect(plist)})
    return out


def dvzip_vectors():
    out = []
    for name, data in [("one short chunk", HELLO), ("exactly one piece (65536 bytes)", lines(2731)[:DVZIP_PIECE]),
                       ("one byte past a piece (two chunks)", lines(2731)[:DVZIP_PIECE + 1]),
                       ("200000 bytes (four chunks)", lines(8334)[:200000])]:
        enc = dvzip(data)
        dec, chunks = dvzip_decode(enc)
        assert dec == data
        out.append({"name": name, "covers": ["DVZ-1", "DVZ-2"], "source": "reconstructed from [RD]",
                    "input_b64": b64(enc),
                    "expect": {"output_len": len(data), "output_sha256": sha256(data), "chunks": len(chunks)}})
    # Text, then a photo's bytes, then text: compressed, stored, compressed.
    data = lines(2731)[:DVZIP_PIECE] + noise(2 * DVZIP_PIECE) + lines(100)
    enc = dvzip(data, store=True)
    dec, chunks = dvzip_decode(enc)
    assert dec == data and len(chunks) == 4
    out.append({"name": "stored chunks between compressed ones", "covers": ["DVZ-1", "DVZ-3", "DVZ-4"],
                "source": "reconstructed as an iPhone sent a JPEG [OWY]",
                "input_b64": b64(enc),
                "expect": {"output_len": len(data), "output_sha256": sha256(data), "chunks": len(chunks)}})
    return out


def cpio_vectors():
    hello = [("hello.txt", HELLO)]
    mixed = [(n, d) for n, _, d in MIXED_FILES]
    return [
        {"name": "odc, Finder layout", "covers": ["UP-2", "UP-2a"], "source": "reconstructed from [RD]",
         "format": "odc", "input_b64": b64(odc_archive(hello)), "expect": {"files": expect_files(hello)}},
        {"name": "odc, no root entry (legacy sender layout)", "covers": ["UP-2", "UP-2a"], "source": "reconstructed from a legacy sender",
         "format": "odc", "input_b64": b64(odc_archive(hello, root=False)), "expect": {"files": expect_files(hello)}},
        {"name": "odc, three files incl. an empty one", "covers": ["UP-2", "UP-2a"], "source": "reconstructed from [RD]",
         "format": "odc", "input_b64": b64(odc_archive(mixed)), "expect": {"files": expect_files(mixed)}},
        {"name": "newc, Finder layout", "covers": ["UP-2"], "source": "reconstructed from [PP]",
         "format": "newc", "input_b64": b64(newc_archive(hello)), "expect": {"files": expect_files(hello)}},
    ]


def upload_vectors():
    hello = [("hello.txt", HELLO)]
    mixed = [(n, d) for n, _, d in MIXED_FILES]
    photo = [(n, d) for n, _, d in PHOTO_FILES]

    def finder_upload(files, transfer_id, store=False):
        body = dvzip(odc_archive(files), store=store)
        headers = [("User-Agent", "AirDrop/1.0"), ("TransferID", transfer_id), ("TotalBytes", "0"),
                   ("Content-Type", "application/x-dvzip"), ("Connection", "keep-alive")]
        return http("/Upload", headers, body, chunked=False)

    def legacy_upload(files):
        body = gzip_member(odc_archive(files, root=False))
        headers = LEGACY_HEADERS[:1] + [("Content-Type", "application/x-cpio")] + LEGACY_HEADERS[2:]
        return http("/Upload", headers, body, chunked=True)

    return [
        {"name": "Finder: DVZip, Content-Length, TransferID header", "after_ask": "ios18-hello",
         "covers": ["UP-1", "UP-1a", "UP-2a", "UP-9", "UP-10", "DVZ-1"], "source": "reconstructed from the [RD] Finder capture",
         "request_b64": b64(finder_upload(hello, TRANSFER_ID)), "expect": {"files": expect_files(hello)}},
        {"name": "Finder: three files", "after_ask": "ios18-three-files",
         "covers": ["UP-1", "UP-1a", "UP-2a", "UP-9", "DVZ-1"], "source": "reconstructed from the [RD] Finder capture",
         "request_b64": b64(finder_upload(mixed, TRANSFER_ID_2)), "expect": {"files": expect_files(mixed)}},
        {"name": "iPhone: a photo's bytes in stored DVZip chunks", "after_ask": "ios18-photo",
         "covers": ["UP-1", "UP-1a", "UP-9", "DVZ-1", "DVZ-3"], "source": "reconstructed as an iPhone sent a JPEG [OWY]",
         "request_b64": b64(finder_upload(photo, TRANSFER_ID_3, store=True)), "expect": {"files": expect_files(photo)}},
        {"name": "Legacy form: gzip cpio, chunked, no TransferID", "after_ask": "legacy-hello",
         "covers": ["UP-1", "UP-1a", "UP-2", "UP-9"], "source": "reconstructed from the legacy form (§6.4)",
         "request_b64": b64(legacy_upload(hello)), "expect": {"files": expect_files(hello)}},
    ]


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "airdrop-vectors.json")
    vectors = {
        "contract": "docs/SPEC.md",
        "version": 1,
        "note": "Reconstructed from documented message shapes, not captured from Apple devices. "
                "Regenerate with gen_airdrop_vectors.py. Encoders should match decoded structure, "
                "not compressed bytes, since zlib output can differ between versions.",
        "constants": {"mtime": MTIME, "uid": UID, "gid": GID, "sender_id": SENDER_ID,
                      "transfer_ids": [TRANSFER_ID, TRANSFER_ID_2, TRANSFER_ID_3], "dvzip_piece": DVZIP_PIECE},
        "discover": discover_vectors(),
        "ask": ask_vectors(),
        "dvzip": dvzip_vectors(),
        "cpio": cpio_vectors(),
        "upload": upload_vectors(),
    }
    with open(out, "w") as f:
        json.dump(vectors, f, indent=1, sort_keys=True)
        f.write("\n")


if __name__ == "__main__":
    main()
