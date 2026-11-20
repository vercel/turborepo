"""Prints a message that a test run left in the outbox of the temporary directory."""

import json
import sys
from pathlib import Path

OUTBOX = Path('/tmp/outbox.json')
RENDERED = Path('/tmp/outbox-rendered')


def message():
    try:
        return json.loads(OUTBOX.read_text())
    except FileNotFoundError:
        sys.exit(f'nothing in {OUTBOX}')


def rendered(name):
    page = RENDERED / (name + '.txt')
    return page.read_text() if page.is_file() else None
