import importlib.machinery
import importlib.util
import pathlib
import tempfile
import unittest
from unittest.mock import patch


path = pathlib.Path(__file__).resolve().parents[1] / 'root/usr/libexec/gh0st-boot'
loader = importlib.machinery.SourceFileLoader('gh0st_boot', str(path))
spec = importlib.util.spec_from_loader(loader.name, loader)
boot = importlib.util.module_from_spec(spec)
loader.exec_module(boot)


class BootTest(unittest.TestCase):
    def test_frames_fit_panel_and_animate(self):
        images = boot.frames()
        self.assertEqual(len(images), 16)
        self.assertTrue(all(len(frame) == boot.FRAME_BYTES for frame in images))
        self.assertEqual(len(set(images)), 16)
        self.assertEqual(images[0][:2], b'\x00\x00')

    def test_play_writes_last_frame_when_present_is_unavailable(self):
        with tempfile.NamedTemporaryFile() as fb:
            fb.truncate(boot.FRAME_BYTES)
            with patch.object(boot.time, 'sleep'), patch.object(boot.fcntl, 'ioctl', side_effect=OSError()):
                boot.play(fb.name)
            self.assertEqual(pathlib.Path(fb.name).read_bytes(), boot.frames()[-1])


if __name__ == '__main__':
    unittest.main()
