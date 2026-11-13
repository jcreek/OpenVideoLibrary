import sqlite3,tempfile,unittest
from pathlib import Path
from bench_suite import seed

class PrefixQueryTests(unittest.TestCase):
 def test_late_prefix_is_case_insensitive_and_uses_range_search(self):
  with tempfile.TemporaryDirectory() as directory:
   path=Path(directory)/'fixture.db';seed(path,10000)
   with sqlite3.connect(path) as db:
    query='SELECT id,title FROM items WHERE title LIKE ? ORDER BY title COLLATE NOCASE,id LIMIT 100 OFFSET ?'
    rows=db.execute(query,('movie 00009%',0)).fetchall()
    self.assertEqual(len(rows),100);self.assertEqual(rows[0][0],9000)
    plan=db.execute('EXPLAIN QUERY PLAN '+query,('movie 00009%',0)).fetchall()
    self.assertTrue(any('SEARCH' in row[3] and 'items_title' in row[3] for row in plan),plan)
    self.assertFalse(any('TEMP B-TREE' in row[3] for row in plan),plan)
if __name__=='__main__':unittest.main()
