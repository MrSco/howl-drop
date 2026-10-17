#!/usr/bin/env python3
"""AirDrop what is put in save_folder/send to a nearby iPhone (experimental).

  outbox.py --outbox DIR --name NAME [--tools DIR] [--window SECONDS]

Each file copied into DIR, once it stops growing, is sent with omdrop-awdl's
send-to-peer (shipped in /usr/lib/omdrop with the radio tools), then moved
to DIR/sent, or DIR/failed when it could not be.

The phone has to be set to Everyone for 10 Minutes: in Contacts Only it
takes files only from a sender with an Apple-signed identity, which only
Apple devices have. While it is Everyone, the add-on's BLE advert (zero
hashes, the same one send-to-peer would raise) wakes it onto AWDL, OWL adds
it as a peer, and its AirDrop port opens in bursts; send-to-peer --wait
fires inside one. The phone then asks to accept, like any AirDrop.

The add-on runs as root and has no polkit, so send-to-peer's `pkexec HELPER
peers` goes through a pass-through pkexec on PATH. Several peers (a Mac
nearby, say): each is asked its name and the one called *iPhone* is used.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

SETTLE_SCANS = 2
PEER_RETRY_S = 3
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


def run(cmd, env, timeout):
    """(exit code, output lines) of a send-to-peer run."""
    try:
        done = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                              env=env, timeout=timeout)
    except subprocess.TimeoutExpired:
        return 124, ['timed out']
    return done.returncode, [line for line in done.stdout.splitlines() if line.strip()]


def pick_iphone(tool, name, env):
    """The MAC of the one peer that names itself *iPhone*, or None."""
    code, lines = run([tool, '--list', '--names', '--name', name], env, 90)
    for line in lines:
        say('  ' + line)
    phones = [line.split()[0] for line in lines
              if re.match(r'[0-9a-f]{2}(:[0-9a-f]{2}){5}\s', line) and 'iphone' in line.lower()]
    return phones[0] if len(phones) == 1 else None


def send(path, tool, name, env, window):
    """Send one file; True when the phone took it."""
    deadline = time.monotonic() + window
    mac = None
    waited = False
    while time.monotonic() < deadline:
        cmd = [tool, '--wait', '60', '--op-timeout', '90', '--name', name]
        if mac:
            cmd += ['--mac', mac]
        cmd += ['--', path]
        code, lines = run(cmd, env, 240)
        text = '\n'.join(lines).lower()
        if code == 0:
            for line in lines:
                say('  ' + line)
            return True
        if 'no devices found' in text:
            if not waited:
                say('no iPhone heard yet: set it to Everyone for 10 Minutes and keep it unlocked; waiting')
                waited = True
            time.sleep(PEER_RETRY_S)
            continue
        for line in lines:
            say('  ' + line)
        if 'several peers' in text and not mac:
            mac = pick_iphone(tool, name, env)
            if mac:
                say(f'sending to the iPhone at {mac}')
                continue
            say('several devices heard and none (or more than one) named iPhone')
            return False
        if 'declined' in text:
            return False
        if 'never opened' in text or 'not in the peer table' in text:
            mac = None  # its address rotates; look again
            continue
        return False
    say(f'gave up after {window} s')
    return False


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
    a = ap.parse_args()
    tool = os.path.join(a.tools, 'send-to-peer')
    if not os.path.exists(tool):
        say(f'no {tool}; sending is not available in this image')
        return 1
    env = {**os.environ, 'PATH': shim_dir() + os.pathsep + os.environ.get('PATH', '')}
    os.makedirs(a.outbox, exist_ok=True)
    say(f'files put in {a.outbox} are AirDropped to a nearby iPhone set to Everyone for 10 Minutes')
    sizes = {}
    while True:
        os.makedirs(a.outbox, exist_ok=True)  # the daily purge removes empty folders
        for path in waiting(a.outbox):
            try:
                size = os.path.getsize(path)
            except OSError:
                continue
            prev = sizes.get(path)
            sizes[path] = (size, prev[1] + 1 if prev and prev[0] == size else 0)
            if sizes[path][1] < SETTLE_SCANS:
                continue
            sizes.pop(path, None)
            name = os.path.basename(path)
            say(f'sending {name} ({size} bytes)')
            ok = send(path, tool, a.name, env, a.window)
            where = 'sent' if ok else 'failed'
            try:
                shutil.move(path, free_path(os.path.join(a.outbox, where), name))
            except OSError as e:
                say(f'could not move {name} to {where}: {e}')
            say(f'{name}: {"sent" if ok else "not sent"}; moved to {where}/')
        time.sleep(a.interval)


if __name__ == '__main__':
    sys.exit(main())
