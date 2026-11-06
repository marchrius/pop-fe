import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from PIL import Image
import psp_backend


class XmbPreviewTests(unittest.TestCase):
    def test_composition_uses_real_assets(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            assets = {}
            for key, color in (("pic1", "blue"), ("cover", "red"), ("pic0", "green")):
                filename = root / (key + '.png')
                Image.new('RGBA', (80, 80), color).save(filename)
                assets[key] = str(filename)
            result = psp_backend.preview_xmb({'assets': assets}, root / 'output')
            with Image.open(result['image']) as image:
                self.assertEqual(image.size, (480, 272))
                self.assertEqual(image.getpixel((0, 0)), (0, 0, 255, 255))
                self.assertEqual(image.getpixel((50, 110)), (255, 0, 0, 255))
                self.assertEqual(image.getpixel((200, 110)), (0, 128, 0, 255))

    def test_selected_theme_replaces_available_assets_and_keeps_fallback(self):
        class ThemeBackend:
            @staticmethod
            def get_image_from_theme(theme, disc_id, directory, filename):
                return Image.new('RGBA', (10, 10), 'yellow') if filename == 'PIC1.PNG' else None

        with tempfile.TemporaryDirectory() as tmp, patch.object(psp_backend, 'popfe_module', return_value=ThemeBackend()):
            config = {'theme': 'TEST', 'disc_ids': ['SLUS00000'], 'assets': {'cover': '/original.png'}}
            result = psp_backend.resolve_theme_assets(config, Path(tmp))
            self.assertEqual(result['cover'], '/original.png')
            with Image.open(result['pic1']) as image:
                self.assertEqual(image.getpixel((0, 0)), (255, 255, 0, 255))


if __name__ == '__main__':
    unittest.main()
