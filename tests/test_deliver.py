"""addon/deliver.py against a local web server: links to files are fetched,
web pages and refused fetches stay shortcuts, other files are moved."""
import contextlib
import http.server
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import threading
import time
import unittest
from unittest import mock

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


class HomeAssistant(http.server.BaseHTTPRequestHandler):
    """Records what the add-on posts to Home Assistant's REST API."""
    calls = []
    down = False

    def log_message(self, *args):
        pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
        HomeAssistant.calls.append((self.path, self.headers.get('Authorization'), json.loads(body)))
        self.send_response(200)
        self.send_header('Content-Length', '2')
        self.end_headers()
        self.wfile.write(b'[]')


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

    def run_once(self, allow_private, max_bytes=1024 * 1024, announcer=None):
        deliver.run(str(self.staging), str(self.dest), True, max_bytes, interval=0, once=True,
                    allow_private=allow_private, announcer=announcer)

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

    def test_shared_text_goes_to_the_text_folder(self):
        (self.staging / 'text-430B-AD01-9A-0.txt').write_text('a note', encoding='utf-8')
        (self.staging / 'notes.txt').write_text('a file', encoding='utf-8')
        (self.staging / 'text-message.txt.heic').write_bytes(b'heic')
        self.run_once(allow_private=True)
        self.assertEqual(self.listing(), ['notes.txt', 'text-message.txt.heic', 'text/text-430B-AD01-9A-0.txt'])

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



