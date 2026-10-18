"""tx_channels: both formats iw prints, and $CONF/channels."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SOURCE = (Path(__file__).resolve().parents[1] / 'userspace/omdrop-discoverable').read_text()
TX_CHANNELS = SOURCE[SOURCE.index('tx_channels()'):SOURCE.index('\n# Take the station off the air')]

INTEGER = '''Band 1:
	* 2437 MHz [6]
	  Maximum TX power: 22.0 dBm
Band 2:
	* 5220 MHz [44]
	  No IR
	* 5745 MHz [149]
	* 5765 MHz [153]
	  Radar detection
'''

DECIMAL = '''Band 1:
	* 2437.0 MHz [6] (20.0 dBm)
Band 2:
	* 5220.0 MHz [44] (20.0 dBm)
	* 5745.0 MHz [149] (20.0 dBm) (no IR)
'''


def offered(listing, channels=None, driver=''):
    with tempfile.TemporaryDirectory() as conf:
        if channels is not None:
            Path(conf, 'channels').write_text(channels)
        script = 'iw() { printf "%s" "$LISTING"; }\n' + TX_CHANNELS + f'\ntx_channels phy0 {driver}\n'
        result = subprocess.run(['bash', '-c', script], env=dict(os.environ, LISTING=listing, CONF=conf),
                                capture_output=True, text=True, timeout=10)
    assert result.returncode == 0, result.stderr
    return result.stdout.split()


class ChannelTests(unittest.TestCase):
    def test_integer_listing(self):
        self.assertEqual(offered(INTEGER), ['5745', '2437'])

    def test_decimal_listing_with_inline_no_ir(self):
        self.assertEqual(offered(DECIMAL), ['5220', '2437'])

    def test_channels_file_sets_the_order(self):
        self.assertEqual(offered(INTEGER.replace('	  No IR\n', ''), '149 6\n'), ['5745', '2437'])

    def test_iwlwifi_defaults_to_channel_6(self):
        self.assertEqual(offered(DECIMAL, driver='iwlwifi'), ['2437'])

    def test_channels_file_overrides_the_iwlwifi_default(self):
        self.assertEqual(offered(DECIMAL, '44 6\n', driver='iwlwifi'), ['5220', '2437'])


if __name__ == '__main__':
    unittest.main()
