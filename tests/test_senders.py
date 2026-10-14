"""addon/senders.py: the senders option becomes the receiver's visibility and
known-senders files, in the form omdrop-plugin's contacts.py hashes."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    'senders', Path(__file__).resolve().parents[1] / 'addon' / 'senders.py')
senders = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(senders)


class SendersTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.config = Path(self.tmp.name) / 'omdrop'

    def tearDown(self):
        self.tmp.cleanup()

    def files(self):
        visibility = (self.config / 'visibility').read_text().strip()
        known = self.config / 'known-senders'
        return visibility, known.read_text().splitlines() if known.exists() else None

    def test_empty_means_everyone(self):
        lines = senders.write(self.config, {'senders': []})
        self.assertEqual(self.files(), ('everyone', None))
        self.assertIn('everyone nearby', lines[0])

    def test_missing_option_means_everyone(self):
        senders.write(self.config, {})
        self.assertEqual(self.files(), ('everyone', None))

    def test_list_turns_on_contacts_only(self):
        lines = senders.write(self.config, {'senders': ['+1 (555) 123-4567', 'Me@iCloud.com']})
        self.assertEqual(self.files(), ('contacts', ['15551234567', 'me@icloud.com']))
        self.assertIn('Contacts Only', lines[0])
        self.assertIn('1 phone number(s) and 1 email', lines[0])
        self.assertEqual(len(lines), 1)

    def test_lines_never_name_a_sender(self):
        lines = senders.write(self.config, {'senders': ['+1 555 123 4567', 'me@icloud.com']})
        joined = '\n'.join(lines)
        self.assertNotIn('5551234567', joined.replace(' ', ''))
        self.assertNotIn('icloud', joined)

    def test_comma_separated_string(self):
        senders.write(self.config, {'senders': '+15551234567, me@icloud.com;\n+44 20 7946 0000'})
        self.assertEqual(self.files()[1], ['15551234567', '442079460000', 'me@icloud.com'])

    def test_duplicates_collapse(self):
        senders.write(self.config, {'senders': ['+1 555 123 4567', '+15551234567', 'ME@icloud.com', 'me@icloud.com']})
        self.assertEqual(self.files()[1], ['15551234567', 'me@icloud.com'])

    def test_phone_without_country_code_warns_but_is_kept(self):
        lines = senders.write(self.config, {'senders': ['555-123-4567']})
        self.assertEqual(self.files(), ('contacts', ['5551234567']))
        self.assertTrue(any('country code' in line for line in lines[1:]))

    def test_garbage_is_dropped(self):
        lines = senders.write(self.config, {'senders': ['bob', 'x@y', '+1 555 123 4567']})
        self.assertEqual(self.files(), ('contacts', ['15551234567']))
        self.assertEqual(sum('neither' in line or 'not an email' in line for line in lines), 2)

    def test_only_garbage_means_everyone(self):
        senders.write(self.config, {'senders': ['bob']})
        self.assertEqual(self.files(), ('everyone', None))

    def test_clearing_the_option_removes_the_list(self):
        senders.write(self.config, {'senders': ['me@icloud.com']})
        senders.write(self.config, {'senders': []})
        self.assertEqual(self.files(), ('everyone', None))

    def test_main_reads_options_json(self):
        options = Path(self.tmp.name) / 'options.json'
        options.write_text(json.dumps({'senders': ['me@icloud.com']}))
        self.assertEqual(senders.main(['--options', str(options), '--config', str(self.config)]), 0)
        self.assertEqual(self.files(), ('contacts', ['me@icloud.com']))


if __name__ == '__main__':
    unittest.main()
