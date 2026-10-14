"""addon/run.sh's radio choice: which Wi-Fi interface carries the window, by
the real owl-profiles table, with the host's interfaces and routes mocked."""
from pathlib import Path
import os
import shutil
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
SOURCE = (ROOT / 'addon/run.sh').read_text()
CHOICE = SOURCE[SOURCE.index('# owl-profiles as the helper reads it'):SOURCE.index('usb_id_of() {')]
# Through PATH: Windows itself looks in System32 first, which is WSL's bash.
BASH = shutil.which('bash') or 'bash'


def run(cards, uplink='', allow_untested=False, body='pick_radio'):
    """cards: {iface: (driver, active_monitor)}."""
    drivers = ' '.join(f'[{n}]={d}' for n, (d, _) in cards.items())
    active = ' '.join(n for n, (_, a) in cards.items() if a)
    script = f'''
set -uo pipefail
LIB={(ROOT / 'userspace').as_posix()}
ALLOW_UNTESTED={'true' if allow_untested else 'false'}
declare -A DRV=({drivers})
say() {{ printf '%s\\n' "$*"; }}
driver_of() {{ echo "${{DRV[$1]:-}}"; }}
phy_of() {{ echo "phy-$1"; }}
bus_id_of() {{ echo "bus"; }}
iw() {{ case " {active} " in *" ${{2#phy-}} "*) echo '	Device supports active monitor (which will ACK incoming frames)' ;; esac; }}
ip() {{ [[ "$*" == '-4 route show default' && -n "{uplink}" ]] && echo "default via 192.168.1.1 dev {uplink} proto dhcp"; return 0; }}
''' + CHOICE + f'''
wifi_ifaces() {{ printf '%s\\n' {' '.join(cards)}; }}
{body}
'''
    out = subprocess.run([BASH, '-s'], input=script.encode(), capture_output=True, timeout=20)
    if out.returncode:
        raise AssertionError(out.stderr.decode(errors='replace'))
    return out.stdout.decode().strip()


class RadioChoiceTest(unittest.TestCase):
    def test_active_monitor_card_beats_an_ibss_card(self):
        self.assertEqual(run({'wlp3s0': ('iwlwifi', False), 'wlx0': ('mt76x2u', True)}),
                         'wlx0 active-monitor tested')

    def test_ibss_card_off_the_network(self):
        self.assertEqual(run({'wlp3s0': ('iwlwifi', False)}), 'wlp3s0 ibss tested')

    def test_never_the_uplink_in_exclusive_mode(self):
        self.assertEqual(run({'wlp3s0': ('iwlwifi', False)}, uplink='wlp3s0'), '')
        self.assertIn('carries Home Assistant', run({'wlp3s0': ('iwlwifi', False)}, uplink='wlp3s0',
                                                    body='explain_radios'))

    def test_shared_card_needs_its_network(self):
        self.assertEqual(run({'wlp1s0': ('mt7925e', False)}, uplink='wlp1s0'), 'wlp1s0 shared tested')
        self.assertEqual(run({'wlp1s0': ('mt7925e', False)}), '')

    def test_exclusive_card_beats_a_shared_uplink(self):
        cards = {'wlp1s0': ('mt7925e', False), 'wlx0': ('mt76x2u', True)}
        self.assertEqual(run(cards, uplink='wlp1s0'), 'wlx0 active-monitor tested')

    def test_driver_without_active_monitor_is_skipped(self):
        self.assertEqual(run({'wlx0': ('mt76x2u', False)}), '')
        self.assertIn('offers none', run({'wlx0': ('mt76x2u', False)}, body='explain_radios'))

    def test_unknown_card_needs_allow_untested(self):
        cards = {'wlan0': ('somedriver', False)}
        self.assertEqual(run(cards, uplink='wlan0'), '')
        self.assertIn('allow_untested', run(cards, uplink='wlan0', body='explain_radios'))
        self.assertEqual(run(cards, uplink='wlan0', allow_untested=True), 'wlan0 shared untested')

    def test_tested_card_beats_an_untested_one(self):
        cards = {'wlan0': ('rtw89_8852ae', False), 'wlp3s0': ('iwlwifi', False)}
        self.assertEqual(run(cards, allow_untested=True), 'wlp3s0 ibss tested')


if __name__ == '__main__':
    unittest.main()
