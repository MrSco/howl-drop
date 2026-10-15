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


# The add-on's verbose option: DEBUG logs every request's headers, which is
# most of the log during a transfer; run.sh passes INFO without verbose.
LOGGING = "logging.basicConfig(level=logging.DEBUG, stream=sys.stdout,\n"
LOGGING_NEW = "logging.basicConfig(level=os.environ.get('HOWLDROP_LOG_LEVEL', 'DEBUG'), stream=sys.stdout,\n"

# Who sent each stored file, for the add-on's Home Assistant event and
# notifications: a note in staging/.howldrop-meta (deliver.py, announce.py).
NOTE_FN_ANCHOR = 'class Handler(BaseHTTPRequestHandler):\n'
NOTE_FN = '''def howl_note(dest, names, sender, url=None):
    """howl-drop: note who sent each stored file (.howldrop-meta/<name>.json)."""
    try:
        meta = os.path.join(dest, '.howldrop-meta')
        os.makedirs(meta, exist_ok=True)
        for name in names:
            with open(os.path.join(meta, name + '.json'), 'w', encoding='utf-8') as fh:
                json.dump({'sender': sender or '', 'url': url}, fh)
    except OSError as e:
        logging.warning('could not note who sent %s: %s', ', '.join(names), e)


class Handler(BaseHTTPRequestHandler):
'''
ASK = "        files = ask.get('Files') or []\n"
ASK_NEW = ASK + "        self._howl_sender = ask.get('SenderComputerName') or ''  # howl-drop\n"
STORED = "            logging.info('upload %s: stored %s', tid, ', '.join(written))\n"
STORED_NEW = STORED + "            howl_note(DEST, written, getattr(self, '_howl_sender', ''))\n"
LINK = "                logging.info('link stored as %s -> %s', stored, item)\n"
LINK_NEW = LINK + "                howl_note(DEST, [stored], ask.get('SenderComputerName') or '', item)\n"

EDITS = [
    ('upload timeout handler', ANCHOR, DIAG),
    ('the logging setup', LOGGING, LOGGING_NEW),
    ('the Handler class', NOTE_FN_ANCHOR, NOTE_FN),
    ("handle_ask's file list", ASK, ASK_NEW),
    ("handle_upload's stored line", STORED, STORED_NEW),
    ("handle_ask's link line", LINK, LINK_NEW),
]

# contacts.py, beside the receiver: Contacts Only refused an iPhone on the
# senders list with "the validation record does not belong to the presented
# certificate" (2026-10-14). The certificate was Apple's and verified; the
# account was read from `openssl x509 -subject` by searching for "CN=", and
# Debian 12's OpenSSL 3.0 prints "CN = ". Read commonName one attribute per
# line instead, which every OpenSSL prints the same.
SUBJECT = '''        subject = subprocess.run(["openssl", "x509", "-in", str(leaf), "-noout", "-subject"],
                                 capture_output=True).stdout.decode(errors="replace")
    if "CN=" not in subject:
        return ""
    cn = subject.split("CN=", 1)[1].strip()
'''
SUBJECT_NEW = '''        # howl-drop: one attribute per line; OpenSSL 3.0 prints "CN = value".
        subject = subprocess.run(["openssl", "x509", "-in", str(leaf), "-noout", "-subject",
                                  "-nameopt", "multiline"],
                                 capture_output=True).stdout.decode(errors="replace")
    cn = ""
    for line in subject.splitlines():
        key, sep, value = line.partition("=")
        if sep and key.strip() == "commonName":
            cn = value.strip()
'''

CONTACTS_EDITS = [
    ("the certificate's account", SUBJECT, SUBJECT_NEW),
]


def apply(path, edits):
    p = Path(path)
    s = p.read_text()
    for what, anchor, new in edits:
        if new in s:
            continue
        if s.count(anchor) != 1:
            sys.exit(f'{path}: {what} not found; the receiver changed')
        s = s.replace(anchor, new)
    p.write_text(s)


def main(path):
    apply(path, EDITS)
    apply(Path(path).with_name('contacts.py'), CONTACTS_EDITS)
    print(f'patched {path}: upload stalls log the kernel and TLS backlog; '
          'the log level follows HOWLDROP_LOG_LEVEL; contacts.py reads the '
          'certificate account on OpenSSL 3.0')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1]))
