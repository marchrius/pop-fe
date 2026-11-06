"""Persistent last folder shared by all file and folder dialogs."""

import json
import os
from pathlib import Path
import tempfile


class FolderHistory:
    def __init__(self, config_file, home=None):
        self.config_file = Path(config_file)
        self.home = Path.home() if home is None else Path(home)
        self.last_directory = self.home
        try:
            value = json.loads(self.config_file.read_text(encoding='utf-8')).get('last_directory')
            if isinstance(value, str) and value and Path(value).is_absolute():
                self.last_directory = Path(value)
        except (OSError, ValueError, TypeError, AttributeError):
            pass

    @staticmethod
    def accessible(directory):
        try:
            if not directory.is_dir() or not os.access(directory, os.R_OK | os.X_OK):
                return False
            with os.scandir(directory):
                return True
        except (OSError, ValueError):
            return False

    def current(self):
        # Recheck each time: removable media may have disappeared since launch.
        return self.last_directory if self.accessible(self.last_directory) else self.home

    def remember(self, selected, *, directory=False):
        if not selected:
            return
        path = Path(selected)
        candidate = path if directory else path.parent
        if not candidate.is_absolute() or not self.accessible(candidate):
            return
        self.last_directory = candidate
        temporary = None
        try:
            self.config_file.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.NamedTemporaryFile(mode='w', encoding='utf-8',
                                             dir=self.config_file.parent, delete=False) as stream:
                temporary = Path(stream.name)
                json.dump({'last_directory': str(candidate)}, stream)
                stream.write('\n')
            temporary.replace(self.config_file)
        except OSError:
            # Keep the in-memory preference when settings cannot be written.
            pass
        finally:
            if temporary is not None:
                try:
                    temporary.unlink(missing_ok=True)
                except OSError:
                    pass