class AnnounceTests(unittest.TestCase):
    """Each delivery fires howldrop_received and notifies the notify targets,
    with the sender from the receiver's note; Home Assistant being away never
    stops a delivery."""

    @classmethod
    def setUpClass(cls):
        cls.ha = http.server.ThreadingHTTPServer(('127.0.0.1', 0), HomeAssistant)
        cls.api = f'http://127.0.0.1:{cls.ha.server_address[1]}/api'
        threading.Thread(target=cls.ha.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.ha.shutdown()

    def setUp(self):
        HomeAssistant.calls = []
        self.tmp = tempfile.TemporaryDirectory()
        self.staging = Path(self.tmp.name, 'staging')
        self.dest = Path(self.tmp.name, 'share')
        (self.staging / '.howldrop-meta').mkdir(parents=True)

    def tearDown(self):
        self.tmp.cleanup()

    def drop(self, name, data, sender, url=None):
        (self.staging / name).write_bytes(data)
        (self.staging / '.howldrop-meta' / (name + '.json')).write_text(
            json.dumps({'sender': sender, 'url': url}), encoding='utf-8')

    def deliver(self, targets=('mobile_app_phone',), api=None, token='secret'):
        announcer = deliver.announce.Announcer(list(targets), token=token, api=api or self.api)
        deliver.run(str(self.staging), str(self.dest), False, 1 << 20, interval=0, once=True,
                    announcer=announcer)

    def call(self, path):
        return [c for c in HomeAssistant.calls if c[0] == path]

    def test_howldrop_exe_gets_an_event_line(self):
        # In WSL, howldrop.exe reads the stack's output for what arrived.
        self.drop('text-1A-2.txt', b'hi', "Rocco's\tiPhone\n")
        out = io.StringIO()
        with mock.patch.dict(os.environ, {'HOWLDROP_EVENTS': 'stdout'}), contextlib.redirect_stdout(out):
            self.deliver(token='')
        events = [line for line in out.getvalue().splitlines() if line.startswith('@howldrop')]
        self.assertEqual(events, ["@howldrop\treceived\ttext/text-1A-2.txt\tRocco's iPhone\ttext"])

    def test_no_event_line_unless_asked(self):
        self.drop('IMG_1.JPG', b'jpeg', 'x')
        out = io.StringIO()
        with mock.patch.dict(os.environ, {'HOWLDROP_EVENTS': ''}), contextlib.redirect_stdout(out):
            self.deliver(token='')
        self.assertNotIn('@howldrop', out.getvalue())

    def test_a_photo_fires_the_event_and_notifies(self):
        self.drop('IMG_1.JPG', b'jpeg', "Rocco's iPhone")
        self.deliver()
        (event,) = self.call('/api/events/howldrop_received')
        self.assertEqual(event[1], 'Bearer secret')
        self.assertEqual(event[2]['sender'], "Rocco's iPhone")
        self.assertEqual(event[2]['kind'], 'image')
        self.assertEqual(event[2]['name'], 'IMG_1.JPG')
        self.assertEqual(event[2]['size'], 4)
        (note,) = self.call('/api/services/notify/mobile_app_phone')
        self.assertEqual(note[2]['message'], "Photo from Rocco's iPhone: IMG_1.JPG")
        self.assertFalse((self.staging / '.howldrop-meta' / 'IMG_1.JPG.json').exists())

    def test_text_is_previewed_and_lands_in_text(self):
        self.drop('text-42C1-BD75-E6-0.txt', 'hello from the phone'.encode(), 'Pat')
        self.deliver(targets=('notify.mobile_app_phone', 'persistent_notification'))
        (event,) = self.call('/api/events/howldrop_received')
        self.assertEqual(event[2]['kind'], 'text')
        self.assertTrue(event[2]['path'].replace('\\', '/').endswith('/text/text-42C1-BD75-E6-0.txt'))
        (note,) = self.call('/api/services/notify/mobile_app_phone')
        self.assertEqual(note[2]['message'], 'Text from Pat: hello from the phone')
        (persistent,) = self.call('/api/services/persistent_notification/create')
        self.assertEqual(persistent[2]['message'], 'Text from Pat: hello from the phone')

    def test_a_link_carries_its_address(self):
        self.drop('example.com-page.url', b'[InternetShortcut]\nURL=https://example.com/page\n', 'Pat',
                  'https://example.com/page')
        self.deliver()
        (event,) = self.call('/api/events/howldrop_received')
        self.assertEqual((event[2]['kind'], event[2]['url']), ('link', 'https://example.com/page'))
        (note,) = self.call('/api/services/notify/mobile_app_phone')
        self.assertEqual(note[2]['message'], 'Link from Pat: https://example.com/page')
        self.assertEqual(note[2]['data']['url'], 'https://example.com/page')

    def test_the_panel_history_names_the_sender(self):
        self.drop('IMG_1.JPG', b'jpeg', 'Pat')
        self.drop('text-42C1-BD75-E6-0.txt', b'hi', 'Sam')
        history = str(Path(self.tmp.name, 'drops.jsonl'))
        deliver.run(str(self.staging), str(self.dest), False, 1 << 20, interval=0, once=True, history=history)
        self.assertEqual(deliver.announce.senders(history),
                         {'IMG_1.JPG': 'Pat', os.path.join('text', 'text-42C1-BD75-E6-0.txt'): 'Sam'})

    def test_no_note_still_announces_without_a_sender(self):
        (self.staging / 'notes.pdf').write_bytes(b'%PDF')
        self.deliver(targets=())
        (event,) = self.call('/api/events/howldrop_received')
        self.assertEqual((event[2]['sender'], event[2]['kind']), ('', 'file'))
        self.assertEqual(self.call('/api/services/notify/mobile_app_phone'), [])

    def test_home_assistant_away_does_not_stop_delivery(self):
        self.drop('IMG_2.JPG', b'jpeg', 'Pat')
        self.deliver(api='http://127.0.0.1:9/api')
        self.assertTrue((self.dest / 'IMG_2.JPG').exists())

    def test_without_a_token_nothing_is_sent(self):
        self.drop('IMG_3.JPG', b'jpeg', 'Pat')
        self.deliver(token='')
        self.assertEqual(HomeAssistant.calls, [])
        self.assertTrue((self.dest / 'IMG_3.JPG').exists())

    def test_a_long_lived_token_stands_in_for_the_supervisor(self):
        saved = {k: os.environ.pop(k, None) for k in ('SUPERVISOR_TOKEN', 'HOWLDROP_HA_TOKEN')}
        try:
            os.environ['HOWLDROP_HA_TOKEN'] = 'long-lived'
            self.assertEqual(deliver.announce.Announcer([]).token, 'long-lived')
            os.environ['SUPERVISOR_TOKEN'] = 'supervisor'
            self.assertEqual(deliver.announce.Announcer([]).token, 'supervisor')
        finally:
            for key, value in saved.items():
                os.environ.pop(key, None)
                if value is not None:
                    os.environ[key] = value

    def test_old_notes_are_pruned(self):
        old = self.staging / '.howldrop-meta' / 'gone.jpg.json'
        old.write_text('{}', encoding='utf-8')
        os.utime(old, (time.time() - 2 * 86400,) * 2)
        self.assertEqual(deliver.announce.prune_notes(str(self.staging), 86400), 1)
        self.assertFalse(old.exists())


class PurgeTests(unittest.TestCase):
    """delete_after_days: files go that many days after they arrive."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dest = Path(self.tmp.name) / 'howldrop'
        (self.dest / 'text').mkdir(parents=True)
        (self.dest / 'debug').mkdir()
        (self.dest / 'photo.jpg').write_bytes(b'jpg')
        (self.dest / 'text' / 'note.txt').write_text('hi')
        (self.dest / 'debug' / 'mon0.pcap0').write_bytes(b'pcap')
        self.now = time.time()

    def tearDown(self):
        self.tmp.cleanup()

    def names(self):
        return sorted(str(p.relative_to(self.dest)).replace(os.sep, '/') for p in self.dest.rglob('*'))

    def test_nothing_goes_before_its_time(self):
        self.assertEqual(deliver.purge(str(self.dest), 30, now=self.now + 29 * 86400), 0)
        self.assertEqual(len(self.names()), 5)

    def test_old_files_go_and_empty_folders_with_them(self):
        self.assertEqual(deliver.purge(str(self.dest), 30, now=self.now + 31 * 86400), 3)
        self.assertEqual(self.names(), ['text'])
        self.assertTrue(self.dest.is_dir())

    def test_age_counts_from_arrival_not_the_files_own_date(self):
        os.utime(self.dest / 'photo.jpg', (1262304000, 1262304000))  # 2010
        self.assertEqual(deliver.purge(str(self.dest), 30, now=self.now), 0)
        self.assertIn('photo.jpg', self.names())

    def test_zero_keeps_everything(self):
        self.assertEqual(deliver.purge(str(self.dest), 0, now=self.now + 9999 * 86400), 0)
        self.assertEqual(len(self.names()), 5)

    def test_a_link_is_deleted_not_followed(self):
        outside = Path(self.tmp.name) / 'outside.txt'
        outside.write_text('keep')
        try:
            (self.dest / 'link').symlink_to(outside)
        except OSError:
            self.skipTest('cannot create a symbolic link here')
        deliver.purge(str(self.dest), 30, now=self.now + 31 * 86400)
        self.assertTrue(outside.exists())
        self.assertNotIn('link', self.names())

if __name__ == '__main__':
    unittest.main()
