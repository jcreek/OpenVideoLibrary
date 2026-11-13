#!/usr/bin/env python3
"""Compare compiled candidate prototypes. Results do not select a production backend."""
import argparse,concurrent.futures,datetime,hashlib,http.client,json,os,platform,random,shutil,sqlite3,statistics,subprocess,tempfile,threading,time,urllib.parse
from pathlib import Path

def seed(path,count):
 with sqlite3.connect(path) as c:
  c.executescript('PRAGMA journal_mode=WAL; CREATE TABLE items(id INTEGER PRIMARY KEY,title TEXT NOT NULL,path TEXT UNIQUE NOT NULL,mtime INTEGER NOT NULL DEFAULT 0,size INTEGER NOT NULL DEFAULT 0);CREATE INDEX items_title ON items(title COLLATE NOCASE,id);')
  c.executemany('INSERT INTO items(id,title,path) VALUES(?,?,?)',((i,f'Movie {i:08d}',f'/synthetic/{i}.mp4') for i in range(1,count+1)))
 c.close()

def sample(pid):
 try:
  row=subprocess.check_output(['ps','-o','rss=,time=','-p',str(pid)],text=True).split()
  parts=row[1].split(':');seconds=sum(float(x)*60**i for i,x in enumerate(reversed(parts)))
  return int(row[0])*1024,seconds
 except (subprocess.SubprocessError,ValueError,IndexError):return 0,0

def load(requests,concurrency,query,stop,background):
 local=threading.local()
 def one(i):
  if not hasattr(local,'conn'):local.conn=http.client.HTTPConnection('127.0.0.1',8097,timeout=30)
  start=time.perf_counter()
  try:
   local.conn.request('GET','/items?'+urllib.parse.urlencode({'offset':(i%10)*100,'q':query}))
   response=local.conn.getresponse();body=response.read();data=json.loads(body)
   if response.status!=200 or len(data)!=100 or any(not isinstance(v.get('id'),int) or not isinstance(v.get('title'),str) for v in data):raise ValueError('invalid response')
   return (time.perf_counter()-start)*1000,False
  except Exception:
   local.conn.close();return (time.perf_counter()-start)*1000,True
 start=time.perf_counter()
 with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:values=list(pool.map(one,range(requests)))
 elapsed=time.perf_counter()-start;lat=sorted(v for v,error in values if not error)
 def percentile(p):return lat[min(len(lat)-1,int((len(lat)-1)*p))] if lat else None
 return {'seconds':elapsed,'requests':requests,'errors':sum(e for _,e in values),'successful_rps':len(lat)/elapsed,'p50_ms':percentile(.5),'p95_ms':percentile(.95),'p99_ms':percentile(.99)}

