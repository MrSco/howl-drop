#!/usr/bin/env python3
"""AirDrop what is put in save_folder/send to a nearby iPhone (experimental).

  outbox.py --outbox DIR --name NAME [--tools DIR] [--window SECONDS] [--status FILE]

Each file in DIR, once it stops growing, is sent with omdrop-awdl's
send-to-peer (shipped in /usr/lib/omdrop with the radio tools), then moved
to DIR/sent, or DIR/failed when it could not be. Files get there by Samba,
from the panel's Send to iPhone (panel.py), or from a paired PC through the
relay (howldrop-bridge relay). --status keeps the latest sends, with how
far each got, as JSON for the panel.

The phone has to be set to Everyone for 10 Minutes: in Contacts Only it
takes files only from a sender with an Apple-signed identity, which only
Apple devices have. While it is Everyone, the add-on's BLE advert (zero
hashes, the same one send-to-peer would raise) wakes it onto AWDL, OWL adds
it as a peer, and its AirDrop port opens in bursts; send-to-peer --wait
fires inside one. The phone then asks to accept, like any AirDrop. First
landed 2026-10-16: /Discover named the phone, /Ask was accepted after 4 s,
and a 2.6 MB picture uploaded in 4 s.

The add-on runs as root and has no polkit, so send-to-peer's `pkexec HELPER
peers` goes through a pass-through pkexec on PATH. Which phone: --to (the
send_to option) names it, and only the device answering to that name gets
the file. Without it: the only device heard, or of several (a Mac nearby,
say) the one whose name says iPhone.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time

SETTLE_SCANS = 2
PEER_RETRY_S = 3
STATUS_KEPT = 30
SHIM = '#!/bin/sh\n# send-to-peer runs `pkexec HELPER peers`; the add-on is root already.\nexec "$@"\n'


def say(*parts):
    print(*parts, flush=True)


def shim_dir():
    d = tempfile.mkdtemp(prefix='howldrop-pkexec-')
    path = os.path.join(d, 'pkexec')
    with open(path, 'w') as fh:
        fh.write(SHIM)
    os.chmod(path, 0o755)
    return d


def ensure_identity(tools, name, home='/root', store='/data/omdrop-sender'):
    """Make the sender's self-signed identity, kept in store, before any send.

    airdrop-send.py presents the pair in ~/.omdrop/keys. Run as root (the
    add-on always is) it will not create one there and fails with
    `identity error: self-signed-missing` (2026-10-16), so it is made here,
    with omdrop-awdl's own function, and ~/.omdrop points at store so it
    survives a restart. Returns an error to log, or None.
    """
    keys = os.path.join(store, 'keys')
    link = os.path.join(home, '.omdrop')
    try:
        os.makedirs(keys, mode=0o700, exist_ok=True)
        if not os.path.lexists(link):
            os.symlink(store, link)
        if os.path.realpath(link) != os.path.realpath(store):
            return f'{link} is not {store}; the sender will use its own'
        sys.path.insert(0, tools)
        try:
            import awdl_identity
        finally:
            sys.path.remove(tools)
        from pathlib import Path
        awdl_identity._create_self_signed(Path(keys), name)
    except Exception as e:  # noqa: BLE001 - logged, and the send then says why
        return f'could not make the sender identity: {e}'
    return None


# --- what the panel shows ---------------------------------------------------------

class Status:
    """The latest sends, newest first, written to path as JSON for the panel."""

    def __init__(self, path):
        self.path = path
        self.jobs = []
        self.lock = threading.Lock()
        self.serial = 0

    def add(self, name):
        with self.lock:
            self.serial += 1
            job = {'id': f'{int(time.time())}-{self.serial}', 'name': name, 'state': 'waiting',
                   'detail': 'waiting to send', 'time': int(time.time())}
            self.jobs.insert(0, job)
            del self.jobs[STATUS_KEPT:]
        self.write()
        return job

    def update(self, job, state=None, detail=None):
        with self.lock:
            if state:
                job['state'] = state
            if detail:
                job['detail'] = detail
            job['time'] = int(time.time())
        self.write()

    def write(self):
        if not self.path:
            return
        try:
            with self.lock:
                text = json.dumps({'jobs': self.jobs})
            with open(self.path + '.new', 'w', encoding='utf-8') as fh:
                fh.write(text)
            os.replace(self.path + '.new', self.path)
        except OSError:
            pass


# --- one send ------------------------------------------------------------------------

def useful(line):
    """The lines worth logging: not the sender's debug detail."""
    return not (' DEBUG ' in line or 'DeprecationWarning' in line or line.lstrip().startswith('ctx.'))


