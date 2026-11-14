"""Sends an e-mail that a test run captured in the temporary directory."""

import json
import sys
from pathlib import Path

CAPTURED_JSON = Path('/tmp/captured-email.json')
PREVIEWS_DIR = Path('/tmp/email-previews')


def captured():
    if not CAPTURED_JSON.exists():
        print(f'error: {CAPTURED_JSON} not found', file=sys.stderr)
        sys.exit(1)
    return json.loads(CAPTURED_JSON.read_text())


def preview(arg):
    target = PREVIEWS_DIR / f'{arg}.html'
    if not target.exists():
        return None
    return target.read_text()
