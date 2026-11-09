#!/usr/bin/env python3
"""Off-host HTTP load through a trusted SSH tunnel; candidates and metrics run on N100."""
import argparse,concurrent.futures,datetime,http.client,json,os,pathlib,random,select,shlex,socket,statistics,subprocess,tempfile,threading,time,urllib.parse

def read_event(process,timeout=60):
 if not select.select([process.stdout],[],[],timeout)[0]:raise RuntimeError('SSH worker response timed out')
 line=process.stdout.readline()
 if not line:raise RuntimeError('SSH worker exited without response')
 result=json.loads(line)
 if result.get('error'):raise RuntimeError(result['error'])
 return result

def load(count,concurrency,query,port):
 local=threading.local()
 def request(i):
  if not hasattr(local,'conn'):local.conn=http.client.HTTPConnection('127.0.0.1',port,timeout=20)
  start=time.perf_counter()
  try:
   local.conn.request('GET','/items?'+urllib.parse.urlencode({'q':query,'offset':(i%10)*100}));response=local.conn.getresponse();data=json.loads(response.read())
   if response.status!=200 or len(data)!=100 or any(not isinstance(v.get('id'),int) or not isinstance(v.get('title'),str) for v in data):raise ValueError('invalid response')
   return (time.perf_counter()-start)*1000,False
  except Exception:local.conn.close();return (time.perf_counter()-start)*1000,True
 start=time.perf_counter()
 with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:rows=list(pool.map(request,range(count)))
 elapsed=time.perf_counter()-start;values=sorted(v for v,e in rows if not e)
 def percentile(p):return values[min(len(values)-1,int((len(values)-1)*p))] if values else None
 return {'requests':count,'seconds':elapsed,'errors':sum(e for _,e in rows),'successful_rps':len(values)/elapsed,'p50_ms':percentile(.5),'p95_ms':percentile(.95),'p99_ms':percentile(.99)}