def stage_of(line):
    """What a sender line says about how far the send got, or None."""
    if 'waiting up to' in line:
        return 'waiting for the iPhone to listen'
    if ' FOUND ' in line:
        return 'asking: accept it on the iPhone'
    if 'ASK -> accepted' in line:
        return 'accepted; uploading'
    return None


def run(cmd, env, timeout, on_line=None):
    """(exit code, output lines) of a send-to-peer run, each line also given
    to on_line as it comes."""
    try:
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
    except OSError as e:
        return 127, [str(e)]
    timer = threading.Timer(timeout, proc.kill)
    timer.start()
    lines = []
    try:
        for raw in proc.stdout:
            line = raw.rstrip('\n')
            if not line.strip():
                continue
            lines.append(line)
            if on_line:
                on_line(line)
        code = proc.wait()
    finally:
        timer.cancel()
    if code < 0:
        return 124, lines + ['timed out']
    return code, lines


def pick_iphone(tool, name, env):
    """The MAC of the one peer that names itself *iPhone*, or None."""
    code, lines = run([tool, '--list', '--names', '--name', name], env, 90)
    for line in lines:
        say('  ' + line)
    phones = [line.split()[0] for line in lines
              if re.match(r'[0-9a-f]{2}(:[0-9a-f]{2}){5}\s', line) and 'iphone' in line.lower()]
    return phones[0] if len(phones) == 1 else None


def send(path, tool, name, env, window, progress=None, to=None):
    """Send one file. (True, detail) when the phone took it, else (False, why).
    progress(detail) hears how far it has got. to: the phone's name (part of
    it will do); only the device that answers to it is sent to. Without it,
    the only device heard, or of several the one called *iPhone*."""
    progress = progress or (lambda detail: None)
    deadline = time.monotonic() + window
    mac = None
    waited = False

    def on_line(line):
        if useful(line):
            say('  ' + line)
        stage = stage_of(line)
        if stage:
            progress(stage)

    # iOS names a phone "Rocco’s iPhone", with a curly apostrophe, and the
    # name is matched as written: a typed straight one is tried both ways.
    names = [to] if to else []
    if to and ("'" in to or '’' in to):
        names.append(to.replace("'", '’') if "'" in to else to.replace('’', "'"))
    attempt = 0
    while time.monotonic() < deadline:
        cmd = [tool, '--wait', '60', '--op-timeout', '90', '--name', name]
        if names:
            cmd += ['--mac', names[attempt % len(names)]]
            attempt += 1
        elif mac:
            cmd += ['--mac', mac]
        cmd += ['--', path]
        code, lines = run(cmd, env, 240, on_line)
        text = '\n'.join(lines).lower()
        if code == 0:
            return True, 'on the iPhone'
        if 'no devices found' in text:
            if not waited:
                say('no iPhone heard yet: set it to Everyone for 10 Minutes and keep it unlocked; waiting')
                progress('no iPhone heard yet: set AirDrop to Everyone for 10 Minutes')
                waited = True
            time.sleep(PEER_RETRY_S)
            continue
        if to and 'no device answering to' in text:
            # Named only while it listens: keep asking until the deadline.
            if not waited:
                say(f'{to} is not answering yet: set its AirDrop to Everyone for 10 Minutes; waiting')
                progress(f'waiting for {to}: set its AirDrop to Everyone for 10 Minutes')
                waited = True
            time.sleep(PEER_RETRY_S)
            continue
        if to and 'matches more than one device' in text:
            return False, f'more than one device answers to "{to}": use more of its name'
        if 'several peers' in text and not mac:
            mac = pick_iphone(tool, name, env)
            if mac:
                say(f'sending to the iPhone at {mac}')
                continue
            return False, 'several devices nearby, and none (or more than one) called iPhone'
        if 'declined' in text:
            return False, 'declined on the iPhone'
        if 'never opened' in text or 'not in the peer table' in text:
            mac = None  # its address rotates; look again
            continue
        last = next((line for line in reversed(lines) if useful(line)), f'the sender exited {code}')
        return False, last.strip()
    say(f'gave up after {window} s')
    return False, f'the iPhone did not answer within {window // 60} minutes: is AirDrop set to Everyone?'


