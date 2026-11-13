#!/usr/bin/env python3
"""Isolated N100 application playback check. Reads NAS media; uses disposable app data."""
import argparse,hashlib,http.client,json,os,pathlib,secrets,socket,subprocess,tempfile,threading,time,urllib.error,urllib.request
from remote_worker import proc_sample

def main():
 p=argparse.ArgumentParser();p.add_argument('--output',required=True);p.add_argument('--repetitions',type=int,default=3);a=p.parse_args();source=pathlib.Path(__file__).resolve().parents[1];workspace=source.parent;sample=json.loads((workspace/'sample.json').read_text());port=18096
 with socket.socket() as sock:
  if sock.connect_ex(('127.0.0.1',port))==0:raise RuntimeError('playback test port occupied')
 report={'kind':'N100 application playback; loopback FFmpeg first decoded frame, not browser/LAN acceptance','media':sample['probe'],'results':[]}
 with tempfile.TemporaryDirectory(prefix='app-trial-',dir=workspace) as tmp:
  root=pathlib.Path(tmp);env={**os.environ,'OVL_LISTEN':f'127.0.0.1:{port}','OVL_DATA':str(root/'data'),'OVL_ACCELERATOR':'auto','OVL_MAX_STREAMS':'8','OVL_TRANSCODE_BYTES':str(128<<20),'OVL_MIN_FREE_BYTES':str(128<<20),'OVL_SECURE_COOKIES':'false'}
  token='';log=(root/'server.log').open('w+');server=subprocess.Popen([str(source/'bin/ovl')],env=env,stdout=log,stderr=log)
  def api(path,method='GET',data=None):
   headers={'Content-Type':'application/json','X-OVL-Request':'1'}
   if token:headers['Authorization']='Bearer '+token
   request=urllib.request.Request(f'http://127.0.0.1:{port}/api/v1/'+path,method=method,data=None if data is None else json.dumps(data).encode(),headers=headers)
   with urllib.request.urlopen(request,timeout=30) as response:return json.load(response)
  try:
   deadline=time.monotonic()+20
   while True:
    try:api('status');break
    except OSError:
     if server.poll() is not None or time.monotonic()>deadline:raise RuntimeError('app did not start')
     time.sleep(.05)
   password=secrets.token_urlsafe(24);api('setup','POST',{'Name':'benchmark','Password':password});token=api('login','POST',{'Name':'benchmark','Password':password})['token'];report['capabilities']=api('capabilities')
   api('libraries','POST',{'Name':'Read-only NAS fixture','Path':sample['scan_root'],'Kind':'tv'});api('scan','POST')
   deadline=time.monotonic()+120
   while api('scan')['running']:
    if time.monotonic()>deadline:raise RuntimeError('scan timed out')
    time.sleep(.1)
   import sqlite3
   with sqlite3.connect(root/'data/library.db') as db:item=db.execute('SELECT item_id FROM media_files WHERE path=?',(sample['path'],)).fetchone()[0]
   for mode,adaptive in [('direct',False),('transcode',False),('transcode',True)]:
    for repetition in range(a.repetitions):
     before=proc_sample(server.pid);start=time.perf_counter();session=api(f'items/{item}/playback','POST',{'Mode':mode,'Adaptive':adaptive,'Audio':0,'Start':60});created=time.perf_counter();client=None
     try:
      url=f'http://127.0.0.1:{port}'+session['url']
      try:client=subprocess.run(['ffmpeg','-nostdin','-v','error','-threads','1','-headers','Authorization: Bearer '+token+'\r\n','-ss','0' if mode!='direct' else '60','-i',url,'-frames:v','1','-f','null','-'],capture_output=True,text=True,timeout=40)
      except subprocess.TimeoutExpired:raise RuntimeError('playback client exceeded 40 seconds') from None
      elapsed=time.perf_counter()-start
      children=[]
      for path in pathlib.Path('/proc').iterdir():
       if not path.name.isdigit():continue
       try:
        raw=(path/'stat').read_text();fields=raw[raw.rfind(')')+2:].split()
        if int(fields[1])!=server.pid:continue
        argv=(path/'cmdline').read_bytes().split(b'\0');args=[x.decode(errors='replace') for x in argv if x]
        encoders=[args[i+1] for i,x in enumerate(args[:-1]) if x=='-c:v'];children.append({'encoders':encoders,'rss_bytes':proc_sample(int(path.name))[0]})
       except (OSError,ValueError,IndexError):pass
      report['results'].append({'mode':mode,'adaptive':adaptive,'repetition':repetition,'api_setup_ms':(created-start)*1000,'first_decoded_frame_ms':elapsed*1000,'success':client.returncode==0,'core_rss_bytes':proc_sample(server.pid)[0],'core_cpu_seconds':max(0,proc_sample(server.pid)[1]-before[1]),'ffmpeg_workers':children,'error':client.stderr[-1500:].replace(token,'[redacted]').replace(sample['path'],'[media]') if client.returncode else ''})
      print(mode,'adaptive='+str(adaptive),'first_frame_ms='+str(round(elapsed*1000,1)),'success='+str(client.returncode==0),flush=True)
     finally:
      if session.get('id'):api('playback/'+session['id'],'DELETE')
   pathlib.Path(a.output).write_text(json.dumps(report,indent=2)+'\n')
   if not all(row['success'] for row in report['results']):raise RuntimeError('one or more playback checks failed')
  finally:
   server.terminate()
   try:server.wait(timeout=15)
   except subprocess.TimeoutExpired:server.kill();server.wait()
   log.close()
if __name__=='__main__':main()
