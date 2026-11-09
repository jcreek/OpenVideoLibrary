#!/usr/bin/env python3
"""Consistent SQLite snapshots and verified restore into a fresh directory."""
import argparse, hashlib, json, os, shutil, sqlite3, tempfile
from pathlib import Path

def checksum(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda:f.read(1024*1024),b''):h.update(block)
    return h.hexdigest()
def validate(db):
    if db.execute('PRAGMA integrity_check').fetchone()[0]!='ok':raise ValueError('database integrity check failed')
    if db.execute('PRAGMA user_version').fetchone()[0] not in (1,2):raise ValueError('unsupported schema version')
    if db.execute('PRAGMA foreign_key_check').fetchone():raise ValueError('foreign key check failed')
def backup(data,output):
    data,output=Path(data).resolve(),Path(output).resolve()
    if output.exists():raise ValueError('backup destination already exists')
    if data==output or data in output.parents:raise ValueError('backup must be outside the data directory')
    output.parent.mkdir(parents=True,exist_ok=True)
    staging=Path(tempfile.mkdtemp(prefix='.ovl-backup-',dir=output.parent));os.chmod(staging,0o700)
    try:
        with sqlite3.connect((data/'library.db').as_uri()+'?mode=ro',uri=True) as source:
            with sqlite3.connect(staging/'library.db') as dest:source.backup(dest);validate(dest)
        os.chmod(staging/'library.db',0o600)
        art=data/'artwork'
        if art.is_dir():
            (staging/'artwork').mkdir(mode=0o700)
            for p in art.iterdir():
                if p.is_file() and not p.is_symlink() and p.suffix=='.jpg':shutil.copy2(p,staging/'artwork'/p.name)
        files={str(p.relative_to(staging)):checksum(p) for p in staging.rglob('*') if p.is_file()}
        (staging/'manifest.json').write_text(json.dumps({'format':1,'files':files},indent=2)+'\n')
        staging.rename(output)
    except BaseException:
        shutil.rmtree(staging);raise

def restore(source,destination):
    source,destination=Path(source).resolve(),Path(destination).resolve()
    if destination.exists():raise ValueError('restore requires a new, nonexistent data directory')
    manifest=json.loads((source/'manifest.json').read_text())
    if manifest.get('format')!=1 or 'library.db' not in manifest.get('files',{}):raise ValueError('invalid backup manifest')
    for relative,expected in manifest['files'].items():
        p=source/relative
        if Path(relative).is_absolute() or '..' in Path(relative).parts or p.is_symlink() or not p.resolve().is_relative_to(source) or not p.is_file() or checksum(p)!=expected:raise ValueError('invalid backup file: '+relative)
    with sqlite3.connect((source/'library.db').as_uri()+'?mode=ro',uri=True) as db:validate(db)
    destination.mkdir(mode=0o700,parents=True)
    try:
        for relative in manifest['files']:
            p=destination/relative;p.parent.mkdir(mode=0o700,parents=True,exist_ok=True);shutil.copy2(source/relative,p)
        with sqlite3.connect(destination/'library.db') as db:
            db.execute('DELETE FROM sessions');db.commit();validate(db)
    except BaseException:
        shutil.rmtree(destination);raise

def main():
    p=argparse.ArgumentParser(description=__doc__);sub=p.add_subparsers(dest='command',required=True)
    a=sub.add_parser('backup');a.add_argument('--data',required=True);a.add_argument('--output',required=True)
    a=sub.add_parser('restore');a.add_argument('--source',required=True);a.add_argument('--data',required=True)
    a=p.parse_args()
    try:
        if a.command=='backup':backup(a.data,a.output)
        else:restore(a.source,a.data)
    except (ValueError,OSError,sqlite3.Error) as e:p.exit(1,str(e)+'\n')
if __name__=='__main__':main()
