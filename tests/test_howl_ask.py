"""addon/howl_ask.py: what the receiver tells howldrop.exe about an incoming
AirDrop, how it waits for Accept or Decline, and the upload's progress."""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    'howl_ask', Path(__file__).resolve().parents[1] / 'addon' / 'howl_ask.py')
howl_ask = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(howl_ask)

ASK = {'SenderComputerName': 'Rocco’s iPhone',
       'Files': [{'FileName': 'IMG_1.HEIC', 'FileSize': 1500000}, {'FileName': 'a\tb.MOV', 'FileSize': 30000000}]}


def events(fn):
    out = io.StringIO()
    with mock.patch.dict(os.environ, {'HOWLDROP_EVENTS': 'stdout'}), contextlib.redirect_stdout(out):
        result = fn()
    return result, [line.split('\t')[1:] for line in out.getvalue().splitlines()]


class Clock:
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class DescribeTests(unittest.TestCase):
    def test_files(self):
        self.assertEqual(howl_ask.describe(ASK), ('Rocco’s iPhone', ['IMG_1.HEIC', 'a\tb.MOV'], 31500000))

    def test_a_link_shows_its_address(self):
        self.assertEqual(howl_ask.describe({'Items': ['https://example.com/']}),
                         ('Someone', ['https://example.com/'], 0))

    def test_odd_sizes_count_as_nothing(self):
        self.assertEqual(howl_ask.describe({'Files': [{'FileName': 'x', 'FileSize': 'big'}, 'junk']})[2], 0)

    def test_incoming_event_has_one_field_per_name(self):
        _, lines = events(lambda: howl_ask.incoming(ASK, 'abc'))
        self.assertEqual(lines, [['incoming', 'abc', 'Rocco’s iPhone', '2', '31500000', 'IMG_1.HEIC', 'a b.MOV']])

    def test_no_events_unless_asked(self):
        out = io.StringIO()
        with mock.patch.dict(os.environ, {'HOWLDROP_EVENTS': ''}), contextlib.redirect_stdout(out):
            howl_ask.incoming(ASK, 'abc')
        self.assertEqual(out.getvalue(), '')


class DecisionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = self.tmp.name
        self.clock = Clock()

    def tearDown(self):
        self.tmp.cleanup()

    def wait(self, timeout=60):
        return howl_ask.wait_decision(self.dir, 'abc', timeout, sleep=self.clock.sleep, clock=self.clock)

    def test_accept_is_read_and_removed(self):
        Path(self.dir, 'abc.accept').touch()
        self.assertEqual(self.wait(), 'accepted')
        self.assertFalse(Path(self.dir, 'abc.accept').exists())

    def test_decline(self):
        Path(self.dir, 'abc.decline').touch()
        self.assertEqual(self.wait(), 'declined')

    def test_another_transfers_answer_is_not_ours(self):
        Path(self.dir, 'other.accept').touch()
        self.assertEqual(self.wait(timeout=1), 'timeout')
        self.assertTrue(Path(self.dir, 'other.accept').exists())

    def test_no_answer_times_out(self):
        self.assertEqual(self.wait(timeout=60), 'timeout')
        self.assertGreaterEqual(self.clock.now, 60)

    def test_ask_user_says_what_waits_and_how_it_ended(self):
        Path(self.dir, 'abc.decline').touch()
        ok, lines = events(lambda: howl_ask.ask_user(ASK, 'abc', self.dir, sleep=self.clock.sleep, clock=self.clock))
        self.assertFalse(ok)
        self.assertEqual(lines[0][:5], ['ask', 'abc', 'Rocco’s iPhone', '2', '31500000'])
        self.assertEqual(lines[1], ['asked', 'abc', 'declined'])


class ProgressTests(unittest.TestCase):
    def test_counts_and_throttles(self):
        clock = Clock()
        target = io.BytesIO()
        progress = howl_ask.Progress(target, 'abc', 300, every=0.25, clock=clock)

        def run():
            progress.write(b'x' * 100)   # first write reports
            clock.now += 0.1
            progress.write(b'x' * 100)   # too soon: quiet
            clock.now += 0.2
            progress.write(b'x' * 100)   # 0.3 s since the last report
            progress.done()

        _, lines = events(run)
        self.assertEqual(lines, [['receiving', 'abc', '100', '300'], ['receiving', 'abc', '300', '300'],
                                 ['receiving', 'abc', '300', '300']])
        self.assertEqual(target.getvalue(), b'x' * 300)

    def test_the_file_is_still_the_file(self):
        with tempfile.TemporaryFile() as raw:
            progress = howl_ask.Progress(raw, 'abc', 0)
            self.assertEqual(progress.fileno(), raw.fileno())

    def test_total_from_headers(self):
        self.assertEqual(howl_ask.total_of({'TotalBytes': '781493'}), 781493)
        self.assertEqual(howl_ask.total_of({}), 0)
        self.assertEqual(howl_ask.total_of({'TotalBytes': 'x'}), 0)


if __name__ == '__main__':
    unittest.main()
