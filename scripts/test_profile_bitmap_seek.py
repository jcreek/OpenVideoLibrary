import os
from pathlib import Path
import sys
import tempfile
import unittest
from profile_bitmap_seek import run

class ProfileChildBounds(unittest.TestCase):
    def assert_reaped(self, marker):
        pid = int(marker.read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)

    def test_output_overflow_kills_and_reaps(self):
        with tempfile.TemporaryDirectory() as folder:
            marker = Path(folder) / 'child.pid'
            source = f"import os;open({str(marker)!r},'w').write(str(os.getpid()));os.write(1,b'x'*(2<<20));__import__('time').sleep(60)"
            with self.assertRaisesRegex(RuntimeError, 'output limit'):
                run([sys.executable, '-c', source], timeout=5)
            self.assert_reaped(marker)

    def test_deadline_kills_and_reaps(self):
        with tempfile.TemporaryDirectory() as folder:
            marker = Path(folder) / 'child.pid'
            source = f"import os,time;open({str(marker)!r},'w').write(str(os.getpid()));time.sleep(60)"
            with self.assertRaises(TimeoutError):
                run([sys.executable, '-c', source], timeout=1)
            self.assert_reaped(marker)

if __name__ == '__main__':
    unittest.main()
