#!/usr/bin/env python3
"""The HowlDrop panel in Home Assistant's sidebar (ingress): recent drops in
save_folder with who sent them, thumbnails, links, text, downloads and delete.

  panel.py --dest DIR [--port N] [--bind ADDR] [--history FILE] [--thumbs DIR] [--static DIR]

The add-on runs on the host network, so this server is on the host's
addresses too. It binds the hassio bridge address when there is one and,
wherever it listens, answers only the Supervisor's ingress proxy
(172.30.32.2), which lets through only signed-in Home Assistant users.
HOWLDROP_PANEL_ALLOW (comma-separated addresses and networks such as
192.168.1.0/24, or *) changes that, for a Docker install without Home
Assistant (docker/) or a test.

Anyone in range can send in Everyone mode, so a drop is untrusted: its name
is only ever text in the page (app.js), and a file is served inline only as
a picture, a video or plain text, under a sandbox, and otherwise as a
download. Nothing a phone sends can run as script under Home Assistant's
address. Only files in save_folder and its text/ are listed, served or
deleted.
"""
import argparse
import hashlib
import http.server
import ipaddress
import json
import os
import socketserver
import subprocess
import sys
import threading
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import announce  # noqa: E402
from deliver import url_of_shortcut  # noqa: E402

SUPERVISOR = '172.30.32.2'
HASSIO_GATEWAY = '172.30.32.1'
DEFAULT_PORT = 47640
LIMIT = 300
THUMB_PX = 360
THUMBS_KEPT = 2000
CHUNK = 256 * 1024

# What may be shown in the browser rather than downloaded.
INLINE = {
    '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.png': 'image/png', '.gif': 'image/gif',
    '.webp': 'image/webp', '.heic': 'image/heic', '.heif': 'image/heif',
    '.mp4': 'video/mp4', '.m4v': 'video/mp4', '.mov': 'video/quicktime', '.webm': 'video/webm',
    '.txt': 'text/plain; charset=utf-8',
}
THUMBABLE = {'.jpg', '.jpeg', '.png', '.gif', '.webp', '.bmp', '.tif', '.tiff', '.heic', '.heif'}
# What an iPhone sends a photo as. Only Safari shows it, so the panel shows a
# JPEG made with libheif's heif-convert instead.
HEIF = {'.heic', '.heif'}
VIEWS_KEPT = 200  # full-size JPEGs of HEIC photos, a few MB each
HEIF_TIMEOUT_S = 60
STATIC = {
    '/': ('index.html', 'text/html; charset=utf-8'),
    '/app.js': ('app.js', 'text/javascript; charset=utf-8'),
    '/style.css': ('style.css', 'text/css; charset=utf-8'),
    '/icon.png': ('icon.png', 'image/png'),
}
PAGE_CSP = ("default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; "
            "media-src 'self'; connect-src 'self'; frame-ancestors 'self'; base-uri 'none'; form-action 'none'")
FILE_CSP = "default-src 'none'; img-src 'self'; media-src 'self'; style-src 'unsafe-inline'; sandbox"


# --- what is in save_folder ----------------------------------------------------------

def arrived(st):
    """When a file arrived: the later of its modification and change times
    (deliver.py's purge counts age the same way)."""
    return max(st.st_mtime, st.st_ctime)


def listing(dest, history=None, limit=LIMIT):
    """The newest drops in dest and dest/text, newest first."""
    who = announce.senders(history) if history else {}
    items = []
    for folder in ('', 'text'):
        base = os.path.join(dest, folder)
        try:
            names = os.listdir(base)
        except OSError:
            continue
        for name in names:
            if name.startswith('.') or name.endswith('.partial'):
                continue
            path = os.path.join(base, name)
            try:
                st = os.lstat(path)
            except OSError:
                continue
            if not os.path.isfile(path) or os.path.islink(path):
                continue
            rel = os.path.join(folder, name) if folder else name
            items.append((arrived(st), rel, path, st))
    items.sort(key=lambda item: item[0], reverse=True)
    out = []
    for when, rel, path, st in items[:limit]:
        ext = os.path.splitext(name_of(rel))[1].lower()
        url = url_of_shortcut(path) if ext == '.url' else None
        kind = announce.kind_of(path, url)
        entry = {
            'id': rel.replace(os.sep, '/'),
            'name': name_of(rel),
            'kind': kind,
            'size': st.st_size,
            'time': int(when),
            'version': int(st.st_mtime),
            'sender': who.get(rel, ''),
            'inline': ext in INLINE,
            'thumb': ext in THUMBABLE,
        }
        if url and url.lower().startswith(('http://', 'https://')):
            entry['url'] = url
        if kind == 'text':
            entry['text'] = announce.text_preview(path)
        out.append(entry)
    return out


