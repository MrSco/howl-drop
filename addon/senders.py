#!/usr/bin/env python3
"""Turn the add-on's `senders` option into the receiver's Contacts Only files.

omdrop-plugin's receiver reads two files from ~/.config/omdrop:
  visibility     `contacts` or `everyone`
  known-senders  one phone number or email per line, in the clear
In `contacts` mode it answers /Ask only for a sender whose Apple-signed
validation record carries one of those identifiers, hashed, and whose TLS
client certificate Apple issued to the same Apple ID; anyone else gets 403.
It also stays off the share sheet of a sender it can identify as unknown.

An empty `senders` means everyone nearby, as before. The files are rewritten
on every start, so removing the last sender turns the restriction off.

The log names counts, never identifiers.

  senders.py --options /data/options.json --config /root/.config/omdrop
"""
import argparse
import json
import re
import sys
from pathlib import Path


def entries(value):
    """The option as a list of strings: a list from the add-on UI, or one
    string separated by commas, semicolons or new lines."""
    if value is None:
        return []
    if isinstance(value, str):
        value = re.split(r'[,;\n]', value)
    if not isinstance(value, list):
        return []
    return [str(v).strip() for v in value if str(v).strip()]


def normalize(entry):
    """What contacts.py hashes: an email lowercased, a phone number as digits.

    Returns (kind, value, warning); value None means the entry is unusable.
    """
    if '@' in entry:
        local, _, domain = entry.partition('@')
        if not local or '.' not in domain:
            return 'email', None, f'{entry!r} is not an email address'
        return 'email', entry.lower(), None
    digits = re.sub(r'\D', '', entry)
    if len(digits) < 7:
        return 'phone', None, f'{entry!r} is neither an email address nor a phone number'
    warning = None
    if not entry.lstrip().startswith('+'):
        # Apple hashes the number with its country code. "555 123 4567"
        # never matches a phone that is +1 555 123 4567.
        warning = (f'a phone number without "+" ({len(digits)} digits) is matched exactly as written: '
                   'include the country code, e.g. +1 555 123 4567')
    return 'phone', digits, warning


def write(config, options):
    config = Path(config)
    config.mkdir(parents=True, exist_ok=True)
    phones, emails, notes = [], [], []
    for entry in entries(options.get('senders')):
        kind, value, warning = normalize(entry)
        if warning:
            notes.append(warning)
        if value is None:
            continue
        bucket = emails if kind == 'email' else phones
        if value not in bucket:
            bucket.append(value)
    known = config / 'known-senders'
    visibility = config / 'visibility'
    if not phones and not emails:
        visibility.write_text('everyone\n')
        known.unlink(missing_ok=True)
        lines = ['Senders: everyone nearby can send (the senders option is empty).']
    else:
        known.write_text(''.join(f'{v}\n' for v in phones + emails))
        known.chmod(0o600)
        visibility.write_text('contacts\n')
        lines = [f'Senders: Contacts Only. {len(phones)} phone number(s) and {len(emails)} email address(es) '
                 'may send; an iPhone signed in to another Apple ID is refused.']
    return lines + [f'Senders: {n}' for n in notes]


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument('--options', default='/data/options.json')
    p.add_argument('--config', required=True)
    args = p.parse_args(argv)
    try:
        with open(args.options) as handle:
            options = json.load(handle)
    except FileNotFoundError:
        options = {}
    for line in write(args.config, options):
        print(line, flush=True)
    return 0


if __name__ == '__main__':
    sys.exit(main())
