"""Exercise the supervisor's loop with command doubles, never the real radio."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

# Through PATH: Windows itself looks in System32 first, which is WSL's bash.
BASH = shutil.which('bash') or 'bash'
SOURCE = (Path(__file__).resolve().parents[1] / 'userspace/omdrop-discoverable').read_text()
SUPERVISE = SOURCE[SOURCE.index('supervise()'):SOURCE.index('\ncmd_start()')]

class SuperviseTests(unittest.TestCase):
    def run_case(self, until=0, owl=True, mon=True, hostapd=None):
        """Run the loop until it ends the window; returns what it did."""
        with tempfile.TemporaryDirectory() as tmp:
            stop = Path(tmp, 'stop')
            stop.write_text('#!/bin/sh\necho "$*" >> "$RUN/commands"\n', newline='\n')
            stop.chmod(0o755)
            Path(tmp, 'until').write_text(f'{until}\n')
            if hostapd is not None:
                Path(tmp, 'hostapd.pid').write_text('1\n')
            env = dict(os.environ, RUN=Path(tmp).as_posix(), SELF=stop.as_posix(), OWL=str(int(owl)),
                       MON_UP=str(int(mon)), HOSTAPD=str(int(bool(hostapd))))
            script = r'''
set -uo pipefail
IF=awdl0
MON=mon0
LIB=/usr/lib/omdrop
OWL_BIN=$LIB/owl
OWL_LOG=$RUN/owl.log
ADV_FLAGS=137
ADV_INTERVAL=3
ADV_BURST="--count 3 --interval 0.05"
ADV_BURST_EXCLUSIVE="--count 64 --interval 0.0164"
rounds=0
log() { echo "log $*" >> "$RUN/commands"; }
pid_of() { echo 1; }
alive() { if [[ "${2:-}" == hostapd ]]; then [[ "$HOSTAPD" == 1 ]]; else [[ "$OWL" == 1 ]]; fi; }
ip() { [[ "$*" == 'link show mon0' && "$MON_UP" == 1 ]]; }
python3() { echo announce >> "$RUN/commands"; }
# A healthy window never ends by itself: two rounds are enough to see it go on.
sleep() { rounds=$((rounds + 1)); [[ "$rounds" -lt 2 ]] || exit 0; }
''' + SUPERVISE + '\nsupervise\n'
            result = subprocess.run([BASH, '-s'], input=script.encode(), env=env, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr.decode(errors='replace'))
            commands = Path(tmp, 'commands')
            return commands.read_text() if commands.exists() else ''

    def test_healthy_window_keeps_announcing(self):
        out = self.run_case()
        self.assertEqual(out.count('announce'), 2)
        self.assertNotIn('stop', out)

    def test_expired_window_stops(self):
        out = self.run_case(until=1)
        self.assertIn('stop expired', out)
        self.assertNotIn('announce', out)

    def test_dead_owl_stops(self):
        out = self.run_case(owl=False)
        self.assertIn('stop owl-exited', out)
        self.assertNotIn('announce', out)

    def test_vanished_monitor_interface_stops(self):
        out = self.run_case(mon=False)
        self.assertIn('log mon0 is gone', out)
        self.assertIn('stop radio-lost', out)
        self.assertNotIn('announce', out)

    def test_dead_hostapd_stops(self):
        out = self.run_case(hostapd=False)
        self.assertIn('log hostapd exited', out)
        self.assertIn('stop hostapd-exited', out)
        self.assertNotIn('announce', out)

    def test_running_hostapd_keeps_announcing(self):
        out = self.run_case(hostapd=True)
        self.assertEqual(out.count('announce'), 2)
        self.assertNotIn('stop', out)

if __name__ == '__main__':
    unittest.main()