def main():
 p=argparse.ArgumentParser();p.add_argument('--host',required=True);p.add_argument('--key',required=True);p.add_argument('--workspace',default='/home/ovlbench/ovl-workbench');p.add_argument('--port',type=int,default=18098);p.add_argument('--requests',type=int,default=2000);p.add_argument('--repetitions',type=int,default=5);p.add_argument('--concurrency',type=int,nargs='+',default=[1,10,25,50,100]);p.add_argument('--modes',nargs='+',choices=['idle','mixed-cpu','mixed-vaapi'],default=['idle','mixed-vaapi']);p.add_argument('--candidates',nargs='+',choices=['rust'],default=['rust']);p.add_argument('--items',type=int,nargs='+',default=[10000,100000]);p.add_argument('--output',required=True);a=p.parse_args()
 if a.requests<100 or a.repetitions<3 or min(a.concurrency)<1:p.error('require >=100 requests, >=3 repetitions and positive concurrency')
 if any(n not in [10000,100000] for n in a.items):p.error('supported fixture sizes are 10000 and 100000')
 with socket.socket() as sock:
  if sock.connect_ex(('127.0.0.1',a.port))==0:p.error('local tunnel port occupied')
 ssh=['ssh','-o','BatchMode=yes','-o','StrictHostKeyChecking=yes','-o','IdentitiesOnly=yes','-o','ConnectTimeout=5','-i',str(pathlib.Path(a.key).expanduser())]
 report={'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'kind':'N100 prototype matrix; off-host HTTP client; SSH transport','transport':'single persistent SSH tunnel, shared by candidates','isolation':'existing household services continue running; OS/NAS caches not evicted','configuration':{'requests':a.requests,'repetitions':a.repetitions,'concurrency':a.concurrency,'modes':a.modes,'candidates':a.candidates,'items':a.items},'results':[]};output=pathlib.Path(a.output);output.parent.mkdir(parents=True,exist_ok=True)
 tunnel=subprocess.Popen(ssh+['-o','ExitOnForwardFailure=yes','-N','-L',f'127.0.0.1:{a.port}:127.0.0.1:8097',a.host],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 def save():
  temp=output.with_suffix('.tmp');temp.write_text(json.dumps(report,indent=2)+'\n');temp.replace(output)
 try:
  time.sleep(.5)
  if tunnel.poll() is not None:raise RuntimeError('SSH forwarding failed')
  rng=random.Random(17)
  for mode in a.modes:
   for count in a.items:
    for concurrency in a.concurrency:
     for query in ['',('Movie 00009' if count==10000 else 'Movie 0009')]:
      for repetition in range(a.repetitions):
       candidates=list(a.candidates);rng.shuffle(candidates)
       for candidate in candidates:
        args=['python3',a.workspace+'/source/scripts/remote_worker.py','--candidate',candidate,'--count',str(count),'--mode',mode]
        with tempfile.TemporaryFile() as log:
         worker=subprocess.Popen(ssh+[a.host,shlex.join(args)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=log,text=True,bufsize=1);stop=threading.Event();background={'range_requests':0,'scan_started':0,'errors':0};threads=[]
         def background_task(kind):
          conn=http.client.HTTPConnection('127.0.0.1',a.port,timeout=20)
          while not stop.is_set():
           try:
            if kind=='scan':conn.request('POST','/scan')
            else:conn.request('GET','/media',headers={'Range':'bytes=0-1048575'})
            response=conn.getresponse();response.read()
            if kind=='scan' and response.status==202:background['scan_started']+=1
            elif kind=='range' and response.status==206:background['range_requests']+=1
            elif response.status!=409:background['errors']+=1
           except Exception:background['errors']+=1;conn.close()
           stop.wait(.5 if kind=='scan' else .1)
          conn.close()
         try:
          ready=read_event(worker)
          if report['results'] and ready['candidate_info']['sqlite_version']!=report['results'][0]['candidate_info']['sqlite_version']:raise RuntimeError('SQLite engine versions differ; comparison rejected')
          if mode!='idle':
           threads=[threading.Thread(target=background_task,args=(k,)) for k in ['scan','range']]
           for thread in threads:thread.start()
          # Warm transport/readiness separately from the measured request distribution.
          load(10,1,query,a.port)
          worker.stdin.write('{"command":"start"}\n');worker.stdin.flush();read_event(worker)
          result=load(a.requests,concurrency,query,a.port);stop.set()
          for thread in threads:thread.join(timeout=21)
          worker.stdin.write('{"command":"finish"}\n');worker.stdin.flush();metrics=read_event(worker)
          result.update(metrics);result.update({'candidate':candidate,'mode':mode,'items':count,'concurrency':concurrency,'query':query,'repetition':repetition,'candidate_info':ready['candidate_info'],'binary_sha256':ready['binary_sha256'],'background':background,'sample':ready['sample']});report['results'].append(result);save()
          print(f'{mode} {count} c={concurrency} {candidate} p95={result["p95_ms"]:.2f}ms errors={result["errors"]+metrics["child_errors"]+background["errors"]}',flush=True)
         finally:
          stop.set()
          if worker.stdin:worker.stdin.close()
          try:worker.wait(timeout=30)
          except subprocess.TimeoutExpired:worker.terminate();worker.wait(timeout=5)
  groups={}
  for row in report['results']:groups.setdefault(f'{row["mode"]}/{row["candidate"]}/{row["items"]}/{row["concurrency"]}/{row["query"]}',[]).append(row)
  report['summary']={key:{'p95_median_ms':statistics.median(r['p95_ms'] for r in rows),'p95_stdev_ms':statistics.stdev(r['p95_ms'] for r in rows),'successful_rps_median':statistics.median(r['successful_rps'] for r in rows),'core_peak_rss_median_bytes':statistics.median(r['core_peak_rss_bytes'] for r in rows),'core_cpu_median_seconds':statistics.median(r['core_cpu_seconds'] for r in rows),'errors':sum(r['errors']+r['child_errors']+r['background']['errors'] for r in rows)} for key,rows in groups.items()};report['complete']=True;save()
  if any(v['errors'] for v in report['summary'].values()):raise RuntimeError('matrix recorded request/background errors')
 finally:
  tunnel.terminate()
  try:tunnel.wait(timeout=5)
  except subprocess.TimeoutExpired:tunnel.kill();tunnel.wait()
if __name__=='__main__':main()
