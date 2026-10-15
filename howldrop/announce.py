"""Tell Home Assistant about each delivered drop: a howldrop_received event,
for automations, and notifications to the targets in the notify option.

Calls go through the Supervisor's proxy to Home Assistant's REST API
(homeassistant_api in config.yaml; the Supervisor puts SUPERVISOR_TOKEN in
the add-on's environment). Nothing here can hold up or undo a delivery: a
failed call is a log line.

Who sent a file comes from the note the receiver leaves in staging,
.howldrop-meta/<stored name>.json (patch_receiver.py, howl_note).
"""
import json
import os
import time
import urllib.error
import urllib.request

API = os.environ.get('HOWLDROP_HA_URL', 'http://supervisor/core/api')
TIMEOUT_S = 5
TEXT_PREVIEW = 300
HISTORY_KEPT = 1000

IMAGE = {'.jpg', '.jpeg', '.png', '.heic', '.heif', '.gif', '.webp', '.tif', '.tiff', '.bmp', '.dng'}
VIDEO = {'.mov', '.mp4', '.m4v', '.avi', '.mkv', '.webm', '.hevc'}
WORDS = {'image': 'Photo', 'video': 'Video', 'text': 'Text', 'link': 'Link', 'file': 'File'}


def kind_of(path, url=None):
    ext = os.path.splitext(path)[1].lower()
    if url or ext == '.url':
        return 'link'
    if os.path.basename(os.path.dirname(path)) == 'text':
        return 'text'
    if ext in IMAGE:
        return 'image'
    if ext in VIDEO:
        return 'video'
    return 'file'


def read_note(staging, name):
    """The receiver's note for a staged file, removed once read: (sender, url)."""
    note = os.path.join(staging, '.howldrop-meta', name + '.json')
    try:
        with open(note, encoding='utf-8') as fh:
            data = json.load(fh)
        os.unlink(note)
    except (OSError, ValueError):
        return '', None
    return str(data.get('sender') or ''), data.get('url')


def prune_notes(staging, older_than_s, now=None):
    """Remove notes whose file never arrived (a failed upload leaves one)."""
    meta = os.path.join(staging, '.howldrop-meta')
    now = time.time() if now is None else now
    try:
        names = os.listdir(meta)
    except OSError:
        return 0
    gone = 0
    for name in names:
        note = os.path.join(meta, name)
        try:
            if now - os.path.getmtime(note) > older_than_s:
                os.unlink(note)
                gone += 1
        except OSError:
            pass
    return gone


def record(history, path, dest, sender, url=None, now=None):
    """Note a delivered file in history, the file the panel (panel.py) reads
    senders from: one JSON line each, newest last, the last HISTORY_KEPT
    kept. No history, or a failed write, records nothing."""
    if not history:
        return
    entry = {'path': os.path.relpath(path, dest), 'sender': sender or '', 'time': int(now or time.time())}
    if url:
        entry['url'] = url
    try:
        os.makedirs(os.path.dirname(history) or '.', exist_ok=True)
        with open(history, 'a', encoding='utf-8') as fh:
            fh.write(json.dumps(entry) + '\n')
        if os.path.getsize(history) > HISTORY_KEPT * 400:
            with open(history, encoding='utf-8') as fh:
                lines = fh.readlines()[-HISTORY_KEPT:]
            with open(history + '.new', 'w', encoding='utf-8') as fh:
                fh.writelines(lines)
            os.replace(history + '.new', history)
    except OSError:
        pass


def senders(history):
    """{path relative to save_folder: sender} from history; the latest wins."""
    out = {}
    try:
        with open(history, encoding='utf-8') as fh:
            for line in fh:
                try:
                    entry = json.loads(line)
                    out[str(entry['path'])] = str(entry.get('sender') or '')
                except (ValueError, KeyError, TypeError):
                    continue
    except OSError:
        pass
    return out


def text_preview(path):
    try:
        with open(path, encoding='utf-8', errors='replace') as fh:
            text = fh.read(TEXT_PREVIEW + 1)
    except OSError:
        return ''
    return text if len(text) <= TEXT_PREVIEW else text[:TEXT_PREVIEW].rstrip() + '…'


class Announcer:
    def __init__(self, targets, token=None, api=API):
        self.targets = [t.strip() for t in targets if t and t.strip()]
        # Under Docker without the Supervisor (docker/), a long-lived access
        # token and HOWLDROP_HA_URL reach a Home Assistant elsewhere.
        self.token = token if token is not None else (
            os.environ.get('SUPERVISOR_TOKEN') or os.environ.get('HOWLDROP_HA_TOKEN', ''))
        self.api = api.rstrip('/')

    @property
    def enabled(self):
        return bool(self.token)

    def _post(self, path, payload):
        req = urllib.request.Request(
            f'{self.api}/{path}', data=json.dumps(payload).encode(), method='POST',
            headers={'Authorization': f'Bearer {self.token}', 'Content-Type': 'application/json'})
        with urllib.request.urlopen(req, timeout=TIMEOUT_S) as response:
            response.read()

    def drop(self, path, dest, sender='', url=None):
        """Announce the delivered file at path; returns lines for the log."""
        if not self.enabled:
            return []
        kind = kind_of(path, url)
        name = os.path.basename(path)
        try:
            size = os.path.getsize(path)
        except OSError:
            size = None
        event = {'sender': sender, 'name': name, 'path': path, 'folder': dest,
                 'kind': kind, 'size': size, 'url': url}
        lines = []
        try:
            self._post('events/howldrop_received', event)
        except (OSError, urllib.error.URLError) as e:
            lines.append(f'Home Assistant event failed: {e}')

        who = f' from {sender}' if sender else ''
        if kind == 'text':
            message = f'{WORDS[kind]}{who}: {text_preview(path)}'
        elif kind == 'link' and url:
            message = f'{WORDS[kind]}{who}: {url}'
        else:
            message = f'{WORDS[kind]}{who}: {name}'
        for target in self.targets:
            try:
                if target in ('persistent_notification', 'persistent_notification.create'):
                    self._post('services/persistent_notification/create',
                               {'title': 'HowlDrop', 'message': message})
                else:
                    service = target.split('.', 1)[1] if target.startswith('notify.') else target
                    payload = {'title': 'HowlDrop', 'message': message}
                    if kind == 'link' and url:
                        payload['data'] = {'url': url, 'clickAction': url}
                    self._post(f'services/notify/{service}', payload)
            except (OSError, urllib.error.URLError) as e:
                lines.append(f'notify {target} failed: {e}')
        return lines
