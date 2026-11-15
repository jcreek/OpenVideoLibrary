import os
import tempfile
import unittest
from pathlib import Path

from validate_n100 import source_hash


class SourceHashTests(unittest.TestCase):
    def tree_hash(self, files):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'):
                (root / name).write_text(name)
            (root / 'crates').mkdir()
            (root / 'scripts').mkdir()
            for name, content in files.items():
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                (root / name).write_bytes(content)
            previous = os.getcwd()
            os.chdir(root)
            try:
                return source_hash()
            finally:
                os.chdir(previous)

    def test_file_boundaries_change_the_hash(self):
        merged = self.tree_hash({'scripts/zz-a': b'Xscripts/zz-bY'})
        split = self.tree_hash({'scripts/zz-a': b'X', 'scripts/zz-b': b'Y'})
        self.assertNotEqual(merged, split)

    def test_skips_files_rsync_does_not_stage(self):
        plain = self.tree_hash({'scripts/run.py': b'1'})
        noisy = self.tree_hash({'scripts/run.py': b'1', 'scripts/.DS_Store': b'x', 'crates/._x': b'y'})
        self.assertEqual(plain, noisy)
        self.assertEqual(plain, self.tree_hash({'scripts/run.py': b'1', 'scripts/__pycache__/run.pyc': b'z'}))
        self.assertNotEqual(plain, self.tree_hash({'scripts/run.py': b'2'}))

    def test_paths_and_crates_are_covered(self):
        base = self.tree_hash({'scripts/run.py': b'1', 'crates/lib.rs': b'a'})
        self.assertNotEqual(base, self.tree_hash({'scripts/other.py': b'1', 'crates/lib.rs': b'a'}))
        self.assertNotEqual(base, self.tree_hash({'scripts/run.py': b'1', 'crates/lib.rs': b'b'}))


if __name__ == '__main__':
    unittest.main()
