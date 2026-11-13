#!/usr/bin/env python3
"""Reproducible browse microbenchmark. Not a playback or stack-selection result."""
import argparse, concurrent.futures, json, platform, sqlite3, time, urllib.request
p=argparse.ArgumentParser(); sub=p.add_subparsers(dest='command',required=True)
s=sub.add_parser('seed');s.add_argument('database');s.add_argument('--items',type=int,choices=[10000,100000],default=10000)
s=sub.add_parser('run');s.add_argument('--url',default='http://localhost:8097');s.add_argument('--requests',type=int,default=1000);s.add_argument('--concurrency',type=int,default=10);s.add_argument('--label',required=True)
a=p.parse_args()
if a.command=='seed':
 c=sqlite3.connect(a.database);c.execute('PRAGMA journal_mode=WAL');c.execute('CREATE TABLE IF NOT EXISTS items(id INTEGER PRIMARY KEY,title TEXT NOT NULL,path TEXT UNIQUE NOT NULL,mtime INTEGER NOT NULL DEFAULT 0,size INTEGER NOT NULL DEFAULT 0)');c.execute('CREATE INDEX IF NOT EXISTS items_title ON items(title COLLATE NOCASE,id)');c.execute('DELETE FROM items');c.executemany('INSERT INTO items(id,title,path) VALUES(?,?,?)',((i,f'Movie {i:08d}',f'/synthetic/{i}.mp4') for i in range(1,a.items+1)));c.commit();c.close()
else:
 def request(i):
  t=time.perf_counter()
  try:
   with urllib.request.urlopen(a.url+'/items?offset='+str((i%10)*100),timeout=30) as r:
    data=json.load(r)
    if len(data)!=100:raise ValueError('invalid response')
   return (time.perf_counter()-t)*1000,False
  except Exception:return (time.perf_counter()-t)*1000,True
 start=time.perf_counter()
 with concurrent.futures.ThreadPoolExecutor(max_workers=a.concurrency) as pool:results=list(pool.map(request,range(a.requests)))
 elapsed=time.perf_counter()-start;lat=sorted(x[0] for x in results if not x[1])
 def percentile(p):return lat[min(len(lat)-1,int((len(lat)-1)*p))] if lat else None
 print(json.dumps({'label':a.label,'host':platform.platform(),'requests':a.requests,'concurrency':a.concurrency,'errors':sum(x[1] for x in results),'seconds':elapsed,'successful_rps':len(lat)/elapsed,'p50_ms':percentile(.5),'p95_ms':percentile(.95),'p99_ms':percentile(.99)},indent=2))

 if any(x[1] for x in results):raise SystemExit(1)