def name_of(rel):
    return os.path.basename(rel)


def resolve(dest, rel):
    """The file in dest or dest/text that rel names, or None."""
    if not rel or '\0' in rel or rel.startswith(('/', '\\')):
        return None
    parts = rel.replace('\\', '/').split('/')
    if len(parts) == 2 and parts[0] == 'text':
        folder, name = 'text', parts[1]
    elif len(parts) == 1:
        folder, name = '', parts[0]
    else:
        return None
    if name in ('', '.', '..') or name.startswith('.'):
        return None
    base = os.path.realpath(os.path.join(dest, folder))
    path = os.path.join(base, name)
    if os.path.islink(path) or not os.path.isfile(path):
        return None
    if os.path.dirname(os.path.realpath(path)) != base:
        return None
    return path


# --- thumbnails ----------------------------------------------------------------------

def cache_key(path):
    st = os.stat(path)
    return hashlib.sha256(f'{path}\0{st.st_mtime_ns}\0{st.st_size}'.encode()).hexdigest()[:32]


def temp_name(out):
    return f'{out}.{os.getpid()}.{threading.get_ident()}.new'


def heif_view(path, cache):
    """A full-size JPEG of a HEIC photo, made once and kept in cache; None
    when it cannot be made. libheif applies the photo's own rotation while
    decoding, so the JPEG is saved without EXIF: no orientation tag can turn
    it a second time, and the photo's location stays in the original."""
    try:
        out = os.path.join(cache, cache_key(path) + '-view.jpg')
    except OSError:
        return None
    if os.path.exists(out):
        return out
    try:
        from PIL import Image
    except ImportError:
        return None
    tmp = temp_name(out)
    made = tmp + '.jpg'
    try:
        os.makedirs(cache, exist_ok=True)
        subprocess.run(['heif-convert', '-q', '92', path, made], stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, timeout=HEIF_TIMEOUT_S, check=True)
        # A file holding several images is written as NAME-1.jpg, NAME-2.jpg...:
        # the first is the photo.
        if not os.path.exists(made):
            made = tmp + '-1.jpg'
        with Image.open(made) as img:
            img = img.convert('RGB')
            img.save(tmp, 'JPEG', quality=90)
        os.replace(tmp, out)
    except (OSError, ValueError, subprocess.SubprocessError):
        return None
    finally:
        for leftover in (tmp, tmp + '.jpg', *(f'{tmp}-{i}.jpg' for i in range(1, 9))):
            try:
                os.unlink(leftover)
            except OSError:
                pass
    prune_cache(cache, '-view.jpg', VIEWS_KEPT)
    return out


def thumbnail(path, cache):
    """A small JPEG of a picture, made once and kept in cache; None when it
    cannot be made (no Pillow, or not a picture it reads)."""
    try:
        out = os.path.join(cache, cache_key(path) + '.jpg')
    except OSError:
        return None
    if os.path.exists(out):
        return out
    try:
        from PIL import Image, ImageOps
    except ImportError:
        return None
    heif = os.path.splitext(path)[1].lower() in HEIF
    source = heif_view(path, cache) if heif else path
    if not source:
        return None
    tmp = temp_name(out)
    try:
        os.makedirs(cache, exist_ok=True)
        with Image.open(source) as img:
            if not heif:  # heif_view has turned it already
                img = ImageOps.exif_transpose(img)
            img.thumbnail((THUMB_PX, THUMB_PX))
            if img.mode not in ('RGB', 'L'):
                img = img.convert('RGB')
            img.save(tmp, 'JPEG', quality=82)
        os.replace(tmp, out)
    except Exception:  # noqa: BLE001 - any file a phone sends; a bad one gets no thumbnail
        try:
            os.unlink(tmp)
        except OSError:
            pass
        return None
    prune_cache(cache, '.jpg', THUMBS_KEPT, skip='-view.jpg')
    return out


