import hashlib, json, os, pathlib, secrets, shutil, signal, socket, sqlite3, subprocess, tempfile, time, urllib.parse, urllib.request
home=pathlib.Path.home()
root=pathlib.Path(tempfile.mkdtemp(prefix='ovl-pair-100k-',dir=home))
print(root,flush=True)
count=100_000
media=root/'media';media.mkdir()
clip=root/'fixture.mp4'
subprocess.run(['ffmpeg','-v','error','-f','lavfi','-i','color=size=64x64:rate=4:duration=0.5','-c:v','libx264','-pix_fmt','yuv420p',str(clip)],check=True,timeout=20)
clip.chmod(0o444)
source_clips=root/'source-clips'
source_clips.mkdir()
for group in range(100):
 folder=media/f'{group:03d}'
 folder.mkdir()
 seed=source_clips/f'{group:03d}.mp4'
 shutil.copyfile(clip,seed)
 seed.chmod(0o444)
 for i in range(group*1000+1,(group+1)*1000+1):
  name=f'Fixture {i:06d}'
  os.link(seed,folder/(name+'.mp4'))
  nfo=folder/(name+'.nfo')
  nfo.write_text(f'<movie><title>{name}</title><sorttitle>{name}</sorttitle></movie>')
  nfo.chmod(0o444)
 folder.chmod(0o555)
media.chmod(0o555)
print('fixture generated',count,flush=True)
password=secrets.token_urlsafe(24)
jelly=home/'ovl-jellyfin-12.4-hc7ns1a3/jellyfin'
rust=home/'ovl-rust-music-artists-final/source/target/release/ovl-server'
base_headers={'X-OVL-Request':'1','Authorization':'MediaBrowser Client="OVL benchmark", Device="N100", DeviceId="benchuser", Version="1.0"'}
def endpoint(port,path,method='GET',data=None,headers=None):
 req=urllib.request.Request(f'http://127.0.0.1:{port}'+path,method=method,headers={'Content-Type':'application/json',**(headers or base_headers)},data=None if data is None else json.dumps(data).encode())
 with urllib.request.urlopen(req,timeout=20) as res:
  raw=res.read(500001)
  if len(raw)>500000: raise ValueError('oversized response')
  return json.loads(raw) if raw else None
def free_port():
 with socket.socket() as sock:
  sock.bind(('127.0.0.1',0));return sock.getsockname()[1]
def digest(path):
 with path.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def paths(arm,query,offset,user,library):
 if arm=='rust':
  return '/api/v1/items?'+urllib.parse.urlencode(dict(q=query,offset=offset,limit=50,library=library))
 return '/Items?'+urllib.parse.urlencode(dict(UserId=user,ParentId=library,Recursive='true',IncludeItemTypes='Movie',SortBy='SortName',SortOrder='Ascending',SearchTerm=query,StartIndex=offset,Limit=50,EnableTotalRecordCount='false'))
def titles(arm,payload):
 return [v['title'] if arm=='rust' else v['Name'] for v in (payload if arm=='rust' else payload['Items'])]
def cancel(signum,frame):
 raise SystemExit(128+signum)
