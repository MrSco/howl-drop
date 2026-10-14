"""Read an AirDrop /Upload body and answer once the archive is whole.

The iPhone sends /Upload chunked and sometimes never sends the chunked
terminator ("0\\r\\n\\r\\n"). In the 2026-10-14 awdl0 capture a 1.1 MB photo
crossed in 1.1 s, its last segment carried PSH, our ACK covered every byte,
and then the phone sent nothing for 30 s: no resend, no terminator. The body
we held was the whole archive. omdrop-plugin's receiver waits for the
terminator, times out after 30 s and answers 408, and the phone reports a
failed transfer. A Mac answers when the archive is complete; so does this.

The archive is complete when its DVZip framing ends exactly (a gzip stream
reaches its end, or block framing ends on a block boundary) and the CPIO
stream inside it ends with its TRAILER!!! entry. That is tracked as the body
arrives, so it costs one extra decompression and no rereading. After the
archive completes, the terminator gets a short grace period. If it does not
come, the receiver answers anyway and closes the connection. The receiver's
own store_upload still decodes and extracts the body as before, and refuses
anything that does not hold up.

patch_receiver.py installs this beside airdrop-serve.py and routes
handle_upload's body read through read().
"""
import logging
import socket
import zlib

# How long the terminator may lag a complete archive before we answer.
TERMINATOR_GRACE_SECONDS = 1.0
# The CPIO trailer entry's name, NUL-terminated as the header stores it.
TRAILER = b'TRAILER!!!\x00'
TAIL_BYTES = 4096


class ArchiveEnd:
    """Follows a DVZip body as it arrives: is it a complete archive yet?"""

    def __init__(self):
        self.kind = None          # 'gzip' or 'blocks', from the first bytes
        self.head = b''
        self.tail = b''           # the last TAIL_BYTES of decompressed output
        self.out = 0
        self.broken = None        # why tracking gave up; the body is still read
        self.gzip = None
        # block framing
        self.header = b''
        self.left = 0             # bytes left in the current block
        self.stored = False
        self.block = None         # zlib decoder for a compressed block

    def _emit(self, data):
        if data:
            self.out += len(data)
            self.tail = (self.tail + data)[-TAIL_BYTES:]

    def feed(self, data):
        if self.broken or not data:
            return
        try:
            self._feed(data)
        except (zlib.error, ValueError) as e:
            self.broken = str(e)

    def _feed(self, data):
        if self.kind is None:
            self.head += data
            if len(self.head) < 2:
                return
            data, self.head = self.head, b''
            if data[:2] == b'\x1f\x8b':
                self.kind = 'gzip'
                self.gzip = zlib.decompressobj(31)
            else:
                self.kind = 'blocks'
        if self.kind == 'gzip':
            if self.gzip.eof:
                raise ValueError('data after the end of the gzip stream')
            self._emit(self.gzip.decompress(data))
            if self.gzip.eof and self.gzip.unused_data:
                raise ValueError('data after the end of the gzip stream')
            return
        while data:
            if not self.left:
                need = 4 - len(self.header)
                self.header += data[:need]
                data = data[need:]
                if len(self.header) < 4:
                    return
                value = int.from_bytes(self.header, 'big')
                self.header = b''
                self.left = value & 0x7fffffff
                self.stored = bool(value & 0x80000000)
                self.block = None if self.stored else zlib.decompressobj(zlib.MAX_WBITS)
                if not self.left:
                    continue
            piece, data = data[:self.left], data[self.left:]
            self.left -= len(piece)
            if self.stored:
                self._emit(piece)
            else:
                self._emit(self.block.decompress(piece))
                if not self.left:
                    self._emit(self.block.flush())
                    if not self.block.eof or self.block.unused_data:
                        raise ValueError('compressed block does not end with its data')

    @property
    def complete(self):
        if self.broken or self.kind is None:
            return False
        if self.kind == 'gzip':
            ended = self.gzip.eof
        else:
            ended = not self.left and not self.header
        return ended and TRAILER in self.tail

    def describe(self):
        if self.broken:
            return f'archive tracking stopped: {self.broken}'
        if self.kind is None:
            return 'no archive bytes yet'
        if self.kind == 'gzip':
            where = 'gzip stream ended' if self.gzip.eof else 'gzip stream still open'
        elif self.left or self.header:
            where = f'inside a block, {self.left} bytes of it missing'
        else:
            where = 'on a block boundary'
        trailer = 'with' if TRAILER in self.tail else 'without'
        return f'{self.kind} archive {where}, {self.out} bytes decompressed, {trailer} the CPIO trailer'


