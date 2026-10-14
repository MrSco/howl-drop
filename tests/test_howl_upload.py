"""addon/howl_upload.py over a socket pair: a chunked upload whose archive is
complete is answered without the terminator, one with the terminator reads as
before, and a truncated one still times out."""
import gzip
import importlib.util
import io
from pathlib import Path
import socket
import threading
import time
import types
import unittest
import zlib

SPEC = importlib.util.spec_from_file_location(
    'howl_upload', Path(__file__).resolve().parents[1] / 'addon' / 'howl_upload.py')
howl_upload = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(howl_upload)


# The parts of omdrop-plugin's airdrop-serve.py that howl_upload.read uses,
# reduced to what the test needs.
class RequestBodyError(ValueError):
    status = 400


class BodyTooLarge(RequestBodyError):
    status = 413


class LimitedWriter:
    def __init__(self, target, limit, label, limit_error=None, check_disk=False):
        self.target, self.limit, self.total = target, limit, 0

    @property
    def remaining(self):
        return self.limit - self.total

    def write(self, data):
        self.target.write(data)
        self.total += len(data)
        return len(data)


class Deadline:
    def __init__(self):
        self.transferred = 0

    def note(self, size):
        self.transferred += size


def copy_exact(source, writer, size, truncated, deadline):
    left = size
    while left:
        data = source.read1(min(left, 65536))
        if not data:
            raise RequestBodyError(truncated)
        deadline.note(len(data))
        writer.write(data)
        left -= len(data)


SERVE = types.SimpleNamespace(
    RequestBodyError=RequestBodyError, BodyTooLarge=BodyTooLarge, LimitedWriter=LimitedWriter,
    _copy_exact=copy_exact, read_request_body=None, MAX_CHUNK_LINE_BYTES=128,
    MAX_TRAILER_BYTES=8192, READ_IDLE_TIMEOUT_SECONDS=1.5)


def odc(name, data):
    name = name.encode() + b'\x00'
    fields = [0, 1, 0o100644, 0, 0, 1, 0]
    head = b'070707' + b''.join(b'%06o' % f for f in fields) + b'%011o' % 0
    return head + b'%06o' % len(name) + b'%011o' % len(data) + name + data


def cpio(files):
    body = b''.join(odc(n, d) for n, d in files) + odc('TRAILER!!!', b'')
    return body + b'\x00' * (-len(body) % 512)


PHOTO = bytes(range(256)) * 12000 + b'TRAILER!!!\x00 inside the photo'


def gzip_dvzip(files):
    return gzip.compress(cpio(files))


def block_dvzip(files, size=100000):
    raw = cpio(files)
    out = b''
    for i, start in enumerate(range(0, len(raw), size)):
        piece = raw[start:start + size]
        if i % 2:
            out += (0x80000000 | len(piece)).to_bytes(4, 'big') + piece
        else:
            packed = zlib.compress(piece)
            out += len(packed).to_bytes(4, 'big') + packed
    return out


def chunked(body, size=65536, terminator=True):
    out = b''
    for start in range(0, len(body), size):
        piece = body[start:start + size]
        out += b'%x\r\n' % len(piece) + piece + b'\r\n'
    return out + (b'0\r\n\r\n' if terminator else b'')


class Upload:
    """The server end of a socket pair, as handle_upload sees it."""

    def __init__(self, wire, hold_open=3.0):
        self.server, client = socket.socketpair()
        self.server.settimeout(SERVE.READ_IDLE_TIMEOUT_SECONDS)
        self.rfile = self.server.makefile('rb')
        self.request = self.server
        self.headers = {}
        self.close_connection = False

        def send():
            client.sendall(wire)
            time.sleep(hold_open)
            client.close()
        threading.Thread(target=send, daemon=True).start()

    def close(self):
        self.rfile.close()
        self.server.close()

    def read(self):
        raw = io.BytesIO()
        started = time.monotonic()
        try:
            total = howl_upload.read(self, raw, 1 << 30, ('chunked', None), Deadline(), 'T', SERVE)
        finally:
            self.elapsed = time.monotonic() - started
        return total, raw.getvalue()


