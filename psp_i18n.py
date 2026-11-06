"""Small UTF-8 message catalogs shared by the GTK app and its backend."""

import json
import os
from pathlib import Path

LOCALE_DIR = Path(__file__).resolve().parent / 'locales'


def language(environ=None):
    env = os.environ if environ is None else environ
    # LANGUAGE may contain an ordered list, as with GNU gettext.
    candidates = env.get('LANGUAGE', '').split(':')
    candidates.append(env.get('LC_ALL') or env.get('LC_MESSAGES') or env.get('LANG') or 'en')
    for candidate in candidates:
        code = candidate.replace('-', '_').split('_')[0].split('.')[0].lower()
        if code in ('it', 'en'):
            return code
    return 'en'


def translator(code):
    try:
        messages = json.loads((LOCALE_DIR / (code + '.json')).read_text(encoding='utf-8'))
    except (OSError, ValueError):
        messages = {}
    return lambda message: messages.get(message, message)


LANGUAGE = language()
_ = translator(LANGUAGE)
