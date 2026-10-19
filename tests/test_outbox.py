"""addon/outbox.py against a fake send-to-peer: it waits for the phone to be
heard, picks the iPhone among several peers, and files what happened."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    'outbox', Path(__file__).resolve().parents[1] / 'addon' / 'outbox.py')
outbox = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(outbox)

# The fake reads its script from $FAKE_PLAN: one line per call, "CODE TEXT"
# (TEXT with \n for new lines). It logs each argv to $FAKE_CALLS and checks
# that pkexec is on PATH, as the real one needs.
FAKE = r'''#!/bin/sh
command -v pkexec >/dev/null || { echo "no pkexec"; exit 99; }
echo "$*" >> "$FAKE_CALLS"
n=$(wc -l < "$FAKE_CALLS")
line=$(sed -n "${n}p" "$FAKE_PLAN")
[ -n "$line" ] || { echo "plan ran out"; exit 98; }
printf '%b\n' "${line#* }"
exit "${line%% *}"
'''


@unittest.skipIf(os.name == 'nt', 'the fake is a shell script')
class OutboxTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.tools = root / 'tools'
        self.tools.mkdir()
        fake = self.tools / 'send-to-peer'
        fake.write_text(FAKE)
        fake.chmod(fake.stat().st_mode | stat.S_IEXEC)
        self.plan = root / 'plan'
        self.calls = root / 'calls'
        self.calls.write_text('')
        self.photo = root / 'photo.jpg'
        self.photo.write_bytes(b'jpeg')
        self.env = {**os.environ, 'PATH': outbox.shim_dir() + os.pathsep + os.environ['PATH'],
                    'FAKE_PLAN': str(self.plan), 'FAKE_CALLS': str(self.calls)}
        outbox.PEER_RETRY_S = 0

    def tearDown(self):
        self.tmp.cleanup()

    def send(self, plan, window=30):
        self.plan.write_text('\n'.join(plan) + '\n')
        self.stages = []
        ok, self.detail = outbox.send(str(self.photo), str(self.tools / 'send-to-peer'), 'HowlDrop', self.env,
                                      window, self.stages.append)
        return ok, self.calls.read_text().splitlines()

    def test_progress_follows_the_sender(self):
        ok, _ = self.send([
            "0 waiting up to 60s for aa:bb:cc:dd:ee:ff to start listening on 8770\\n"
            "2026-10-16 20:50:41,376 INFO send: +  0.96s FOUND direct 'Rocco iPhone' at x; asking\\n"
            "2026-10-16 20:50:42,465 DEBUG PIL.Image: Importing PngImagePlugin\\n"
            "2026-10-16 20:50:46,579 INFO send: +  6.16s ASK -> accepted (4.15s)",
        ])
        self.assertTrue(ok)
        self.assertEqual(self.stages, ['waiting for the iPhone to listen', 'asking: accept it on the iPhone',
                                       'accepted; uploading'])
        self.assertEqual(self.detail, 'on the iPhone')

    def test_waits_until_the_phone_is_heard(self):
        ok, calls = self.send([
            '2 no devices found: nothing nearby is advertising.',
            '2 no devices found: nothing nearby is advertising.',
            '0 sending photo.jpg to aa:bb:cc:dd:ee:ff\\nUPLOAD photo.jpg -> ok',
        ])
        self.assertTrue(ok)
        self.assertEqual(len(calls), 3)
        self.assertIn('--wait 60', calls[0])
        self.assertIn('--name HowlDrop', calls[0])
        self.assertTrue(calls[0].endswith('-- ' + str(self.photo)))

    def test_picks_the_iphone_among_several_peers(self):
        ok, calls = self.send([
            '2 several peers heard; choose one with --mac:\\n  11:22:33:44:55:66  ? dBm\\n  aa:bb:cc:dd:ee:ff  ? dBm',
            "0 11:22:33:44:55:66  ? dBm  [fe80::1%awdl0]:8770  Pat's MacBook\\n"
            "aa:bb:cc:dd:ee:ff  ? dBm  [fe80::2%awdl0]:8770  Rocco's iPhone",
            '0 UPLOAD photo.jpg -> ok',
        ])
        self.assertTrue(ok)
        self.assertIn('--list --names', calls[1])
        self.assertIn('--mac aa:bb:cc:dd:ee:ff', calls[2])

    def test_send_to_names_the_phone(self):
        self.plan.write_text('\n'.join([
            '2 no device answering to "Rocco\'s iPhone" is receiving right now.',
            '0 UPLOAD photo.jpg -> ok',
        ]) + '\n')
        ok, detail = outbox.send(str(self.photo), str(self.tools / 'send-to-peer'), 'HowlDrop', self.env, 30,
                                 None, "Rocco's iPhone")
        calls = self.calls.read_text().splitlines()
        self.assertTrue(ok)
        # Asked as typed, then with iOS's curly apostrophe.
        self.assertIn("--mac Rocco's iPhone", calls[0])
        self.assertIn('--mac Rocco’s iPhone', calls[1])

    def test_send_to_more_than_one_match(self):
        self.plan.write_text('2 "iPhone" matches more than one device:\n')
        ok, detail = outbox.send(str(self.photo), str(self.tools / 'send-to-peer'), 'HowlDrop', self.env, 30,
                                 None, 'iPhone')
        self.assertFalse(ok)
        self.assertIn('more than one device', detail)

    def test_a_declined_transfer_is_not_retried(self):
        ok, calls = self.send(['3 ASK -> declined (3.61s)', '0 should not run'])
        self.assertFalse(ok)
        self.assertEqual(len(calls), 1)
        self.assertEqual(self.detail, 'declined on the iPhone')

    def test_an_ask_nobody_accepts_says_so(self):
        # 2026-10-18: the phone closed the ask after 30 s without an answer.
        ok, _ = self.send([
            "1 2026-10-18 21:24:49,564 INFO send: +  0.07s FOUND direct 'Rocco iPhone' at x; asking\\n"
            "2026-10-18 21:25:20,007 INFO send: + 30.51s ASK failed: RemoteDisconnected('Remote end closed "
            "connection without response')",
        ])
        self.assertFalse(ok)
        self.assertEqual(self.detail, 'not accepted on the iPhone in time: send it again and tap Accept')

    def test_another_failure_reads_without_the_log_prefix(self):
        ok, _ = self.send(["1 2026-10-18 21:25:20,007 INFO send: + 30.51s UPLOAD failed: broken pipe"])
        self.assertFalse(ok)
        self.assertEqual(self.detail, 'UPLOAD failed: broken pipe')

    def test_gives_up_when_the_phone_never_shows(self):
        ok, calls = self.send(['2 no devices found'] * 500, window=0.2)
        self.assertFalse(ok)


@unittest.skipIf(os.name == 'nt', 'symlinks')
class IdentityTests(unittest.TestCase):
    """The sender's identity is made in the store, through ~/.omdrop."""

    def test_makes_the_pair_through_the_link(self):
        with tempfile.TemporaryDirectory() as d:
            tools, home, store = (os.path.join(d, n) for n in ('tools', 'home', 'store'))
            os.makedirs(tools)
            os.makedirs(home)
            # omdrop-awdl's function, as far as the outbox uses it.
            Path(tools, 'awdl_identity.py').write_text(
                'def _create_self_signed(directory, name):\n'
                '    (directory / "certificate.self-signed.pem").write_text(name)\n'
                '    (directory / "key.self-signed.pem").write_text(name)\n')
            self.assertIsNone(outbox.ensure_identity(tools, 'HowlDrop', home, store))
            self.assertEqual(os.path.realpath(os.path.join(home, '.omdrop')), os.path.realpath(store))
            self.assertEqual(Path(home, '.omdrop', 'keys', 'certificate.self-signed.pem').read_text(), 'HowlDrop')
            self.assertIsNone(outbox.ensure_identity(tools, 'HowlDrop', home, store))  # again: fine

    def test_a_failure_is_reported(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertIn('could not make', outbox.ensure_identity(os.path.join(d, 'none'), 'x',
                                                                    os.path.join(d, 'home'), os.path.join(d, 's')))


class NearbyTests(unittest.TestCase):
    ROWS = ['e2:9d:03:6c:58:23  ? dBm  [fe80::e09d:3ff:fe6c:5823%awdl0]:8770  Rocco’s iPhone',
            '16:2f:da:27:3d:76  -33 dBm  [fe80::142f:daff:fe27:3d76%awdl0]:8770  (no response)',
            'fe:63:e6:68:9f:12  -14 dBm  [fe80::fc63:e6ff:fe68:9f12%awdl0]:8770  (anonymous)',
            'aa:bb:cc:dd:ee:ff  ? dBm  [fe80::1%awdl0]:8770  Office Mac ',
            'waiting for names']

    def test_only_devices_that_said_a_name(self):
        self.assertEqual(outbox.parse_nearby(self.ROWS),
                         [('e2:9d:03:6c:58:23', 'Rocco’s iPhone'), ('aa:bb:cc:dd:ee:ff', 'Office Mac')])

    def test_written_for_the_panel_and_the_relay(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, 'nearby.json')
            outbox.write_nearby(path, [('aa:bb:cc:dd:ee:ff', 'Rocco’s iPhone')], True)
            data = json.loads(Path(path).read_text(encoding='utf-8'))
            self.assertEqual((data['scanning'], data['devices'][0]['name']), (True, 'Rocco’s iPhone'))
            self.assertEqual(Path(d, 'nearby.txt').read_text(encoding='utf-8'), 'scanning\ndevice\tRocco’s iPhone\n')

    def test_howldrop_exe_gets_the_scan_as_event_lines(self):
        out = io.StringIO()
        with mock.patch.dict(os.environ, {'HOWLDROP_EVENTS': 'stdout'}), contextlib.redirect_stdout(out):
            outbox.tell_nearby([], True)
            outbox.tell_nearby([('aa:bb:cc:dd:ee:ff', 'Rocco’s iPhone'), ('e2:9d:03:6c:58:23', 'Office Mac')], False)
        self.assertEqual(out.getvalue().splitlines(),
                         ['@howldrop\tnearby\tscanning', '@howldrop\tnearby\tdone\tRocco’s iPhone\tOffice Mac'])

    def test_a_file_names_its_device(self):
        with tempfile.TemporaryDirectory() as d:
            photo = os.path.join(d, 'a.jpg')
            Path(photo).write_bytes(b'x')
            self.assertIsNone(outbox.target_of(photo))
            Path(photo + outbox.TARGET_SUFFIX).write_text('Pat’s iPhone\n', encoding='utf-8')
            self.assertEqual(outbox.target_of(photo), 'Pat’s iPhone')
            self.assertEqual(outbox.waiting(d), [photo])  # the sidecar is not a file to send


class StatusTests(unittest.TestCase):
    def test_newest_first_and_capped(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, 'status.json')
            status = outbox.Status(path)
            first = status.add('a.jpg')
            status.update(first, 'sent', 'on the iPhone')
            for i in range(outbox.STATUS_KEPT + 5):
                status.add(f'{i}.jpg')
            jobs = json.loads(Path(path).read_text())['jobs']
            self.assertEqual(len(jobs), outbox.STATUS_KEPT)
            self.assertEqual(jobs[0]['name'], f'{outbox.STATUS_KEPT + 4}.jpg')
            self.assertEqual(jobs[0]['state'], 'waiting')

    def test_the_log_skips_debug_detail(self):
        self.assertFalse(outbox.useful('2026-10-16 20:50:42,465 DEBUG PIL.Image: Importing PngImagePlugin'))
        self.assertFalse(outbox.useful('/usr/lib/omdrop/airdrop-send.py:105: DeprecationWarning: ssl'))
        self.assertTrue(outbox.useful('2026-10-16 20:50:46,579 INFO send: +  6.16s ASK -> accepted (4.15s)'))


class FreePathTests(unittest.TestCase):
    def test_keeps_both(self):
        with tempfile.TemporaryDirectory() as d:
            first = outbox.free_path(os.path.join(d, 'sent'), 'a.jpg')
            Path(first).write_bytes(b'x')
            self.assertEqual(os.path.basename(outbox.free_path(os.path.join(d, 'sent'), 'a.jpg')), 'a-1.jpg')


if __name__ == '__main__':
    unittest.main()
