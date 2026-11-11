import sqlite3
import tempfile
import unittest
from contextlib import closing
from pathlib import Path

from backup import backup, checksum
from repair_library_path import repair

SCHEMA = Path(__file__).resolve().parent.parent / 'crates/server/src'


class RepairLibraryTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name).resolve()
        self.data = self.root / 'data'
        self.data.mkdir()
        self.snapshot = self.root / 'snapshot'
        self.out = self.root / 'restored'
        self.media = self.root / 'canonical'
        self.media.mkdir()
        self.track = self.media / 'track.mp3'
        self.track.write_bytes(b'fixture')
        self.old = self.root / 'alias'
        self.old.symlink_to(self.media, target_is_directory=True)
        with closing(sqlite3.connect(self.data / 'library.db')) as db, db:
            db.executescript((SCHEMA / 'schema.sql').read_text())
            db.executescript((SCHEMA / 'migrations/002_media_files.sql').read_text())
            db.executescript((SCHEMA / 'migrations/expansion.sql').read_text())
            db.execute("INSERT INTO users VALUES(7,'viewer',X'1234',0)")
            db.execute("INSERT INTO sessions VALUES('obsolete',7,1000)")
            db.execute("INSERT INTO libraries VALUES(4,'Music',?,'movie',0)", (str(self.old),))
            db.execute("INSERT INTO ovl_music_libraries VALUES(4)")
            db.execute("INSERT INTO access VALUES(7,4)")
            db.execute("INSERT INTO items(id,title) VALUES(23,'A track')")
            db.execute("INSERT INTO media_files VALUES(41,23,4,?,123,7)", (str(self.old/'track.mp3'),))
            db.execute("INSERT INTO progress VALUES(7,23,42,0,1)")
            db.execute("INSERT INTO ovl_lists VALUES(5,7,'queue','Saved',2,0,1)")
            db.execute("INSERT INTO ovl_list_entries VALUES(5,0,23)")
        backup(self.data, self.snapshot)

    def test_repairs_paths_without_replacing_ids_or_snapshot(self):
        original = checksum(self.snapshot / 'library.db')
        result = repair(self.snapshot, self.out, 4, self.media)
        self.assertEqual(result['verified_file_count'], 1)
        self.assertEqual(checksum(self.snapshot / 'library.db'), original)
        with closing(sqlite3.connect(self.out / 'library.db')) as db:
            self.assertEqual(db.execute('SELECT id,path FROM libraries').fetchone(), (4, str(self.media)))
            self.assertEqual(db.execute('SELECT * FROM media_files').fetchone(), (41,23,4,str(self.track),123,7))
            for query, expected in [
                ('SELECT * FROM ovl_music_libraries', (4,)),
                ('SELECT * FROM access', (7,4)),
                ('SELECT user_id,item_id,position FROM progress', (7,23,42)),
                ('SELECT * FROM ovl_list_entries', (5,0,23)),
                ('SELECT password FROM users', (b'\x12\x34',)),
                ('SELECT count(*) FROM sessions', (0,)),
            ]:
                self.assertEqual(db.execute(query).fetchone(), expected)
        self.assertEqual(self.track.read_bytes(), b'fixture')

    def test_rejects_redirected_or_changed_target_without_output(self):
        self.track.unlink()
        outside = self.root / 'outside.mp3'
        outside.write_bytes(b'fixture')
        self.track.symlink_to(outside)
        with self.assertRaises(ValueError):
            repair(self.snapshot, self.out, 4, self.media)
        self.assertFalse(self.out.exists())
        self.track.unlink()
        self.track.write_bytes(b'changed size')
        with self.assertRaises(ValueError):
            repair(self.snapshot, self.out, 4, self.media)
        self.assertFalse(self.out.exists())

    def test_refuses_existing_destination(self):
        self.out.mkdir()
        sentinel = self.out / 'keep'
        sentinel.write_text('keep')
        with self.assertRaises(ValueError):
            repair(self.snapshot, self.out, 4, self.media)
        self.assertEqual(sentinel.read_text(), 'keep')