class ArchiveEndTest(unittest.TestCase):
    def test_gzip_completes_only_at_its_end(self):
        body = gzip_dvzip([('a.jpg', PHOTO)])
        t = howl_upload.ArchiveEnd()
        t.feed(body[:-8])
        self.assertFalse(t.complete)
        t.feed(body[-8:])
        self.assertTrue(t.complete, t.describe())

    def test_blocks_complete_only_on_the_trailer(self):
        body = block_dvzip([('a.jpg', PHOTO)])
        t = howl_upload.ArchiveEnd()
        for start in range(0, len(body), 777):
            self.assertFalse(t.complete, start)
            t.feed(body[start:start + 777])
        self.assertTrue(t.complete, t.describe())

    def test_trailer_text_in_a_file_is_not_the_end(self):
        raw = cpio([('a.jpg', PHOTO)])
        cut = raw.index(b'TRAILER!!!\x00 inside') + 20
        t = howl_upload.ArchiveEnd()
        t.feed((0x80000000 | cut).to_bytes(4, 'big') + raw[:cut])
        self.assertTrue(howl_upload.TRAILER in t.tail)
        # Stored framing on a block boundary with the text in the tail: the
        # one case the check cannot tell apart, which store_upload then
        # refuses. gzip has no such case:
        g = howl_upload.ArchiveEnd()
        g.feed(gzip.compress(raw)[:-20])
        self.assertFalse(g.complete)

    def test_garbage_stops_tracking_without_raising(self):
        t = howl_upload.ArchiveEnd()
        t.feed(b'\x1f\x8bnot gzip at all')
        self.assertFalse(t.complete)
        self.assertIn('stopped', t.describe())


class ReadTest(unittest.TestCase):
    def upload(self, wire):
        up = Upload(wire)
        self.addCleanup(up.close)
        return up

    def test_complete_archive_without_terminator_is_answered(self):
        body = gzip_dvzip([('a.jpg', PHOTO)])
        up = self.upload(chunked(body, terminator=False))
        with self.assertLogs(level='INFO') as logs:
            total, raw = up.read()
        self.assertEqual(raw, body)
        self.assertEqual(total, len(body))
        self.assertTrue(up.close_connection)
        self.assertLess(up.elapsed, howl_upload.TERMINATOR_GRACE_SECONDS + 1)
        self.assertIn('answering without it', '\n'.join(logs.output))

    def test_block_framing_without_terminator_is_answered(self):
        body = block_dvzip([('a.jpg', PHOTO), ('b.heic', PHOTO[:5000])])
        up = self.upload(chunked(body, size=40000, terminator=False))
        total, raw = up.read()
        self.assertEqual(raw, body)
        self.assertTrue(up.close_connection)

    def test_terminator_keeps_the_connection(self):
        body = gzip_dvzip([('a.jpg', PHOTO)])
        up = self.upload(chunked(body))
        total, raw = up.read()
        self.assertEqual(raw, body)
        self.assertFalse(up.close_connection)
        self.assertEqual(up.server.gettimeout(), SERVE.READ_IDLE_TIMEOUT_SECONDS)
        self.assertLess(up.elapsed, 0.5)

    def test_truncated_archive_still_times_out(self):
        body = gzip_dvzip([('a.jpg', PHOTO)])
        up = self.upload(chunked(body[:-3000], terminator=False))
        with self.assertLogs(level='WARNING') as logs:
            with self.assertRaises(socket.timeout):
                up.read()
        self.assertIn('gzip stream still open', '\n'.join(logs.output))

    def test_content_length_goes_to_the_receiver(self):
        seen = []
        serve = types.SimpleNamespace(**vars(SERVE))
        serve.read_request_body = lambda *a, **k: seen.append(k['plan']) or 7
        self.assertEqual(howl_upload.read(types.SimpleNamespace(rfile=None, headers={}), io.BytesIO(), 10, ('length', 7), Deadline(), 'T', serve), 7)
        self.assertEqual(seen, [('length', 7)])


if __name__ == '__main__':
    unittest.main()
