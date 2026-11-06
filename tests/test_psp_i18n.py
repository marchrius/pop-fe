import ast
import json
from pathlib import Path
import string
import unittest

from psp_i18n import language, translator

ROOT = Path(__file__).resolve().parents[1]


class LocalizationTests(unittest.TestCase):
    def test_locale_selection_and_fallback(self):
        for env, expected in (
            ({}, 'en'), ({'LANG': 'it_IT.UTF-8'}, 'it'),
            ({'LANG': 'en_US.UTF-8'}, 'en'), ({'LANG': 'de_DE.UTF-8'}, 'en'),
            ({'LANG': 'it_IT', 'LC_ALL': 'en_US'}, 'en'),
            ({'LANG': 'en_US', 'LC_MESSAGES': 'it_IT'}, 'it'),
            ({'LANGUAGE': 'fr:it:en'}, 'it'),
            ({'LANGUAGE': 'en', 'LANG': 'it_IT'}, 'en'),
        ):
            with self.subTest(env=env):
                self.assertEqual(language(env), expected)

    def test_catalogs_cover_ui_and_keep_placeholders(self):
        catalogs = {code: json.loads((ROOT / 'locales' / (code + '.json')).read_text()) for code in ('en', 'it')}
        self.assertEqual(catalogs['en'].keys(), catalogs['it'].keys())
        for filename in ('psp_gui_gtk.py', 'psp_backend.py'):
            for node in ast.walk(ast.parse((ROOT / filename).read_text())):
                if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == '_':
                    self.assertIn(ast.literal_eval(node.args[0]), catalogs['en'])
        formatter = string.Formatter()
        for key, translated in catalogs['it'].items():
            expected = {field for _, field, _, _ in formatter.parse(key) if field}
            actual = {field for _, field, _, _ in formatter.parse(translated) if field}
            self.assertEqual(expected, actual, key)

    def test_translation_and_missing_message(self):
        self.assertEqual(translator('it')('Ready'), 'Pronto')
        self.assertEqual(translator('en')('Ready'), 'Ready')
        self.assertEqual(translator('it')('Untranslated message'), 'Untranslated message')


if __name__ == '__main__':
    unittest.main()
