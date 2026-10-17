"""addon/outbox.py against a fake send-to-peer: it waits for the phone to be
heard, picks the iPhone among several peers, and files what happened."""
import importlib.util
import os
from pathlib import Path
import stat
import tempfile
import unittest

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
        ok = outbox.send(str(self.photo), str(self.tools / 'send-to-peer'), 'HowlDrop', self.env, window)
        return ok, self.calls.read_text().splitlines()

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

    def test_a_declined_transfer_is_not_retried(self):
        ok, calls = self.send(['3 ASK -> declined (3.61s)', '0 should not run'])
        self.assertFalse(ok)
        self.assertEqual(len(calls), 1)

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


class FreePathTests(unittest.TestCase):
    def test_keeps_both(self):
        with tempfile.TemporaryDirectory() as d:
            first = outbox.free_path(os.path.join(d, 'sent'), 'a.jpg')
            Path(first).write_bytes(b'x')
            self.assertEqual(os.path.basename(outbox.free_path(os.path.join(d, 'sent'), 'a.jpg')), 'a-1.jpg')


if __name__ == '__main__':
    unittest.main()