# --- the folder -------------------------------------------------------------------

def free_path(directory, name):
    os.makedirs(directory, exist_ok=True)
    stem, ext = os.path.splitext(name)
    path = os.path.join(directory, name)
    n = 1
    while os.path.exists(path):
        path = os.path.join(directory, f'{stem}-{n}{ext}')
        n += 1
    return path


def waiting(outbox):
    try:
        names = sorted(os.listdir(outbox))
    except OSError:
        return []
    return [os.path.join(outbox, n) for n in names
            if not n.startswith('.') and not n.endswith('.partial') and os.path.isfile(os.path.join(outbox, n))]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--outbox', required=True)
    ap.add_argument('--name', default='HowlDrop', help='the name the phone shows for this sender')
    ap.add_argument('--tools', default='/usr/lib/omdrop')
    ap.add_argument('--window', type=int, default=180, help='seconds to keep trying one file')
    ap.add_argument('--interval', type=float, default=1.0)
    ap.add_argument('--status', help='where to keep the latest sends as JSON, for the panel')
    ap.add_argument('--to', default='', help="the iPhone's name (send_to); empty: pick on its own")
    a = ap.parse_args()
    tool = os.path.join(a.tools, 'send-to-peer')
    if not os.path.exists(tool):
        say(f'no {tool}; sending is not available in this image')
        return 1
    env = {**os.environ, 'PATH': shim_dir() + os.pathsep + os.environ.get('PATH', '')}
    problem = ensure_identity(a.tools, a.name)
    if problem:
        say(problem)
    os.makedirs(a.outbox, exist_ok=True)
    status = Status(a.status)
    status.write()
    say(f'files put in {a.outbox} are AirDropped to a nearby iPhone set to Everyone for 10 Minutes')
    sizes = {}
    jobs = {}
    while True:
        os.makedirs(a.outbox, exist_ok=True)  # the daily purge removes empty folders
        for path in waiting(a.outbox):
            try:
                size = os.path.getsize(path)
            except OSError:
                continue
            if path not in jobs:
                jobs[path] = status.add(os.path.basename(path))
            prev = sizes.get(path)
            sizes[path] = (size, prev[1] + 1 if prev and prev[0] == size else 0)
            if sizes[path][1] < SETTLE_SCANS:
                continue
            sizes.pop(path, None)
            job = jobs.pop(path)
            name = os.path.basename(path)
            say(f'sending {name} ({size} bytes)')
            status.update(job, 'sending', 'looking for the iPhone')
            ok, detail = send(path, tool, a.name, env, a.window, lambda d, job=job: status.update(job, detail=d),
                              a.to.strip() or None)
            where = 'sent' if ok else 'failed'
            try:
                shutil.move(path, free_path(os.path.join(a.outbox, where), name))
            except OSError as e:
                say(f'could not move {name} to {where}: {e}')
            status.update(job, where, detail)
            say(f'{name}: {"sent" if ok else "not sent: " + detail}; moved to {where}/')
        time.sleep(a.interval)


if __name__ == '__main__':
    sys.exit(main())