signal.signal(signal.SIGTERM,cancel)
def run(arm):
 port=free_port();trial=root/arm;trial.mkdir()
 if arm=='jellyfin':
  conf=trial/'config';conf.mkdir()
  (conf/'network.xml').write_text(f'<NetworkConfiguration><InternalHttpPort>{port}</InternalHttpPort><EnableIPv6>false</EnableIPv6><EnableRemoteAccess>false</EnableRemoteAccess><AutoDiscovery>false</AutoDiscovery><EnableUPnP>false</EnableUPnP><LocalNetworkAddresses><string>127.0.0.1</string></LocalNetworkAddresses></NetworkConfiguration>')
  command=[str(jelly/'jellyfin'),'--datadir',str(trial/'data'),'--configdir',str(conf),'--cachedir',str(trial/'cache'),'--logdir',str(trial/'logs'),'--webdir',str(jelly/'jellyfin-web'),'--ffmpeg','/usr/bin/ffmpeg','--nonetchange']
  env=os.environ.copy()
 else:
  command=[str(rust)]
  env={**os.environ,'OVL_LISTEN':f'127.0.0.1:{port}','OVL_DATA':str(trial/'data'),'OVL_ACCELERATOR':'cpu'}
 log=trial/'process.log'
 result=None
 with log.open('wb') as out:
  proc=subprocess.Popen(command,env=env,cwd=trial,stdout=out,stderr=subprocess.STDOUT,start_new_session=True)
  try:
   for _ in range(160):
    if proc.poll() is not None: raise RuntimeError(f'{arm} exited: {proc.returncode}')
    try:
     endpoint(port,'/System/Info/Public' if arm=='jellyfin' else '/api/v1/status')
     if arm=='rust' or 'Core startup complete' in log.read_text(): break
    except OSError: pass
    time.sleep(.25)
   else: raise RuntimeError(f'{arm} unavailable')
   if arm=='jellyfin':
    endpoint(port,'/Startup/User')
    endpoint(port,'/Startup/User','POST',{'Name':'benchuser','Password':password})
    endpoint(port,'/Startup/Complete','POST')
    auth=endpoint(port,'/Users/AuthenticateByName','POST',{'Username':'benchuser','Pw':password})
    headers={'Authorization':base_headers['Authorization']+', Token="'+auth['AccessToken']+'"'};user=auth['User']['Id']
    query=urllib.parse.urlencode(dict(name='Fixture',collectionType='movies',paths=str(media),refreshLibrary='true'))
    options=dict(EnableRealtimeMonitor=False,EnableInternetProviders=False,SaveLocalMetadata=False,MetadataSavers=[],DisabledLocalMetadataReaders=[],TypeOptions=[dict(Type='Movie',MetadataFetchers=[],ImageFetchers=[])])
    endpoint(port,'/Library/VirtualFolders?'+query,'POST',dict(LibraryOptions=options),headers)
    library=next(x['ItemId'] for x in endpoint(port,'/Library/VirtualFolders',headers=headers) if x['Name']=='Fixture')
   else:
    endpoint(port,'/api/v1/setup','POST',{'Name':'benchuser','Password':password})
    auth=endpoint(port,'/api/v1/login','POST',{'Name':'benchuser','Password':password})
    headers={'Authorization':'Bearer '+auth['token'],'X-OVL-Request':'1'};user=''
    library=endpoint(port,'/api/v1/libraries','POST',{'Name':'Fixture','Path':str(media),'Kind':'movie'},headers)['id']
    endpoint(port,'/api/v1/scan','POST',{},headers)
   print(arm,'scan requested',flush=True)
   start=time.monotonic()
   deadline=start+(10_800 if arm=='jellyfin' else 1800)
   while time.monotonic()<deadline:
    if shutil.disk_usage(root).free < 20 * (1 << 30):
     raise RuntimeError('fixture filesystem free space below 20 GiB')
    if proc.poll() is not None:
     raise RuntimeError(f'{arm} exited {proc.returncode}')
    if arm=='jellyfin':
     tasks=endpoint(port,'/ScheduledTasks',headers=headers)
     task=next(x for x in tasks if x['Key']=='RefreshLibrary')
     db=trial/'data/data/jellyfin.db'
     with sqlite3.connect(f'file:{db}?mode=ro',uri=True,timeout=10) as conn:
      rows,keys=conn.execute("select count(*),count(PresentationUniqueKey) from BaseItems where Type like '%.Movie' and Name like 'Fixture %'").fetchone()
     print(json.dumps(dict(arm=arm,elapsed=round(time.monotonic()-start),rows=rows,keys=keys,state=task['State'],progress=task.get('CurrentProgressPercentage'))),flush=True)
     if keys==count and rows==count and task['State']=='Idle':
      with sqlite3.connect(f'file:{db}?mode=ro',uri=True,timeout=10) as conn:
       unique,=conn.execute("select count(distinct PresentationUniqueKey) from BaseItems where Type like '%.Movie' and Name like 'Fixture %'").fetchone()
      assert unique==count, ('duplicate presentation keys',unique)
      break
     time.sleep(60)
    else:
     if not endpoint(port,'/api/v1/scan',headers=headers)['running']:
      break
     time.sleep(2)
   else:
    raise RuntimeError(f'{arm} scan deadline exceeded')
   checks=[]
   for offset,expected in ((0,range(1,51)),(count-50,range(count-49,count+1)),(count,())):
    names=titles(arm,endpoint(port,paths(arm,'',offset,user,library),headers=headers))
    assert names==[f'Fixture {n:06d}' for n in expected],(arm,offset,len(names))
    checks.append(dict(offset=offset,count=len(names),first=names[:1],last=names[-1:]))
   result=dict(seconds=time.monotonic()-start,browse=checks)
   print(arm,json.dumps(result),flush=True)
   return result
  finally:
   if proc.poll() is None:
    os.killpg(proc.pid,signal.SIGTERM)
    try:proc.wait(timeout=25)
    except subprocess.TimeoutExpired:os.killpg(proc.pid,signal.SIGKILL);proc.wait();raise
   print(arm,'exit',proc.returncode,flush=True)
   if result is not None:
    assert proc.returncode==0, ('unclean shutdown',arm,proc.returncode)
    result['exit_code']=proc.returncode
report=dict(scope='100k native scan preparation; no timings accepted',items=count,fixture_sha256=digest(clip),binary_sha256={'jellyfin':digest(jelly/'jellyfin'),'rust':digest(rust)},arms={})
for arm in ('rust','jellyfin'):
 report['arms'][arm]=run(arm)
 (root/'ready.json').write_text(json.dumps(report,indent=2)+'\n')