def prune_cache(cache, suffix, keep, skip=None):
    """Keep the newest `keep` files in cache ending in suffix (not in skip)."""
    try:
        names = [os.path.join(cache, n) for n in os.listdir(cache)
                 if n.endswith(suffix) and not (skip and n.endswith(skip))]
    except OSError:
        return
    if len(names) <= keep:
        return
    names.sort(key=lambda p: os.path.getmtime(p) if os.path.exists(p) else 0)
    for old in names[:len(names) - keep]:
        try:
            os.unlink(old)
        except OSError:
            pass


# --- the server ----------------------------------------------------------------------

def disposition(kind, name):
    ascii_name = ''.join(c if c.isascii() and c.isprintable() and c not in '"\\;' else '_' for c in name)
    return f'{kind}; filename="{ascii_name}"; filename*=UTF-8\'\'{urllib.parse.quote(name, safe="")}'


class Handler(http.server.BaseHTTPRequestHandler):
    server_version = 'HowlDrop'
    sys_version = ''
    protocol_version = 'HTTP/1.1'

    def log_message(self, fmt, *args):  # the add-on log is for the radio
        pass

    def allowed(self):
        try:
            peer = ipaddress.ip_address(self.client_address[0])
        except ValueError:
            return False
        return any(peer in network for network in self.server.allow)

    def send_plain(self, code, text):
        body = (text + '\n').encode()
        self.send_response(code)
        self.send_header('Content-Type', 'text/plain; charset=utf-8')
        self.send_header('Content-Length', str(len(body)))
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.end_headers()
        if self.command != 'HEAD':
            self.wfile.write(body)

    def send_json(self, data):
        body = json.dumps(data).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Cache-Control', 'no-store')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.end_headers()
        self.wfile.write(body)

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        if not self.allowed():
            return self.send_plain(403, 'forbidden')
        url = urllib.parse.urlsplit(self.path)
        query = urllib.parse.parse_qs(url.query)
        if url.path in STATIC:
            return self.send_static(*STATIC[url.path])
        if url.path == '/api/drops':
            return self.send_json({'drops': listing(self.server.dest, self.server.history),
                                   'folder': self.server.dest})
        if url.path in ('/file', '/thumb'):
            path = resolve(self.server.dest, (query.get('p') or [''])[0])
            if not path:
                return self.send_plain(404, 'not found')
            if url.path == '/thumb':
                small = thumbnail(path, self.server.thumbs)
                if not small:
                    return self.send_plain(404, 'no thumbnail')
                return self.send_file(small, 'image/jpeg', 'inline', name_of(path))
            ext = os.path.splitext(path)[1].lower()
            if ext in HEIF and not query.get('download'):
                view = heif_view(path, self.server.thumbs)
                if view:
                    stem = os.path.splitext(name_of(path))[0]
                    return self.send_file(view, 'image/jpeg', 'inline', stem + '.jpg')
            if ext in INLINE and not query.get('download'):
                return self.send_file(path, INLINE[ext], 'inline', name_of(path))
            return self.send_file(path, 'application/octet-stream', 'attachment', name_of(path))
        return self.send_plain(404, 'not found')

    def do_POST(self):
        if not self.allowed():
            return self.send_plain(403, 'forbidden')
        # JSON only: a page elsewhere cannot send it here without the browser
        # asking first, so no other site can delete through a signed-in user.
        if self.headers.get('Content-Type', '').split(';')[0].strip() != 'application/json':
            return self.send_plain(415, 'JSON only')
        try:
            length = int(self.headers.get('Content-Length') or 0)
            body = json.loads(self.rfile.read(min(length, 65536)) or b'{}')
        except ValueError:
            return self.send_plain(400, 'bad request')
        if urllib.parse.urlsplit(self.path).path != '/api/delete' or not isinstance(body, dict):
            return self.send_plain(404, 'not found')
        ids = body.get('ids') if isinstance(body.get('ids'), list) else [body.get('id')]
        deleted = 0
        for rel in ids[:LIMIT]:
            path = resolve(self.server.dest, rel if isinstance(rel, str) else '')
            if path:
                try:
                    os.unlink(path)
                    deleted += 1
                except OSError:
                    pass
        return self.send_json({'deleted': deleted})

    def send_static(self, name, ctype):
        path = os.path.join(self.server.static, name)
        try:
            with open(path, 'rb') as fh:
                body = fh.read()
        except OSError:
            return self.send_plain(404, 'not found')
        self.send_response(200)
        self.send_header('Content-Type', ctype)
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Cache-Control', 'no-cache')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.send_header('Content-Security-Policy', PAGE_CSP)
        self.send_header('Referrer-Policy', 'no-referrer')
        self.end_headers()
        if self.command != 'HEAD':
            self.wfile.write(body)

    def send_file(self, path, ctype, kind, name):
        try:
            fh = open(path, 'rb')
        except OSError:
            return self.send_plain(404, 'not found')
        with fh:
            size = os.fstat(fh.fileno()).st_size
            start, end = 0, size - 1
            ranged = self.headers.get('Range', '')
            partial = False
            if ranged.startswith('bytes=') and ',' not in ranged and size > 0:
                first, _, last = ranged[6:].strip().partition('-')
                try:
                    if first:
                        start = int(first)
                        end = min(int(last), size - 1) if last else size - 1
                    elif last:
                        start = max(size - int(last), 0)
                    partial = start <= end and start < size
                except ValueError:
                    partial = False
                if not partial:
                    self.send_response(416)
                    self.send_header('Content-Range', f'bytes */{size}')
                    self.send_header('Content-Length', '0')
                    self.end_headers()
                    return
            self.send_response(206 if partial else 200)
            self.send_header('Content-Type', ctype)
            self.send_header('Content-Length', str(end - start + 1 if size else 0))
            self.send_header('Accept-Ranges', 'bytes')
            if partial:
                self.send_header('Content-Range', f'bytes {start}-{end}/{size}')
            self.send_header('Content-Disposition', disposition(kind, name))
            self.send_header('Content-Security-Policy', FILE_CSP)
            self.send_header('X-Content-Type-Options', 'nosniff')
            self.send_header('Cache-Control', 'private, max-age=86400')
            self.send_header('Referrer-Policy', 'no-referrer')
            self.end_headers()
            if self.command == 'HEAD' or not size:
                return
            fh.seek(start)
            left = end - start + 1
            while left > 0:
                chunk = fh.read(min(CHUNK, left))
                if not chunk:
                    break
                self.wfile.write(chunk)
                left -= len(chunk)


