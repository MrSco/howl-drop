"""howl-drop: ask the PC's user before accepting an AirDrop, and report the
upload's progress, for howldrop.exe running the receiver in WSL.

patch_receiver.py puts this beside omdrop-plugin's airdrop-serve.py and
calls it from handle_ask and handle_upload. Nothing here runs unless
howldrop.exe asks for it:

  HOWLDROP_ASK_DIR  set: a sender who isn't a known contact waits for the
                    user. The receiver prints an `ask` event (below) and
                    holds its answer until ID.accept or ID.decline appears
                    in this folder (a Windows folder seen as /mnt/c/...,
                    which has no inotify: polled), or ASK_TIMEOUT_S pass.
  HOWLDROP_EVENTS   stdout: print events, as announce.event does.

Events, tab-separated after @howldrop:
  ask ID SENDER COUNT BYTES NAME...       a sender waits for Accept / Decline
  asked ID accepted|declined|timeout      how it ended
  incoming ID SENDER COUNT BYTES NAME...  an accepted ask: on its way
  receiving ID BYTES TOTAL                the upload so far, at most every 0.25 s
  stored ID NAME...                       the upload's files are written
  failed ID WHY                           the upload broke off
"""
import os
import secrets
import sys
import time

ASK_TIMEOUT_S = 60
POLL_S = 0.2
PROGRESS_EVERY_S = 0.25
NAMES_SHOWN = 20


def event(*fields):
    """As announce.event: one line, fields without tabs or line breaks."""
    if os.environ.get('HOWLDROP_EVENTS') != 'stdout':
        return
    clean = [' '.join(str(f).replace('\t', ' ').splitlines()) for f in fields]
    print('\t'.join(['@howldrop'] + clean), file=sys.stdout, flush=True)


def ask_dir():
    return os.environ.get('HOWLDROP_ASK_DIR') or None


def new_id():
    return secrets.token_hex(6)


def describe(ask):
    """(sender, names, total bytes) of an /Ask plist."""
    sender = str(ask.get('SenderComputerName') or 'Someone')
    files = ask.get('Files') or []
    names = [str(f.get('FileName') or '?') for f in files if isinstance(f, dict)]
    total = 0
    for f in files:
        try:
            total += int(f.get('FileSize') or 0) if isinstance(f, dict) else 0
        except (TypeError, ValueError):
            pass
    if not names:  # a links ask carries its addresses in Items
        names = [str(i) for i in (ask.get('Items') or []) if isinstance(i, str)]
    return sender, names, total


def wait_decision(directory, tid, timeout=ASK_TIMEOUT_S, sleep=time.sleep, clock=time.monotonic):
    """'accepted', 'declined' or 'timeout': the first of ID.accept or
    ID.decline to appear in directory, removed once read."""
    deadline = clock() + timeout
    paths = (('accepted', os.path.join(directory, tid + '.accept')),
             ('declined', os.path.join(directory, tid + '.decline')))
    while True:
        for outcome, path in paths:
            if os.path.exists(path):
                try:
                    os.unlink(path)
                except OSError:
                    pass
                return outcome
        if clock() >= deadline:
            return 'timeout'
        sleep(POLL_S)


def incoming(ask, tid):
    """An accepted ask: what is on its way, for the PC's transfer card."""
    sender, names, total = describe(ask)
    event('incoming', tid, sender, len(names), total, *names[:NAMES_SHOWN])


def ask_user(ask, tid, directory, **wait):
    """Show the ask on the PC and wait: True when the user accepted."""
    sender, names, total = describe(ask)
    event('ask', tid, sender, len(names), total, *names[:NAMES_SHOWN])
    outcome = wait_decision(directory, tid, **wait)
    event('asked', tid, outcome)
    return outcome == 'accepted'


class Progress:
    """The upload's temporary file, counting what is written to it."""

    def __init__(self, target, tid, total, every=PROGRESS_EVERY_S, clock=time.monotonic):
        self.target = target
        self.tid = tid
        self.total = total
        self.every = every
        self.clock = clock
        self.bytes = 0
        self.last = None

    def write(self, data):
        written = self.target.write(data)
        self.bytes += written or 0
        now = self.clock()
        if self.last is None or now - self.last >= self.every:
            self.last = now
            event('receiving', self.tid, self.bytes, self.total)
        return written

    def done(self):
        event('receiving', self.tid, self.bytes, self.total)

    def __getattr__(self, name):  # fileno, flush, tell: the file's own
        return getattr(self.target, name)


def total_of(headers):
    """The upload's size from its TotalBytes header, or 0."""
    try:
        return max(0, int(headers.get('TotalBytes') or 0))
    except (TypeError, ValueError):
        return 0
