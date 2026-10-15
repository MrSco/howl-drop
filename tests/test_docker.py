"""docker/ runs the add-on's image without the Supervisor, so it has to keep
up with addon/config.yaml: the same default options, and the same rights."""
import json
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]


def config_section(name):
    """The lines of a top-level section of addon/config.yaml."""
    lines = (ROOT / 'addon' / 'config.yaml').read_text(encoding='utf-8').splitlines()
    start = lines.index(f'{name}:') + 1
    out = []
    for line in lines[start:]:
        if line and not line.startswith(' '):
            break
        out.append(line)
    return out


def scalar(text):
    text = text.strip()
    if text == '[]':
        return []
    if text in ('true', 'false'):
        return text == 'true'
    if re.fullmatch(r'-?\d+', text):
        return int(text)
    if len(text) >= 2 and text[0] == text[-1] == '"':
        return text[1:-1]
    return text


class DockerTests(unittest.TestCase):
    def test_options_match_the_add_on_defaults(self):
        defaults = {}
        for line in config_section('options'):
            key, _, value = line.strip().partition(':')
            defaults[key] = scalar(value)
        options = json.loads((ROOT / 'docker' / 'options.json').read_text(encoding='utf-8'))
        self.assertEqual(options, defaults)

    def test_compose_asks_for_what_the_add_on_does(self):
        compose = (ROOT / 'docker' / 'compose.yaml').read_text(encoding='utf-8')
        for line in config_section('privileged'):
            self.assertIn(f'- {line.strip(" -")}', compose)
        for needed in ('network_mode: host', 'pid: host', '/dev/net/tun', 'apparmor:unconfined',
                       '/run/dbus:/run/dbus', ':/data', ':/share/howldrop'):
            self.assertIn(needed, compose)


if __name__ == '__main__':
    unittest.main()