class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def parse_allow(entries):
    """Addresses and networks (192.168.1.0/24), or * for any, as networks.
    An entry that is neither is left out."""
    networks = []
    for entry in entries:
        entry = entry.strip()
        if entry == '*':
            networks += [ipaddress.ip_network('0.0.0.0/0'), ipaddress.ip_network('::/0')]
            continue
        try:
            networks.append(ipaddress.ip_network(entry, strict=False))
        except ValueError:
            print(f'HOWLDROP_PANEL_ALLOW: {entry!r} is not an address or network; left out', flush=True)
    return networks


def serve(dest, port, history, thumbs, static, allow, hosts=(HASSIO_GATEWAY, '0.0.0.0')):
    """Listen on the first of hosts that can be bound; return the server.
    allow: the addresses and networks it answers (parse_allow)."""
    last = None
    for host in hosts:
        try:
            server = Server((host, port), Handler)
            break
        except OSError as e:
            last = e
    else:
        raise last
    server.dest = os.path.realpath(dest)
    server.history = history
    server.thumbs = thumbs
    server.static = static
    server.allow = parse_allow(allow)
    return server


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--dest', required=True, help='save_folder')
    ap.add_argument('--port', type=int, default=DEFAULT_PORT)
    ap.add_argument('--bind', help='address to listen on (default: the hassio bridge, else all)')
    ap.add_argument('--history', default='/data/drops.jsonl')
    ap.add_argument('--thumbs', default='/data/thumbs')
    ap.add_argument('--static', default=os.path.join(os.path.dirname(os.path.abspath(__file__)), 'panel'))
    a = ap.parse_args()
    allow = [s.strip() for s in os.environ.get('HOWLDROP_PANEL_ALLOW', SUPERVISOR).split(',') if s.strip()]
    os.makedirs(a.dest, exist_ok=True)
    hosts = (a.bind,) if a.bind else (HASSIO_GATEWAY, '0.0.0.0')
    server = serve(a.dest, a.port, a.history, a.thumbs, a.static, allow, hosts)
    host, port = server.server_address[:2]
    print(f'panel on {host}:{port} for {", ".join(sorted(allow))}', flush=True)
    server.serve_forever()


if __name__ == '__main__':
    sys.exit(main())