def run(binary,db,args,concurrency,query):
 env={**os.environ,'BENCH_DB':str(db)}
 if args.media:env['BENCH_MEDIA']=str(Path(args.media).resolve())
 if args.scan_root:env['BENCH_ROOT']=str(Path(args.scan_root).resolve())
 log=tempfile.TemporaryFile();p=subprocess.Popen([str(binary)],env=env,stdout=log,stderr=log)
 encoders=[]
 for _ in range(args.ffmpeg_count):
  encode=['-c:v','copy'] if args.ffmpeg_mode=='remux' else ['-c:v',args.ffmpeg_encoder,'-b:v','3M']
  if args.ffmpeg_encoder=='libx264' and args.ffmpeg_mode=='transcode':encode+=['-preset','veryfast','-threads','2','-pix_fmt','yuv420p']
  command=['ffmpeg','-nostdin','-v','error','-stream_loop','-1','-re','-i',str(Path(args.media).resolve()),*encode,'-c:a','aac','-f','mpegts','pipe:1']
  encoders.append(subprocess.Popen(command,stdout=subprocess.DEVNULL,stderr=log))
 stop=threading.Event();threads=[];background={'scan_started':0,'stream_requests':0,'errors':0}
 def worker(kind):
  conn=http.client.HTTPConnection('127.0.0.1',8097,timeout=30)
  while not stop.is_set():
   try:
    if kind=='scan':conn.request('POST','/scan')
    else:conn.request('GET','/media',headers={'Range':'bytes=0-1048575'})
    response=conn.getresponse();response.read()
    if kind=='scan' and response.status==202:background['scan_started']+=1
    elif kind=='stream' and response.status in (200,206):background['stream_requests']+=1
    elif response.status!=409:background['errors']+=1
   except Exception:background['errors']+=1;conn.close()
   stop.wait(.02 if kind=='stream' else .1)
  conn.close()
 try:
  deadline=time.monotonic()+20
  while True:
   if p.poll() is not None:log.seek(0);raise RuntimeError('candidate exited: '+log.read().decode(errors='replace'))
   try:
    conn=http.client.HTTPConnection('127.0.0.1',8097,timeout=.5);conn.request('GET','/items');r=conn.getresponse();canonical=json.dumps(json.loads(r.read()),sort_keys=True,separators=(',',':'));conn.close();break
   except OSError:
    if time.monotonic()>deadline:raise RuntimeError('candidate startup timeout')
    time.sleep(.05)
  conn=http.client.HTTPConnection("127.0.0.1",8097,timeout=5);conn.request("GET","/info");candidate_info=json.loads(conn.getresponse().read());conn.close()
  cold=load(1,1,query,stop,background)
  if args.scan_root:threads.append(threading.Thread(target=worker,args=('scan',)))
  if args.media:threads.append(threading.Thread(target=worker,args=('stream',)))
  for t in threads:t.start()
  if args.ffmpeg_count:time.sleep(1)
  baseline=sample(p.pid);peak=[baseline[0]];ffmpeg_start=[sample(v.pid) for v in encoders];ffmpeg_peak=[sum(r[0] for r in ffmpeg_start)]
  def monitor():
   while not stop.wait(.1):
    peak[0]=max(peak[0],sample(p.pid)[0]);ffmpeg_peak[0]=max(ffmpeg_peak[0],sum(sample(v.pid)[0] for v in encoders))
  monitor_thread=threading.Thread(target=monitor);monitor_thread.start()
  result=load(args.requests,concurrency,query,stop,background);end=sample(p.pid)
  stop.set();monitor_thread.join()
  result.update({'candidate_info':candidate_info,'ffmpeg_peak_rss_bytes':ffmpeg_peak[0],'ffmpeg_cpu_seconds':sum(max(0,sample(v.pid)[1]-old[1]) for v,old in zip(encoders,ffmpeg_start)),'ffmpeg_exited_early':sum(v.poll() is not None for v in encoders),'process_first_request_ms':cold['p95_ms'],'response_sha256':hashlib.sha256(canonical.encode()).hexdigest(),'core_peak_rss_bytes':max(peak[0],end[0]),'core_cpu_seconds':max(0,end[1]-baseline[1]),'background':background})
  return result
 finally:
  stop.set();p.terminate()
  for encoder in encoders:
   encoder.terminate()
   try:encoder.wait(timeout=5)
   except subprocess.TimeoutExpired:encoder.kill();encoder.wait()
  try:p.wait(timeout=5)
  except subprocess.TimeoutExpired:p.kill();p.wait()
  for t in threads:t.join(timeout=31)
  log.close()

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--rust',default='bench/rust/target/release/ovl-bench-rust');p.add_argument('--items',nargs='+',type=int,default=[10000,100000]);p.add_argument('--concurrency',nargs='+',type=int,default=[1,10,50]);p.add_argument('--requests',type=int,default=2000);p.add_argument('--repetitions',type=int,default=5);p.add_argument('--media');p.add_argument('--scan-root');p.add_argument('--output',required=True);p.add_argument('--ffmpeg-count',type=int,default=0);p.add_argument('--ffmpeg-mode',choices=['remux','transcode'],default='transcode');p.add_argument('--ffmpeg-encoder',choices=['libx264','h264_qsv','h264_nvenc','h264_vaapi'],default='libx264');p.add_argument('--baseline',help='same-host suite result used for variance-derived regression checks');a=p.parse_args()
 if a.requests<1 or a.repetitions<3 or min(a.concurrency)<1 or min(a.items)<1000:p.error('require >=3 repetitions, >=1000 items and positive requests/concurrency')
 if a.ffmpeg_count<0 or (a.ffmpeg_count and not a.media):p.error('FFmpeg workloads require --media and a nonnegative count')
 binaries={'rust':Path(a.rust).resolve()}
 for binary in binaries.values():
  if not binary.is_file():p.error('missing compiled candidate: '+str(binary))
 # Refuse to benchmark against an unrelated process already occupying the port.
 import socket
 with socket.socket() as sock:
  if sock.connect_ex(('127.0.0.1',8097))==0:p.error('port 8097 is already in use')
 rng=random.Random(17);results=[]
 with tempfile.TemporaryDirectory(prefix='ovl-bench-') as tmp:
  for count in a.items:
   original=Path(tmp)/'fixture.db';seed(original,count)
   for concurrency in a.concurrency:
    for query in ['','Movie 0000']:
     for repetition in range(a.repetitions):
      candidates=list(binaries);rng.shuffle(candidates)
      for name in candidates:
       database=Path(tmp)/(name+'.db')
       for suffix in ['','-wal','-shm']:
        candidate=Path(str(database)+suffix)
        if candidate.exists():candidate.unlink()
       src=sqlite3.connect(original);dst=sqlite3.connect(database)
       try:src.backup(dst)
       finally:src.close();dst.close()
       result=run(binaries[name],database,a,concurrency,query);result.update({'candidate':name,'items':count,'concurrency':concurrency,'query':query,'repetition':repetition});results.append(result)
       print(f'{name} {count} c={concurrency} q={query!r} p95={result["p95_ms"]} errors={result["errors"]}',flush=True)
   original.unlink()
 groups={}
 for result in results:
  key=f'{result["candidate"]}/{result["items"]}/{result["concurrency"]}/{result["query"]}'
  groups.setdefault(key,[]).append(result)
 summary={}
 for key,rows in groups.items():
  valid=[r['p95_ms'] for r in rows if r['p95_ms'] is not None]
  summary[key]={'p95_median_ms':statistics.median(valid) if valid else None,'p95_stdev_ms':statistics.stdev(valid) if len(valid)>1 else None,'errors':sum(r['errors']+r['background']['errors']+r['ffmpeg_exited_early'] for r in rows),'successful_rps_median':statistics.median(r['successful_rps'] for r in rows),'repetitions':len(rows)}
 report={'kind':'prototype benchmark; not a production stack selection','utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'host':platform.platform(),'machine':platform.machine(),'sqlite_seed_version':sqlite3.sqlite_version,'binaries':{n:hashlib.sha256(b.read_bytes()).hexdigest() for n,b in binaries.items()},'workload':{'media':bool(a.media),'scan_root':bool(a.scan_root),'requests':a.requests,'ffmpeg_count':a.ffmpeg_count,'ffmpeg_mode':a.ffmpeg_mode,'ffmpeg_encoder':a.ffmpeg_encoder},'isolation':'uncontrolled host; reference-machine qualification must be recorded separately','cache':'process restarted; OS caches not evicted; first sample includes readiness warmup','results':results,'summary':summary}
 Path(a.output).parent.mkdir(parents=True,exist_ok=True);Path(a.output).write_text(json.dumps(report,indent=2)+'\n')
 failed=any(r['errors'] or r['background']['errors'] or r['ffmpeg_exited_early'] for r in results)
 if a.baseline:
  baseline=json.loads(Path(a.baseline).read_text())
  if any(baseline.get(k)!=report.get(k) for k in ['host','machine','workload']):p.error('baseline environment/workload differs')
  for key,row in summary.items():
   old=baseline['summary'].get(key)
   if not old or old['repetitions']<3 or old['p95_stdev_ms'] is None:p.error('baseline lacks sufficient repetitions: '+key)
   threshold=old['p95_median_ms']+3*old['p95_stdev_ms']
   if row['p95_median_ms'] is None or row['p95_median_ms']>threshold:print('REGRESSION '+key,flush=True);failed=True
 if failed:p.exit(1,'benchmark errors or regression detected\n')
if __name__=='__main__':main()
