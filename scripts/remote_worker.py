#!/usr/bin/env python3
"""Private SSH worker: own one candidate and its FFmpeg process; never expose a port."""
import argparse,hashlib,http.client,json,os,pathlib,select,socket,subprocess,sys,tempfile,threading,time
from bench_suite import seed

def emit(value):print(json.dumps(value),flush=True)
def proc_sample(pid):
 try:
  raw=pathlib.Path(f'/proc/{pid}/stat').read_text();parts=raw[raw.rfind(')')+2:].split();cpu=(int(parts[11])+int(parts[12]))/os.sysconf('SC_CLK_TCK');rss=int(parts[21])*os.sysconf('SC_PAGE_SIZE');return rss,cpu
 except (OSError,IndexError,ValueError):return 0,0

def main():
 a=argparse.ArgumentParser();a.add_argument('--candidate',choices=['rust'],required=True);a.add_argument('--count',type=int,required=True);a.add_argument('--mode',choices=['idle','mixed-cpu','mixed-vaapi'],required=True);args=a.parse_args()
 if args.count not in [10000,100000]:a.error('unsupported fixture size')
 root=pathlib.Path(__file__).resolve().parents[1];workspace=root.parent
 binary=root/'bench/rust/target/release/ovl-bench-rust'
 with socket.socket() as sock:
  if sock.connect_ex(('127.0.0.1',8097))==0:raise RuntimeError('port 8097 is occupied')
 with tempfile.TemporaryDirectory(prefix='trial-',dir=workspace) as temporary:
  directory=pathlib.Path(temporary);database=directory/'fixture.db';seed(database,args.count);env={**os.environ,'BENCH_DB':str(database)}
  media=None
  if args.mode!='idle':
   media=json.loads((workspace/'sample.json').read_text());env.update({'BENCH_MEDIA':media['path'],'BENCH_ROOT':media['scan_root']})
  stop=threading.Event();children=[];monitor=None
  with (directory/'process.log').open('w+') as log:
   try:
    server=subprocess.Popen([str(binary)],env=env,stdout=log,stderr=log);children.append(server)
    if media:
     if args.mode=='mixed-vaapi':pre=['-vaapi_device','/dev/dri/renderD128','-hwaccel','vaapi','-hwaccel_output_format','vaapi'];enc=['-vf','scale_vaapi=w=1280:h=720:format=nv12','-c:v','h264_vaapi','-b:v','3M']
     else:pre=[];enc=['-c:v','libx264','-preset','veryfast','-threads','2','-pix_fmt','yuv420p','-b:v','3M']
     cmd=['ffmpeg','-nostdin','-v','error',*pre,'-stream_loop','-1','-re','-i',media['path'],*enc,'-c:a','aac','-f','mpegts','pipe:1'];children.append(subprocess.Popen(cmd,stdout=subprocess.DEVNULL,stderr=log));time.sleep(1)
    deadline=time.monotonic()+20
    while True:
     if any(p.poll() is not None for p in children):log.flush();log.seek(0);raise RuntimeError('candidate/FFmpeg exited: '+log.read()[-2000:])
     try:
      conn=http.client.HTTPConnection('127.0.0.1',8097,timeout=1);conn.request('GET','/info');info=json.loads(conn.getresponse().read());conn.close();break
     except OSError:
      if time.monotonic()>deadline:raise RuntimeError('candidate startup timeout')
      time.sleep(.05)
    peak=[0]*len(children);baseline=[0]*len(children)
    def watch():
     while not stop.wait(.1):
      for i,child in enumerate(children):peak[i]=max(peak[i],proc_sample(child.pid)[0])
    emit({'ready':True,'candidate_info':info,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'sample':None if not media else media['probe']})
    for line in sys.stdin:
     command=json.loads(line).get('command')
     if command=='start':
      for i,child in enumerate(children):peak[i],baseline[i]=proc_sample(child.pid)
      monitor=threading.Thread(target=watch);monitor.start();emit({'started':True})
     elif command=='finish':
      stop.set()
      if monitor:monitor.join()
      stats=[proc_sample(c.pid) for c in children]
      emit({'core_peak_rss_bytes':max(peak[0],stats[0][0]),'core_cpu_seconds':max(0,stats[0][1]-baseline[0]),'ffmpeg_peak_rss_bytes':sum(max(peak[i],stats[i][0]) for i in range(1,len(children))),'ffmpeg_cpu_seconds':sum(max(0,stats[i][1]-baseline[i]) for i in range(1,len(children))),'child_errors':sum(c.poll() is not None for c in children)});break
   finally:
    stop.set()
    if monitor:monitor.join()
    for child in reversed(children):
     if child.poll() is None:child.terminate()
     try:child.wait(timeout=5)
     except subprocess.TimeoutExpired:child.kill();child.wait()
if __name__=='__main__':
 try:main()
 except Exception as e:emit({'error':str(e)});raise SystemExit(1)
