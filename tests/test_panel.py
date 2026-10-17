"""addon/panel.py, the sidebar panel: what it lists, who it answers, what it
serves inline, and that nothing outside save_folder can be read or deleted."""
import importlib.util
import ipaddress
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import unittest
import urllib.error
import urllib.request

ADDON = Path(__file__).resolve().parents[1] / 'addon'
sys.path.insert(0, str(ADDON))
SPEC = importlib.util.spec_from_file_location('panel', ADDON / 'panel.py')
panel = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(panel)
import announce  # noqa: E402


class PanelTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.dest = root / 'share'
        (self.dest / 'text').mkdir(parents=True)
        (self.dest / 'debug').mkdir()
        self.history = str(root / 'drops.jsonl')
        self.outside = root / 'secret.txt'
        self.outside.write_text('not yours')
        self.servers = []

    def tearDown(self):
        for server in self.servers:
            server.shutdown()
            server.server_close()
        self.tmp.cleanup()

    def start(self, allow=('127.0.0.1',), outbox=None, status=None):
        server = panel.serve(str(self.dest), 0, self.history, str(Path(self.tmp.name, 'thumbs')),
                             str(ADDON / 'panel'), set(allow), hosts=('127.0.0.1',), outbox=outbox,
                             send_status_file=status)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.servers.append(server)
        return f'http://127.0.0.1:{server.server_address[1]}'

    def get(self, url, headers=None):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=headers or {}), timeout=5) as reply:
                return reply.status, dict(reply.headers), reply.read()
        except urllib.error.HTTPError as e:
            return e.code, dict(e.headers), e.read()

    def post(self, url, body, ctype='application/json'):
        request = urllib.request.Request(url, data=json.dumps(body).encode(), method='POST',
                                         headers={'Content-Type': ctype})
        try:
            with urllib.request.urlopen(request, timeout=5) as reply:
                return reply.status, json.loads(reply.read())
        except urllib.error.HTTPError as e:
            return e.code, None

    def drop(self, rel, data=b'x', sender=''):
        path = self.dest / rel
        path.write_bytes(data)
        announce.record(self.history, str(path), str(self.dest), sender)
        return path

    def test_lists_drops_newest_first_with_sender(self):
        old = self.drop('IMG_1.JPG', b'\xff\xd8', 'Pat')
        os.utime(old, (1_000_000, 1_000_000))
        self.drop('text/text-AB12-0.txt', 'hello\nthere'.encode(), 'Sam')
        self.drop('page.url', b'[InternetShortcut]\nURL=https://example.com/a\n')
        (self.dest / '.hidden').write_bytes(b'x')
        (self.dest / 'x.jpg.partial').write_bytes(b'x')
        (self.dest / 'debug' / 'mon0.pcap').write_bytes(b'x')
        drops = panel.listing(str(self.dest), self.history)
        names = [d['name'] for d in drops]
        self.assertEqual(set(names), {'IMG_1.JPG', 'text-AB12-0.txt', 'page.url'})
        self.assertEqual(names[-1], 'IMG_1.JPG')  # newest first: the old photo is last
        by_name = {d['name']: d for d in drops}
        self.assertEqual(by_name['IMG_1.JPG']['sender'], 'Pat')
        self.assertEqual(by_name['IMG_1.JPG']['kind'], 'image')
        self.assertEqual(by_name['text-AB12-0.txt']['id'], 'text/text-AB12-0.txt')
        self.assertEqual(by_name['text-AB12-0.txt']['text'], 'hello\nthere')
        self.assertEqual(by_name['page.url']['url'], 'https://example.com/a')

    def test_only_http_links_are_handed_to_the_page(self):
        self.drop('evil.url', b'[InternetShortcut]\nURL=javascript:alert(1)\n')
        (drop,) = panel.listing(str(self.dest), self.history)
        self.assertNotIn('url', drop)

    def test_answers_only_the_allowed_address(self):
        base = self.start(allow=('172.30.32.2',))
        self.assertEqual(self.get(base + '/api/drops')[0], 403)
        self.assertEqual(self.get(base + '/')[0], 403)
        base = self.start()
        status, headers, body = self.get(base + '/')
        self.assertEqual(status, 200)
        self.assertIn(b'HowlDrop', body)
        self.assertIn("script-src 'self'", headers['Content-Security-Policy'])

    def test_allow_takes_networks(self):
        allow = panel.parse_allow(['192.168.1.0/24', '10.0.0.5', 'nonsense'])
        self.assertEqual(len(allow), 2)

        def allowed(address):
            return any(ipaddress.ip_address(address) in network for network in allow)
        self.assertTrue(allowed('192.168.1.40'))
        self.assertTrue(allowed('10.0.0.5'))
        self.assertFalse(allowed('10.0.0.6'))
        base = self.start(allow=('127.0.0.0/8',))
        self.assertEqual(self.get(base + '/api/drops')[0], 200)
        base = self.start(allow=('*',))
        self.assertEqual(self.get(base + '/api/drops')[0], 200)

    def test_resolve_keeps_to_save_folder(self):
        self.drop('a.jpg')
        self.drop('text/t.txt')
        (self.dest / 'debug' / 'x.pcap').write_bytes(b'x')
        os.symlink(self.outside, self.dest / 'link.txt')
        dest = str(self.dest)
        self.assertTrue(panel.resolve(dest, 'a.jpg'))
        self.assertTrue(panel.resolve(dest, 'text/t.txt'))
        for bad in ('../secret.txt', 'text/../../secret.txt', '/etc/passwd', 'debug/x.pcap', 'link.txt',
                    '.', '..', '', 'text', 'a/b/c', 'text\\..\\..\\secret.txt', 'a.jpg\0'):
            self.assertIsNone(panel.resolve(dest, bad), bad)

    def test_serves_pictures_inline_and_the_rest_as_downloads(self):
        self.drop('a.jpg', b'\xff\xd8\xffjpeg')
        self.drop('page.html', b'<script>alert(1)</script>')
        self.drop('pic.svg', b'<svg onload="alert(1)"/>')
        base = self.start()
        status, headers, body = self.get(base + '/file?p=a.jpg')
        self.assertEqual((status, headers['Content-Type']), (200, 'image/jpeg'))
        self.assertTrue(headers['Content-Disposition'].startswith('inline'))
        self.assertIn('sandbox', headers['Content-Security-Policy'])
        self.assertEqual(headers['X-Content-Type-Options'], 'nosniff')
        for name in ('page.html', 'pic.svg'):
            status, headers, _ = self.get(base + '/file?p=' + name)
            self.assertEqual(headers['Content-Type'], 'application/octet-stream', name)
            self.assertTrue(headers['Content-Disposition'].startswith('attachment'), name)
        status, headers, _ = self.get(base + '/file?p=a.jpg&download=1')
        self.assertTrue(headers['Content-Disposition'].startswith('attachment'))
        self.assertEqual(self.get(base + '/file?p=../secret.txt')[0], 404)

    def test_a_strange_name_cannot_break_the_header(self):
        name = 'a"b;cé.txt'
        self.drop(name)
        base = self.start()
        _, headers, _ = self.get(base + '/file?p=' + urllib.request.quote(name) + '&download=1')
        self.assertEqual(headers['Content-Disposition'],
                         'attachment; filename="a_b_c_.txt"; filename*=UTF-8\'\'a%22b%3Bc%C3%A9.txt')

    def test_ranges_for_video(self):
        self.drop('clip.mp4', bytes(range(100)))
        base = self.start()
        status, headers, body = self.get(base + '/file?p=clip.mp4', {'Range': 'bytes=10-19'})
        self.assertEqual((status, body), (206, bytes(range(10, 20))))
        self.assertEqual(headers['Content-Range'], 'bytes 10-19/100')
        status, _, body = self.get(base + '/file?p=clip.mp4', {'Range': 'bytes=-5'})
        self.assertEqual((status, body), (206, bytes(range(95, 100))))
        self.assertEqual(self.get(base + '/file?p=clip.mp4', {'Range': 'bytes=200-'})[0], 416)

    def test_delete_takes_json_and_stays_in_save_folder(self):
        target = self.drop('a.jpg')
        base = self.start()
        self.assertEqual(self.post(base + '/api/delete', {'id': 'a.jpg'}, ctype='text/plain')[0], 415)
        self.assertTrue(target.exists())
        self.assertEqual(self.post(base + '/api/delete', {'ids': ['../secret.txt', 'debug']}),
                         (200, {'deleted': 0}))
        self.assertTrue(self.outside.exists())
        self.assertEqual(self.post(base + '/api/delete', {'id': 'a.jpg'}), (200, {'deleted': 1}))
        self.assertFalse(target.exists())

    def test_thumbnail(self):
        try:
            from PIL import Image
        except ImportError:
            self.skipTest('Pillow is not installed')
        Image.new('RGB', (1200, 800), (200, 30, 30)).save(self.dest / 'big.jpg')
        base = self.start()
        status, headers, body = self.get(base + '/thumb?p=big.jpg')
        self.assertEqual((status, headers['Content-Type']), (200, 'image/jpeg'))
        thumb = Path(self.tmp.name, 'small.jpg')
        thumb.write_bytes(body)
        with Image.open(thumb) as img:
            self.assertEqual(max(img.size), panel.THUMB_PX)
        (self.dest / 'bad.jpg').write_bytes(b'not a picture')
        self.assertEqual(self.get(base + '/thumb?p=bad.jpg')[0], 404)

    def test_heic_is_shown_as_jpeg(self):
        try:
            from PIL import Image
        except ImportError:
            self.skipTest('Pillow is not installed')
        if not panel.heif_opener():
            self.skipTest('pillow-heif is not installed')
        heic = self.dest / 'FullSizeRender.heic'
        heic.write_bytes((Path(__file__).parent / 'data' / 'sample.heic').read_bytes())
        (drop,) = panel.listing(str(self.dest), self.history)
        self.assertTrue(drop['thumb'] and drop['inline'])
        base = self.start()
        status, headers, body = self.get(base + '/file?p=FullSizeRender.heic')
        self.assertEqual((status, headers['Content-Type']), (200, 'image/jpeg'))
        self.assertIn('filename="FullSizeRender.jpg"', headers['Content-Disposition'])
        view = Path(self.tmp.name, 'view.jpg')
        view.write_bytes(body)
        with Image.open(view) as img:
            self.assertEqual(img.size, (640, 480))
            self.assertEqual(dict(img.getexif()), {})
            red, green, blue = img.convert('RGB').getpixel((10, 10))  # the red corner, not turned
            self.assertTrue(red > 200 and green < 60 and blue < 60)
        status, headers, body = self.get(base + '/thumb?p=FullSizeRender.heic')
        self.assertEqual((status, headers['Content-Type']), (200, 'image/jpeg'))
        view.write_bytes(body)
        with Image.open(view) as img:
            self.assertEqual(max(img.size), panel.THUMB_PX)
        _, headers, body = self.get(base + '/file?p=FullSizeRender.heic&download=1')
        self.assertEqual(headers['Content-Type'], 'application/octet-stream')
        self.assertEqual(body, heic.read_bytes())

    def test_a_broken_heic_falls_back(self):
        (self.dest / 'bad.heic').write_bytes(b'not a photo')
        base = self.start()
        self.assertEqual(self.get(base + '/thumb?p=bad.heic')[0], 404)
        status, headers, _ = self.get(base + '/file?p=bad.heic')
        self.assertEqual((status, headers['Content-Type']), (200, 'image/heic'))

    def upload(self, base, name, data, ctype='application/octet-stream'):
        request = urllib.request.Request(base + '/api/send?name=' + urllib.request.quote(name), data=data,
                                         method='POST', headers={'Content-Type': ctype})
        try:
            with urllib.request.urlopen(request, timeout=5) as reply:
                return reply.status, json.loads(reply.read())
        except urllib.error.HTTPError as e:
            return e.code, None

    def test_send_to_iphone_off(self):
        base = self.start()
        self.assertEqual(self.get(base + '/api/send')[2], b'{"enabled": false, "jobs": []}')
        self.assertEqual(self.upload(base, 'a.jpg', b'x')[0], 404)

    def test_send_to_iphone_upload(self):
        outbox = Path(self.tmp.name, 'share', 'send')
        status = Path(self.tmp.name, 'send-status.json')
        status.write_text(json.dumps({'jobs': [{'name': 'old.jpg', 'state': 'sent', 'detail': 'on the iPhone'}]}))
        base = self.start(outbox=str(outbox), status=str(status))
        sent = json.loads(self.get(base + '/api/send')[2])
        self.assertEqual((sent['enabled'], sent['jobs'][0]['name']), (True, 'old.jpg'))

        self.assertEqual(self.upload(base, 'a.jpg', b'one', ctype='text/plain')[0], 415)
        self.assertEqual(self.upload(base, 'IMG 1.jpg', b'one'), (200, {'name': 'IMG 1.jpg'}))
        self.assertEqual(self.upload(base, 'IMG 1.jpg', b'two'), (200, {'name': 'IMG 1-1.jpg'}))
        self.assertEqual((outbox / 'IMG 1.jpg').read_bytes(), b'one')
        self.assertEqual((outbox / 'IMG 1-1.jpg').read_bytes(), b'two')
        # A name is only a name: no way out of the send folder, no dotfile.
        self.assertEqual(self.upload(base, '../../evil.sh', b'x'), (200, {'name': 'evil.sh'}))
        self.assertEqual(self.upload(base, '.hidden', b'x'), (200, {'name': 'hidden'}))
        self.assertFalse(Path(self.tmp.name, 'evil.sh').exists())
        self.assertEqual(sorted(p.name for p in outbox.iterdir()), ['IMG 1-1.jpg', 'IMG 1.jpg', 'evil.sh', 'hidden'])

    def test_history_is_trimmed(self):
        for i in range(announce.HISTORY_KEPT * 2):
            announce.record(self.history, str(self.dest / f'f{i}.jpg'), str(self.dest), 'Pat')
        with open(self.history, encoding='utf-8') as fh:
            lines = fh.readlines()
        self.assertLessEqual(len(lines), announce.HISTORY_KEPT * 2)
        self.assertIn(f'f{announce.HISTORY_KEPT * 2 - 1}.jpg', lines[-1])
        self.assertEqual(announce.senders(self.history)[f'f{announce.HISTORY_KEPT * 2 - 1}.jpg'], 'Pat')


if __name__ == '__main__':
    unittest.main()