class TrackingWriter:
    """Passes body bytes to the receiver's bounded writer and the tracker."""

    def __init__(self, writer, tracker):
        self.writer = writer
        self.tracker = tracker

    @property
    def remaining(self):
        return self.writer.remaining

    @property
    def limit(self):
        return self.writer.limit

    @property
    def total(self):
        return self.writer.total

    def write(self, data):
        written = self.writer.write(data)
        self.tracker.feed(data)
        return written


def _chunk_size(rfile, deadline, serve):
    line = rfile.readline(serve.MAX_CHUNK_LINE_BYTES + 1)
    deadline.note(len(line))
    if not line or len(line) > serve.MAX_CHUNK_LINE_BYTES or not line.endswith(b'\n'):
        raise serve.RequestBodyError('invalid or overlong chunk-size line')
    token = line.split(b';', 1)[0].strip()
    try:
        size = int(token, 16)
    except ValueError:
        raise serve.RequestBodyError(f'invalid chunk size: {token[:40]!r}') from None
    if size < 0:
        raise serve.RequestBodyError('negative chunk size')
    return size


def _trailers(rfile, deadline, serve):
    trailers = 0
    while True:
        line = rfile.readline(serve.MAX_CHUNK_LINE_BYTES + 1)
        deadline.note(len(line))
        trailers += len(line)
        if (not line or len(line) > serve.MAX_CHUNK_LINE_BYTES
                or trailers > serve.MAX_TRAILER_BYTES):
            raise serve.RequestBodyError('invalid or overlong chunk trailers')
        if line in (b'\r\n', b'\n'):
            return


def read_chunked(handler, writer, tracker, deadline, tid, serve):
    """serve._read_chunked, except that a complete archive ends the body
    when the terminator does not follow within the grace period."""
    rfile = handler.rfile
    while True:
        if tracker.complete:
            handler.request.settimeout(TERMINATOR_GRACE_SECONDS)
            try:
                size = _chunk_size(rfile, deadline, serve)
            except (socket.timeout, serve.RequestBodyError) as e:
                # A timed-out read leaves rfile unusable, and a terminator that
                # turns up later would be read as the next request: close.
                handler.close_connection = True
                logging.info('upload %s: archive complete at %d bytes, no end-of-body after %.1f s (%s); '
                             'answering without it', tid, writer.total, TERMINATOR_GRACE_SECONDS,
                             'timed out' if isinstance(e, socket.timeout) else e)
                return
            handler.request.settimeout(serve.READ_IDLE_TIMEOUT_SECONDS)
        else:
            size = _chunk_size(rfile, deadline, serve)
        if size == 0:
            _trailers(rfile, deadline, serve)
            return
        if size > writer.remaining:
            raise serve.BodyTooLarge(f'request body exceeds {writer.limit}-byte limit')
        serve._copy_exact(rfile, writer, size, 'truncated chunk data', deadline)
        ending = rfile.read(2)
        deadline.note(len(ending))
        if ending != b'\r\n':
            raise serve.RequestBodyError('chunk data missing CRLF')


def read(handler, raw, limit, plan, deadline, tid, serve):
    """Stand-in for serve.read_request_body in handle_upload."""
    if plan[0] != 'chunked':
        return serve.read_request_body(handler.rfile, handler.headers, raw, limit,
                                       check_disk=True, plan=plan, deadline=deadline)
    tracker = ArchiveEnd()
    writer = TrackingWriter(serve.LimitedWriter(raw, limit, 'request body', serve.BodyTooLarge, True),
                            tracker)
    try:
        read_chunked(handler, writer, tracker, deadline, tid, serve)
    except socket.timeout:
        logging.warning('upload %s: at the stall, %s', tid, tracker.describe())
        raise
    return writer.total
