#!/usr/bin/env python3
"""Move what the AirDrop receiver saves into save_folder, fetching linked files.

The receiver writes into a staging folder; this moves each item to the share
once it stops growing, so the share and the forwarder only ever see finished
files. A link the phone sends is stored by the receiver as an inert .url
shortcut (it never fetches anything). From Safari or Chrome, sharing a picture
you are looking at sends its address, not its bytes: the sheet says "PNG
Image, 166 KB" and the receiver gets `type links` (2026-10-14). So a .url
whose address serves a file (anything but a web page) is fetched, and the
file takes the shortcut's place; a web page stays a shortcut.

Everyone mode lets any device in range send a link, so fetching is fenced:
http and https only, every hop of the request (redirects too) must resolve to
public addresses only (no LAN, loopback or link-local, so no reaching Home
Assistant or the router), a size cap, and a timeout. A refused or failed fetch
leaves the shortcut.

  deliver.py --staging DIR --dest DIR [--no-fetch] [--max-mb N]
"""
import argparse
import configparser
import ipaddress
import mimetypes
import os
import re
import shutil
import socket
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

PAGE_TYPES = ('text/html', 'application/xhtml+xml')
MAX_REDIRECTS = 5
TIMEOUT_S = 30
CHUNK = 64 * 1024


def say(*parts):
    print(*parts, flush=True)


class Refused(Exception):
    """A fetch this script will not make, or a reply it will not keep."""


def public_host(host, allow_private=False):
    """Every address host resolves to is a public one, or Refused."""
    if not host:
        raise Refused('no host')
    try:
        infos = socket.getaddrinfo(host, None)
    except OSError as e:
        raise Refused(f'{host} does not resolve ({e})')
    for info in infos:
        addr = ipaddress.ip_address(info[4][0].split('%', 1)[0])
        if not allow_private and not addr.is_global:
            raise Refused(f'{host} resolves to {addr}, not a public address')
    return True


class CheckedRedirects(urllib.request.HTTPRedirectHandler):
    """Follow at most MAX_REDIRECTS, each to http(s) on a public host."""

    def __init__(self, allow_private):
        super().__init__()
        self.allow_private = allow_private
        self.hops = 0

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        self.hops += 1
        if self.hops > MAX_REDIRECTS:
            raise Refused('too many redirects')
        parts = urllib.parse.urlsplit(newurl)
        if parts.scheme not in ('http', 'https'):
            raise Refused(f'redirect to {parts.scheme}:')
        public_host(parts.hostname, self.allow_private)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def url_of_shortcut(path):
    """The URL= line of a .url shortcut ([InternetShortcut]), or None."""
    parser = configparser.ConfigParser(interpolation=None)
    try:
        parser.read(path, encoding='utf-8')
        return parser.get('InternetShortcut', 'URL', fallback=None)
    except (configparser.Error, OSError, UnicodeDecodeError):
        return None


def safe_name(name, fallback):
    """A plain file name: the last path segment, no control characters, no
    leading dot (the forwarder skips dotfiles), at most 200 characters."""
    name = urllib.parse.unquote(name or '').replace('\\', '/').rsplit('/', 1)[-1]
    name = re.sub(r'[\x00-\x1f\x7f<>:"|?*]', '_', name).strip(' .')
    return (name or fallback)[:200]


def name_for(response, url):
    """Content-Disposition's filename, else the URL's last segment; given an
    extension from the content type when it has none."""
    name = None
    disposition = response.headers.get('Content-Disposition', '')
    m = re.search(r"filename\*=(?:UTF-8'')?([^;]+)", disposition, re.I) or \
        re.search(r'filename="?([^";]+)"?', disposition, re.I)
    if m:
        name = m.group(1)
    if not name:
        name = urllib.parse.urlsplit(url).path.rstrip('/').rsplit('/', 1)[-1]
    name = safe_name(name, 'download')
    if not os.path.splitext(name)[1]:
        ctype = response.headers.get_content_type()
        ext = mimetypes.guess_extension(ctype) or ''
        name += ext
    return name


def free_path(directory, name):
    """directory/name, or name-1, name-2... when that is taken."""
    stem, ext = os.path.splitext(name)
    for i in range(10000):
        cand = os.path.join(directory, name if i == 0 else f'{stem}-{i}{ext}')
        if not os.path.exists(cand):
            return cand
    raise FileExistsError(name)


