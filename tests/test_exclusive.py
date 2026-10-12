"""Exercise exclusive mode with command doubles, never the real radio."""
import os
import shutil
from pathlib import Path
import subprocess
import tempfile
import unittest

# Through PATH: Windows itself looks in System32 first, which is WSL's bash,
# and WSL drops this environment.
BASH = shutil.which('bash') or 'bash'
SOURCE = (Path(__file__).resolve().parents[1] / 'userspace/omdrop-discoverable').read_text()
FUNCTIONS = SOURCE[SOURCE.index('tx_channels()'):SOURCE.index('# --------------------------------------------------------------------- status')]
STATION = SOURCE[SOURCE.index('station_iface()'):SOURCE.index('\ndriver_of()')]

class ExclusiveTests(unittest.TestCase):
    def run_case(self, extra='', fallback=True, join=True, conf=None):
        with tempfile.TemporaryDirectory() as tmp:
            conf_dir = Path(tmp) / 'conf'
            conf_dir.mkdir()
            for name, value in (conf or {}).items():
                (conf_dir / name).write_text(value + '\n', newline='\n')
            env = dict(os.environ, RUN=tmp, CONF=conf_dir.as_posix(), FALLBACK=str(int(fallback)), JOIN=str(int(join)))
            script = r'''
set -uo pipefail
IBSS=awdlibss0
AWDL_BSSID=00:25:00:ff:94:73
log() { echo "log $*" >> "$RUN/commands"; }
knob() { head -1 "$CONF/$1" 2>/dev/null | tr -d '[:space:]' || true; }
nmcli() {
  echo "nmcli $*" >> "$RUN/commands"
  [[ "$*" != '-g GENERAL.CON-UUID device show wlan0' ]] || echo 12345678-1234-1234-1234-123456789abc
}
ip() { echo "ip $*" >> "$RUN/commands"; }
sysctl() { echo "sysctl $*" >> "$RUN/commands"; [[ "$1" != -n ]] || echo 0; }
sleep() { :; }
cat() {
  if [[ "$1" == /sys/class/net/wlan0/address ]]; then
    echo 02:00:00:00:00:01
  else
    command cat "$@"
  fi
}
iw() {
  echo "iw $*" >> "$RUN/commands"
  if [[ "$*" == 'phy phy0 interface add awdlibss0 type ibss' ]]; then return "$FALLBACK"; fi
  if [[ "$*" == *'ibss join'* && "$JOIN" == 0 ]]; then return 1; fi
  if [[ "$*" == *' link' ]]; then echo 'Joined IBSS 00:25:00:ff:94:73'; fi
}
''' + FUNCTIONS + STATION + '\n' + extra + '\ntouch "$RUN/commands"; cat "$RUN/commands"\n'
            # As bytes over stdin: on Windows, bash -c's argument loses its
            # quoting, and text mode would turn every newline into CRLF.
            result = subprocess.run([BASH, '-s'], input=script.encode(), env=env, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            return result.stdout.decode()

    def test_separate_ibss_path_leaves_station_to_networkmanager(self):
        out = self.run_case('exclusive_up wlan0 phy0 2437 || exit 10\nexclusive_down', fallback=False)
        self.assertIn('iw dev awdlibss0 ibss join omdrop 2437 HT20', out)
        self.assertIn('iw dev awdlibss0 del', out)
        self.assertIn('nmcli device set wlan0 managed yes', out)
        self.assertNotIn('set type', out)
        self.assertNotIn('connection up', out)
        self.assertNotIn('device disconnect', out)

    def test_fallback_converts_station_and_restores_ipv6_and_connection(self):
        out = self.run_case('exclusive_up wlan0 phy0 2437 || exit 10\nexclusive_down\n'
                            '[ ! -e "$RUN/exclusive" ] && [ ! -e "$RUN/converted" ] || exit 11')
        self.assertIn('iw dev wlan0 set type ibss', out)
        self.assertIn('iw dev wlan0 ibss join omdrop 2437 HT20', out)
        self.assertIn('iw dev wlan0 set type managed', out)
        self.assertIn('sysctl -qw net.ipv6.conf.wlan0.disable_ipv6=0', out)
        self.assertIn('connection up uuid 12345678-1234-1234-1234-123456789abc ifname wlan0', out)
        self.assertNotIn('iw dev wlan0 del', out)

    def test_failed_join_also_restores_station(self):
        out = self.run_case('exclusive_up wlan0 phy0 2437 && exit 10\nexclusive_down', join=False)
        self.assertIn('iw dev wlan0 set type managed', out)
        self.assertIn('nmcli device set wlan0 managed yes', out)

    def test_failed_conversion_is_still_undone(self):
        out = self.run_case('''
iw_original=$(declare -f iw)
eval "${iw_original/iw ()/mock_iw ()}"
iw() { if [[ "$*" == 'dev wlan0 set type ibss' ]]; then return 1; fi; mock_iw "$@"; }
exclusive_up wlan0 phy0 2437 && exit 10
exclusive_down
''')
        self.assertIn('iw dev wlan0 set type managed', out)

    def test_idle_stop_does_not_touch_the_station(self):
        out = self.run_case('exclusive_down')
        self.assertNotIn('nmcli', out)
        self.assertNotIn('ip ', out)

    def test_probe_finds_converted_station_during_window(self):
        self.run_case('exclusive_up wlan0 phy0 2437 || exit 10\n[[ $(station_iface) == wlan0 ]] || exit 11')

    def test_failed_type_restore_keeps_state_for_next_try(self):
        out = self.run_case('''exclusive_up wlan0 phy0 2437 || exit 10
iw() { echo "iw $*" >> "$RUN/commands"; [[ "$*" != 'dev wlan0 set type managed' ]]; }
exclusive_down && exit 11
[[ -e "$RUN/exclusive" && -e "$RUN/converted" && -e "$RUN/connection" ]] || exit 12
''')
        self.assertNotIn('nmcli device set wlan0 managed yes', out)

    def test_reconnect_retries_while_networkmanager_catches_up(self):
        out = self.run_case('''exclusive_up wlan0 phy0 2437 || exit 10
attempts=0
nmcli() {
  echo "nmcli $*" >> "$RUN/commands"
  if [[ "$*" == '-w 10 connection up uuid '* ]]; then
    attempts=$((attempts + 1))
    [[ "$attempts" -ge 2 ]]
  fi
}
exclusive_down || exit 11
[[ "$attempts" == 2 ]] || exit 12
''')
        self.assertNotIn("didn't reconnect", out)

    def test_reconnect_retries_are_bounded(self):
        out = self.run_case('''exclusive_up wlan0 phy0 2437 || exit 10
attempts=0
nmcli() {
  echo "nmcli $*" >> "$RUN/commands"
  if [[ "$*" == '-w 10 connection up uuid '* ]]; then
    attempts=$((attempts + 1)); return 1
  fi
}
exclusive_down || exit 11
[[ "$attempts" == 3 ]] || exit 12
''')
        self.assertIn("didn't reconnect", out)

    def test_disconnected_station_does_not_activate_an_arbitrary_connection(self):
        out = self.run_case('''exclusive_up wlan0 phy0 2437 || exit 10
printf '%s' -- > "$RUN/connection"
exclusive_down || exit 11
''')
        self.assertNotIn('connection up uuid', out)
        self.assertIn('nmcli device set wlan0 managed yes', out)

    CHANNELS = r'''
iw() {
  if [[ "$*" == 'phy phy0 channels' ]]; then
    printf '\t* %s\n' '2437 MHz [6]' '5220 MHz [44]' '5745 MHz [149]'
  fi
}
tx_channels phy0 | tr '\n' ' '
'''

    def test_channels_default_to_5ghz_first(self):
        self.assertEqual(self.run_case(self.CHANNELS).split(), ['5220', '5745', '2437'])

    def test_exclusive_channels_knob_sets_the_order(self):
        out = self.run_case(self.CHANNELS, conf={'exclusive-channels': '6'})
        self.assertEqual(out.split(), ['2437'])
        out = self.run_case(self.CHANNELS, conf={'exclusive-channels': '149 6'})
        self.assertEqual(out.split(), ['5745', '2437'])

    JOINED_ON = r'''
iw() {
  echo "iw $*" >> "$RUN/commands"
  if [[ "$*" == 'dev wlan0 link' ]]; then printf '%%s\n' 'Joined IBSS 00:25:00:ff:94:73 (on wlan0)' '	freq: %s'; fi
}
'''

    def test_keep_cell_reuses_a_cell_already_joined(self):
        out = self.run_case(self.JOINED_ON % 2437 + '[[ $(exclusive_up wlan0 phy0 2437) == 2437 ]] || exit 10',
                            conf={'keep-cell': '1'})
        self.assertNotIn('ibss join', out)
        self.assertNotIn('ip link set wlan0 down', out)

    def test_keep_cell_ignores_a_cell_on_another_channel(self):
        out = self.run_case(self.JOINED_ON % 5220 + 'exclusive_up wlan0 phy0 2437 >/dev/null', conf={'keep-cell': '1'})
        self.assertIn('ibss join omdrop 2437 HT20', out)

    def test_without_keep_cell_a_joined_cell_is_joined_again(self):
        out = self.run_case(self.JOINED_ON % 2437 + 'exclusive_up wlan0 phy0 2437 >/dev/null')
        self.assertIn('ibss join omdrop 2437 HT20', out)

if __name__ == '__main__':
    unittest.main()
