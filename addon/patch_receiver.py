#!/usr/bin/env python3
"""Instrument omdrop-plugin's airdrop-serve.py at image build time.

Three photo uploads on the MT7612U stopped 3 KB short with "nothing arrived
for 30 s", at the same byte each time. The awdl0 capture of the third shows
the phone's last segments arriving out of order, the gap filled at t+1.128 s,
our ACK covering every byte, and then 30 s of silence: the bytes reached this
machine and the receiver never read them (2026-10-14). When an upload read
times out, this logs where they are: unread in the kernel's socket queue, or
decrypted and pending in the TLS layer.

The receiver is otherwise unchanged; an anchor that is missing fails the
build rather than shipping a receiver without the log.

  patch_receiver.py /opt/omdrop-receiver/airdrop-serve.py
"""
import sys
from pathlib import Path

ANCHOR = '''        except socket.timeout as e:
            # Chunked framing announces no length, so there is none to name.
            expected = plan[1] if plan else None
'''

DIAG = '''        except socket.timeout as e:
            # Chunked framing announces no length, so there is none to name.
            expected = plan[1] if plan else None
            # howl-drop: where the bytes the phone sent and we ACKed are.
            try:
                import fcntl, termios
                unread = struct.unpack('i', fcntl.ioctl(self.request.fileno(), termios.FIONREAD, b'\\0' * 4))[0]
            except Exception as diag_err:
                unread = f'? ({diag_err})'
            try:
                pending = self.request.pending()
            except Exception as diag_err:
                pending = f'? ({diag_err})'
            try:
                tls = self.request.version()
            except Exception:
                tls = '?'
            logging.warning('upload %s stalled: %s unread in the kernel socket queue, %s pending in the '
                            'TLS layer (%s)', tid, unread, pending, tls)
'''


def main(path):
    p = Path(path)
    s = p.read_text()
    if 'howl-drop: where the bytes' in s:
        return 0
    if s.count(ANCHOR) != 1:
        sys.exit(f'{path}: upload timeout handler not found; the receiver changed')
    p.write_text(s.replace(ANCHOR, DIAG))
    print(f'patched {path}: upload stalls log the kernel and TLS backlog')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1]))