def fetch(url, dest_dir, max_bytes, allow_private=False):
    """Download url into dest_dir when it serves a file; return the path.
    Refused for anything this script won't fetch or keep."""
    parts = urllib.parse.urlsplit(url)
    if parts.scheme not in ('http', 'https'):
        raise Refused(f'{parts.scheme or "no"} scheme')
    public_host(parts.hostname, allow_private)
    opener = urllib.request.build_opener(CheckedRedirects(allow_private))
    req = urllib.request.Request(url, headers={'User-Agent': 'OwlDrop/1.0'})
    try:
        response = opener.open(req, timeout=TIMEOUT_S)
    except urllib.error.URLError as e:
        reason = getattr(e, 'reason', e)
        if isinstance(reason, Refused):
            raise reason
        raise Refused(f'fetch failed ({reason})')
    with response:
        ctype = response.headers.get_content_type()
        if ctype in PAGE_TYPES:
            raise Refused(f'a web page ({ctype})')
        length = response.headers.get('Content-Length')
        if length and length.isdigit() and int(length) > max_bytes:
            raise Refused(f'{int(length)} bytes, over the {max_bytes}-byte cap')
        final = free_path(dest_dir, name_for(response, response.geturl()))
        temp = os.path.join(dest_dir, '.' + os.path.basename(final) + '.partial')
        got = 0
        try:
            with open(temp, 'wb') as out:
                while True:
                    block = response.read(CHUNK)
                    if not block:
                        break
                    got += len(block)
                    if got > max_bytes:
                        raise Refused(f'over the {max_bytes}-byte cap')
                    out.write(block)
            os.replace(temp, final)
        except BaseException:
            try:
                os.unlink(temp)
            except FileNotFoundError:
                pass
            raise
    return final


def move_in(src, dest_dir):
    """Move src into dest_dir under a free name, atomically from the share's
    side: copied to a dotfile there, then renamed."""
    os.makedirs(dest_dir, exist_ok=True)
    final = free_path(dest_dir, os.path.basename(src))
    temp = os.path.join(dest_dir, '.' + os.path.basename(final) + '.partial')
    shutil.copyfile(src, temp)
    os.replace(temp, final)
    os.unlink(src)
    return final


def deliver(path, staging, dest, fetch_links, max_bytes, allow_private=False):
    """Deliver one settled staging file; return a line for the log."""
    rel = os.path.relpath(path, staging)
    target_dir = os.path.join(dest, os.path.dirname(rel))
    if fetch_links and path.endswith('.url'):
        url = url_of_shortcut(path)
        if url:
            try:
                got = fetch(url, target_dir if os.path.isdir(target_dir) else dest, max_bytes, allow_private)
                os.unlink(path)
                return f'fetched {os.path.basename(got)} ({os.path.getsize(got)} bytes) from {url}'
            except Refused as e:
                kept = move_in(path, target_dir)
                return f'kept {os.path.basename(kept)}: not fetched, {e}'
    kept = move_in(path, target_dir)
    return f'delivered {os.path.relpath(kept, dest)}'


def staged_files(staging):
    for root, dirs, files in os.walk(staging):
        dirs[:] = [d for d in dirs if not d.startswith('.')]
        for name in files:
            if name.startswith('.') or name.endswith('.partial'):
                continue
            yield os.path.join(root, name)


def run(staging, dest, fetch_links, max_bytes, interval=0.5, once=False, allow_private=False):
    """Deliver each staging file once its size has held for two scans."""
    os.makedirs(dest, exist_ok=True)
    sizes = {}
    while True:
        seen = set()
        for path in staged_files(staging):
            seen.add(path)
            try:
                size = os.path.getsize(path)
            except OSError:
                continue
            prev = sizes.get(path)
            sizes[path] = (size, (prev[1] + 1) if prev and prev[0] == size else 0)
            if sizes[path][1] >= 2:
                try:
                    say(deliver(path, staging, dest, fetch_links, max_bytes, allow_private))
                except OSError as e:
                    say(f'could not deliver {os.path.relpath(path, staging)}: {e}')
                sizes.pop(path, None)
        for gone in set(sizes) - seen:
            sizes.pop(gone, None)
        if once and not sizes:
            return
        time.sleep(interval)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--staging', required=True)
    ap.add_argument('--dest', required=True)
    ap.add_argument('--no-fetch', action='store_true', help='leave links as .url shortcuts')
    ap.add_argument('--max-mb', type=int, default=100, help='largest linked file fetched (MB)')
    a = ap.parse_args()
    say(f'delivering {a.staging} to {a.dest}; links to files are '
        f'{"left as shortcuts" if a.no_fetch else f"fetched (public hosts only, up to {a.max_mb} MB)"}')
    run(a.staging, a.dest, not a.no_fetch, a.max_mb * 1024 * 1024)


if __name__ == '__main__':
    sys.exit(main())
