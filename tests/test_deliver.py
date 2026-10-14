"""addon/deliver.py against a local web server: links to files are fetched,
web pages and refused fetches stay shortcuts, other files are moved."""
import http.server
import importlib.util
import os
from pathlib import Path
import tempfile
import threading
import unittest

SPEC = importlib.util.spec_from_file_location(
    'deliver', Path(__file__).resolve().parents[1] / 'addon' / 'deliver.py')
deliver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(deliver)

PNG = b'\x89PNG\r\n\x1a\n' + bytes(range(256)) * 4


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        routes = {
            '/img/sunset.png': ('image/png', PNG),
            '/page': ('text/html; charset=utf-8', b'<html>hi</html>'),
            '/big.bin': ('application/octet-stream', b'x' * 5000),
            '/noext': ('image/jpeg', b'\xff\xd8\xff' + b'j' * 100),
        }
        if self.path == '/moved':
            self.send_response(302)
            self.send_header('Location', '/img/sunset.png')
            self.end_headers()
            return
        ctype, body = routes.get(self.path, ('text/plain', b''))
        self.send_response(200 if self.path in routes else 404)
        self.send_header('Content-Type', ctype)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class DeliverTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        cls.base = f'http://127.0.0.1:{cls.server.server_address[1]}'
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.staging = Path(self.tmp.name, 'staging')
        self.dest = Path(self.tmp.name, 'share')
        (self.staging / 'text').mkdir(parents=True)

    def tearDown(self):
        self.tmp.cleanup()

    def shortcut(self, name, url):
        (self.staging / name).write_text(f'[InternetShortcut]\nURL={url}\n', encoding='utf-8')

    def run_once(self, allow_private, max_bytes=1024 * 1024):
        deliver.run(str(self.staging), str(self.dest), True, max_bytes, interval=0, once=True,
                    allow_private=allow_private)

    def listing(self):
        return sorted(str(p.relative_to(self.dest)).replace('\\', '/') for p in self.dest.rglob('*') if p.is_file())

    def test_a_local_address_is_refused_by_default(self):
        self.shortcut('a.url', self.base + '/img/sunset.png')
        self.run_once(allow_private=False)
        self.assertEqual(self.listing(), ['a.url'])

    def test_a_link_to_a_file_becomes_the_file(self):
        self.shortcut('www.example-sunset.png.url', self.base + '/img/sunset.png')
        self.run_once(allow_private=True)
        self.assertEqual(self.listing(), ['sunset.png'])
        self.assertEqual((self.dest / 'sunset.png').read_bytes(), PNG)
        self.assertEqual(list(self.staging.glob('*.url')), [])

    def test_a_redirect_is_followed(self):
        self.shortcut('moved.url', self.base + '/moved')
        self.run_once(allow_private=True)
        self.assertEqual(self.listing(), ['sunset.png'])

    def test_a_web_page_stays_a_shortcut(self):
        self.shortcut('page.url', self.base + '/page')
        self.run_once(allow_private=True)
        self.assertEqual(self.listing(), ['page.url'])

    def test_a_file_over_the_cap_stays_a_shortcut(self):
        self.shortcut('big.url', self.base + '/big.bin')
        self.run_once(allow_private=True, max_bytes=1000)
        self.assertEqual(self.listing(), ['big.url'])
        self.assertEqual([p for p in self.dest.iterdir() if p.name.endswith('.partial')], [])

    def test_a_name_without_extension_gets_one_from_the_type(self):
        self.shortcut('n.url', self.base + '/noext')
        self.run_once(allow_private=True)
        listing = self.listing()
        self.assertEqual(len(listing), 1)
        self.assertTrue(listing[0].startswith('noext.') and listing[0] != 'noext.', listing)

    def test_files_and_text_are_moved_and_names_kept_apart(self):
        (self.staging / 'photo.heic').write_bytes(b'heic')
        (self.staging / 'text' / 'note.txt').write_text('hello', encoding='utf-8')
        (self.dest / 'photo.heic').parent.mkdir(parents=True, exist_ok=True)
        (self.dest / 'photo.heic').write_bytes(b'older')
        self.run_once(allow_private=True)
        self.assertEqual(self.listing(), ['photo-1.heic', 'photo.heic', 'text/note.txt'])
        self.assertEqual((self.dest / 'photo-1.heic').read_bytes(), b'heic')
        self.assertEqual(sorted(p.name for p in self.staging.rglob('*') if p.is_file()), [])

    def test_non_http_schemes_are_refused(self):
        with self.assertRaises(deliver.Refused):
            deliver.fetch('file:///etc/passwd', str(self.dest), 1000)

    def test_a_redirect_to_a_private_address_is_refused(self):
        hop = deliver.CheckedRedirects(allow_private=False)
        for url in ('http://192.168.1.1/admin', 'http://127.0.0.1:8123/api', 'ftp://example.com/x'):
            with self.assertRaises(deliver.Refused, msg=url):
                hop.redirect_request(None, None, 302, 'Found', {}, url)

    def test_private_and_loopback_hosts_are_refused(self):
        for host in ('127.0.0.1', '192.168.1.1', '10.0.0.5', '169.254.1.1', 'localhost'):
            with self.assertRaises(deliver.Refused, msg=host):
                deliver.public_host(host)


if __name__ == '__main__':
    unittest.main()
