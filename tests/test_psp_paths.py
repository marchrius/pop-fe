import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from psp_paths import FolderHistory


class FolderHistoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = self.root / 'home'
        self.home.mkdir()
        self.config = self.root / 'settings/dialogs.json'

    def history(self):
        return FolderHistory(self.config, home=self.home)

    def test_first_use_and_file_selection_survive_restart(self):
        folder = self.root / 'discs'
        folder.mkdir()
        history = self.history()
        self.assertEqual(history.current(), self.home)
        history.remember(str(folder / 'game.cue'))
        self.assertEqual(history.current(), folder)
        self.assertEqual(self.history().current(), folder)

    def test_folder_selection_and_cancel(self):
        folder = self.root / 'PSP'
        folder.mkdir()
        history = self.history()
        history.remember(str(folder), directory=True)
        history.remember(None)
        history.remember('')
        self.assertEqual(self.history().current(), folder)

    def test_removed_or_unreadable_folder_falls_back(self):
        folder = self.root / 'removable'
        folder.mkdir()
        history = self.history()
        history.remember(str(folder), directory=True)
        with patch('psp_paths.os.access', return_value=False):
            self.assertEqual(history.current(), self.home)
        folder.rmdir()
        self.assertEqual(history.current(), self.home)
        self.assertEqual(self.history().current(), self.home)

    def test_corrupt_settings_fall_back(self):
        self.config.parent.mkdir()
        for value in ('broken JSON', 'null', '[]', '{"last_directory": ""}',
                      '{"last_directory": "."}', '{"last_directory": 10}'):
            self.config.write_text(value)
            self.assertEqual(self.history().current(), self.home)

    def test_settings_write_failure_keeps_memory(self):
        history = self.history()
        with patch('psp_paths.tempfile.NamedTemporaryFile', side_effect=PermissionError):
            history.remember(str(self.root), directory=True)
        self.assertEqual(history.current(), self.root)


if __name__ == '__main__':
    unittest.main()
