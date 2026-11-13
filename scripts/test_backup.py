import importlib.util,sqlite3,tempfile,unittest
from pathlib import Path
spec=importlib.util.spec_from_file_location('backup',Path(__file__).with_name('backup.py'));backup=importlib.util.module_from_spec(spec);spec.loader.exec_module(backup)
class BackupTests(unittest.TestCase):
 def test_live_wal_roundtrip_and_corruption(self):
  with tempfile.TemporaryDirectory() as tmp:
   root=Path(tmp);data=root/'data';data.mkdir();db=sqlite3.connect(data/'library.db');db.executescript("PRAGMA journal_mode=WAL;PRAGMA user_version=1;CREATE TABLE items(id INTEGER PRIMARY KEY,title TEXT);CREATE TABLE sessions(token TEXT);INSERT INTO items VALUES(1,'Sample');INSERT INTO sessions VALUES('old-session');");db.commit()
   backup.backup(data,root/'snapshot');backup.restore(root/'snapshot',root/'restored')
   with sqlite3.connect(root/'restored/library.db') as restored:
    self.assertEqual(restored.execute('SELECT title FROM items').fetchone()[0],'Sample');self.assertEqual(restored.execute('SELECT count(*) FROM sessions').fetchone()[0],0)
   with self.assertRaises(ValueError):backup.restore(root/'snapshot',root/'restored')
   with (root/'snapshot/library.db').open('ab') as f:f.write(b'corrupt')
   with self.assertRaises(ValueError):backup.restore(root/'snapshot',root/'corrupt')
   self.assertFalse((root/'corrupt').exists());db.close()
 def test_reject_nested_backup(self):
  with tempfile.TemporaryDirectory() as tmp:
   with self.assertRaises(ValueError):backup.backup(tmp,Path(tmp)/'nested')
if __name__=='__main__':unittest.main()
