#!/usr/bin/env python3
"""Repair one library's paths in a fresh restore of a verified v2 backup."""
import argparse
from contextlib import closing
import json
from pathlib import Path
import shutil
import sqlite3
import stat
import tempfile
import time

from backup import restore, validate

MAX_FILES = 1_000_000


def repair(snapshot, destination, library_id, new_root):
    source = Path(snapshot).resolve(strict=True)
    requested = Path(destination)
    destination = requested.parent.resolve(strict=True) / requested.name
    if destination.exists() or destination.is_symlink():
        raise ValueError('destination must not exist')
    if destination.is_relative_to(source):
        raise ValueError('destination must be outside the backup')
    new_root = Path(new_root)
    if not new_root.is_absolute() or new_root.resolve(strict=True) != new_root or not new_root.is_dir():
        raise ValueError('new root must be an existing canonical directory')
    if any(path.is_relative_to(new_root) or new_root.is_relative_to(path)
           for path in (source, destination)):
        raise ValueError('media root must be separate from backup and destination')

    # Validate and modify a private copy before creating the requested destination.
    with tempfile.TemporaryDirectory(prefix='.ovl-path-repair-', dir=destination.parent) as temp:
        data = Path(temp) / 'restore'
        restore(source, data)
        deadline = time.monotonic() + 1800
        with closing(sqlite3.connect(data / 'library.db', timeout=5)) as db, db:
            if db.execute('PRAGMA user_version').fetchone()[0] != 2:
                raise ValueError('repair requires a v2 backup')
            db.execute('PRAGMA foreign_keys=ON')
            db.execute('BEGIN IMMEDIATE')
            row = db.execute('SELECT path FROM libraries WHERE id=?', (library_id,)).fetchone()
            if row is None:
                raise ValueError('library ID not found')
            old_root = Path(row[0])
            if not old_root.is_absolute() or '..' in old_root.parts or old_root == new_root:
                raise ValueError('expected a distinct absolute stored library path')
            checked = 0
            last = -(1 << 63)
            comparison = ">="
            while True:
                if time.monotonic() > deadline:
                    raise ValueError('path repair exceeded time budget')
                rows = db.execute(
                    'SELECT id,path,size FROM media_files NOT INDEXED WHERE library_id=? '
                    f'AND id{comparison}? ORDER BY id LIMIT 128',
                    (library_id, last)).fetchall()
                if not rows:
                    break
                for file_id, raw, size in rows:
                    if checked >= MAX_FILES or time.monotonic() > deadline:
                        raise ValueError('path repair exceeded work budget')
                    old_path = Path(raw)
                    if not old_path.is_relative_to(old_root) or '..' in old_path.parts:
                        raise ValueError(f'file {file_id} is outside the stored root')
                    candidate = new_root / old_path.relative_to(old_root)
                    try:
                        info = candidate.stat()
                        regular = candidate.resolve(strict=True) == candidate and stat.S_ISREG(info.st_mode)
                    except OSError:
                        regular = False
                    if not regular or info.st_size != size:
                        raise ValueError(f'file {file_id} missing, redirected, or different size')
                    db.execute('UPDATE media_files SET path=? WHERE id=?', (str(candidate), file_id))
                    checked += 1
                last = rows[-1][0]
                comparison = ">"
            if checked == 0:
                raise ValueError('library has no paths to verify')
            db.execute('UPDATE libraries SET path=? WHERE id=?', (str(new_root), library_id))
            validate(db)
        # mkdir refuses a conflicting destination rather than replacing it.
        destination.mkdir(mode=0o700)
        try:
            for entry in data.iterdir():
                entry.rename(destination / entry.name)
        except BaseException:
            shutil.rmtree(destination)
            raise
    return {'library_id': library_id, 'verified_file_count': checked, 'restored_data': str(destination)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--backup', required=True)
    parser.add_argument('--output', required=True, help='new data directory; parent must exist')
    parser.add_argument('--library-id', required=True, type=int)
    parser.add_argument('--root', required=True, help='verified canonical media directory')
    args = parser.parse_args()
    try:
        result = repair(args.backup, args.output, args.library_id, args.root)
    except (ValueError, OSError, sqlite3.Error) as error:
        parser.exit(1, f'{error}\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
