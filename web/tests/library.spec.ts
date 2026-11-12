import {test,expect} from '@playwright/test';
import {spawn,execFileSync,type ChildProcess} from 'node:child_process';
import {mkdtempSync,mkdirSync,rmSync,writeFileSync,copyFileSync,renameSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
let server:ChildProcess,root:string,media:string,music:string,serverLog='';
test.afterEach(async({},testInfo)=>{if(testInfo.status!==testInfo.expectedStatus)await testInfo.attach('server-log',{body:serverLog,contentType:'text/plain'});});
test.beforeEach(async({browser},testInfo)=>{testInfo.annotations.push({type:'browser-version',description:browser.version()});});
const username='browser-admin',password='browser-test-password-123';
test.beforeAll(async()=>{
 root=mkdtempSync(join(process.env.OVL_TEST_TMP||tmpdir(),'ovl-browser-'));media=join(root,'media');mkdirSync(media);
 writeFileSync(join(media,'Example.S01E01.srt'),'1\n00:00:00,000 --> 00:00:04,000\nA local subtitle\n');
 execFileSync('ffmpeg',['-nostdin','-v','error','-f','lavfi','-i','testsrc2=size=640x360:rate=24','-f','lavfi','-i','sine=frequency=440','-i',join(media,'Example.S01E01.srt'),'-t','12','-c:v','libx264','-preset','ultrafast','-g','24','-c:a','aac','-c:s','mov_text',join(media,'Example.S01E01.mp4')]);
 copyFileSync(join(media,'Example.S01E01.mp4'),join(media,'Example.S01E02.mp4'));

 music=join(root,'music');mkdirSync(music);
 execFileSync('ffmpeg',['-nostdin','-v','error','-f','lavfi','-i','sine=frequency=440','-t','4',
  '-c:a','libmp3lame','-metadata','artist=Fixture Artist','-metadata','album=Fixture Album',join(music,'First track.mp3')]);
 execFileSync('ffmpeg',['-nostdin','-v','error','-f','lavfi','-i','color=red:size=32x32','-frames:v','1','-threads','1',join(music,'cover.png')]);
 const embedded=join(music,'embedded');mkdirSync(embedded);
 execFileSync('ffmpeg',['-nostdin','-v','error','-i',join(music,'First track.mp3'),'-i',join(music,'cover.png'),
  '-map','0:a:0','-map','1:v:0','-c','copy','-disposition:v','attached_pic',join(embedded,'Second track.mp3')]);
 writeFileSync(join(music,'First track.lrc'),'[00:00.10]Opening line\n[00:01.50]Jump here\n[offset:+500]\n');

 server=spawn(resolve('../bin/ovl'),[],{env:{...process.env,OVL_DATA:join(root,'data'),OVL_WEB:resolve('dist'),OVL_MIN_FREE_BYTES:'1048576',OVL_TRANSCODE_BYTES:'16777216',OVL_LISTEN:'127.0.0.1:18096',OVL_ACCELERATOR:'cpu',OVL_SECURE_COOKIES:'false'},stdio:'pipe'});
 serverLog='';for(const stream of [server.stdout,server.stderr])stream?.on('data',chunk=>{serverLog=(serverLog+chunk.toString()).slice(-65536);});
 let ready=false;for(let i=0;i<100;i++){try{if((await fetch('http://127.0.0.1:18096/api/v1/status')).ok){ready=true;break;}}catch{}await new Promise(r=>setTimeout(r,50));}if(!ready)throw Error('test server did not start');
});
test.afterAll(async()=>{if(server){const exit=new Promise<void>(r=>server.once('exit',()=>r()));server.kill('SIGTERM');await exit;}rmSync(root,{recursive:true,force:true});});
test('household workflow and viewer authorization',async({page,browserName})=>{
 await page.goto('/');await page.getByLabel('Username',{exact:true}).fill(username);await page.getByLabel('Password',{exact:true}).fill(password);await page.getByRole('button',{name:'Create library',exact:true}).click();await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
 await page.getByText('Manage libraries',{exact:true}).click();await page.getByLabel('Name',{exact:true}).fill('Test TV');await page.getByLabel('Server folder').fill(media);await page.getByLabel('Media type').selectOption('tv');await page.getByRole('button',{name:'Add library',exact:true}).click();
 await page.getByRole('button',{name:'Scan libraries',exact:true}).click();await page.getByRole('button',{name:'Refresh library',exact:true}).click();await expect(page.getByRole('heading',{name:'Example S01E01',exact:true})).toBeVisible();
 await page.getByRole('button',{name:'TV series',exact:true}).click();await page.getByRole('button',{name:/TV SERIES Example/}).click();await expect(page.getByRole('heading',{name:'Example',exact:true})).toBeVisible();
 await page.getByRole('button',{name:/TV ▶ Example S01E01/}).click();await expect(page.getByLabel('Audio',{exact:true}).locator('option')).toHaveCount(1);
 // Engine codec support differs; run transcoding and validate media readiness where H.264 is supported.
 await page.getByLabel('Playback',{exact:true}).selectOption('transcode');await page.getByRole('button',{name:'Apply playback settings'}).click();
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
 await expect(page.getByLabel('Subtitles',{exact:true}).locator('option[value="burn:0"]')).toContainText('burn in');
 await page.getByLabel('Playback',{exact:true}).selectOption('direct');
 await page.getByLabel('Subtitles',{exact:true}).selectOption('burn:0');
 const burned=page.waitForRequest(r=>r.url().endsWith('/playback')&&r.method()==='POST'&&r.postDataJSON().Subtitle===0);
 await page.getByRole('button',{name:'Apply playback settings',exact:true}).click();
 const burnRequest=(await burned).postDataJSON();
 expect(burnRequest.Mode).toBe('transcode');
 expect(burnRequest.Capabilities.max_audio_channels).toBe(2);
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
 await page.getByText('Edit title and metadata',{exact:true}).click();await page.getByLabel('Title',{exact:true}).fill('Edited episode');await page.getByLabel('Overview',{exact:true}).fill('Local metadata survives a refresh.');await page.getByRole('button',{name:'Save metadata',exact:true}).click();await expect(page.getByRole('heading',{name:'Edited episode',exact:true}).first()).toBeVisible();
 await page.getByRole('button',{name:'Close player',exact:true}).click();
 await page.getByText('Manage accounts',{exact:true}).click();await page.getByLabel('Username',{exact:true}).fill('viewer');await page.getByLabel('Initial password').fill('viewer-test-password-123');await page.getByRole('button',{name:'Create viewer',exact:true}).click();await expect(page.getByRole('button',{name:'viewer',exact:true})).toBeVisible();
 const accounts=await page.request.get('/api/v1/users');const viewer=(await accounts.json()).find((u:{name:string})=>u.name==='viewer');expect(viewer.libraries).toEqual([]);
 await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();await page.getByLabel('Username',{exact:true}).fill('viewer');await page.getByLabel('Password',{exact:true}).fill('viewer-test-password-123');await page.getByRole('button',{name:'Sign in',exact:true}).click();await expect(page.getByRole('heading',{name:'Bring your collection.'})).toBeVisible();expect((await page.request.get('/api/v1/items/1/file')).status()).toBe(404);await expect(page.getByText('Manage accounts',{exact:true})).toHaveCount(0);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBeTruthy();
});

test('persistent queue ordering and automatic next episode',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 expect((await page.request.post('/api/v1/login',{headers,data:{Name:username,Password:password}})).status()).toBe(200);
 if(!(await (await page.request.get('/api/v1/libraries')).json()).length){
  expect((await page.request.post('/api/v1/libraries',{headers,data:{Name:'Queue TV',Path:media,Kind:'tv'}})).status()).toBe(201);await page.request.post('/api/v1/scan',{headers});
  await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 }
 const titles=(await (await page.request.get('/api/v1/items')).json()).sort((a:{id:number},b:{id:number})=>a.id-b.id).slice(0,2).map((i:{title:string})=>i.title);expect(titles).toHaveLength(2);
 await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
 await page.getByRole('button',{name:'Collections & playlists',exact:true}).click();await page.getByLabel('List name',{exact:true}).fill('Evening queue');await page.getByLabel('List type',{exact:true}).selectOption('queue');await page.getByRole('button',{name:'Create list',exact:true}).click();
 await expect(page.getByRole('heading',{name:'Evening queue',exact:true})).toBeVisible();
 const lists=await (await page.request.get('/api/v1/lists')).json();const queue=lists.find((l:{name:string})=>l.name==='Evening queue');expect(queue).toBeTruthy();
 for(const [index,title] of titles.entries()){
  await page.getByRole('button',{name:'All media',exact:true}).click();
  await page.getByRole('button',{name:new RegExp('TV ▶ '+title)}).click();
  await expect(page.locator('.player h2')).toHaveText(title);
  await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
  await page.getByLabel('Add to list',{exact:true}).selectOption(String(queue.id));
  const add=page.getByRole('button',{name:'Add title',exact:true});
  if(index===0){
   let release!:()=>void,started!:()=>void,gatedMethod='';
   const gate=new Promise<void>(resolve=>{release=resolve;});
   const waiting=new Promise<void>(resolve=>{started=resolve;});
   await page.route('**/api/v1/lists/'+queue.id,async route=>{
    gatedMethod=route.request().method();
    started();
    await gate;
    await route.continue();
   },{times:1});
   try{
    await add.click();
    await waiting;
    expect(gatedMethod).toBe('GET');
    await page.getByRole('button',{name:new RegExp('TV ▶ '+titles[1])}).click();
    await expect(page.locator('.player h2')).toHaveText(titles[1]);
   }finally{release();}
  }else await add.click();
  await expect(page.getByRole('status').filter({hasText:'Added to Evening queue.'})).toBeVisible();
  await expect.poll(async()=>{
   const saved=await (await page.request.get('/api/v1/lists/'+queue.id)).json();
   return saved.entries.map((entry:{item:{title:string}})=>entry.item.title);
  }).toEqual(titles.slice(0,index+1));
  if(index===1){
   // A rejected write must replace the previous success notice with the error.
   await page.route('**/api/v1/lists/'+queue.id,route=>route.request().method()==='PUT'?route.fulfill({status:409,contentType:'application/json',body:JSON.stringify({error:'list changed; reload before saving'})}):route.continue(),{times:2});
   await add.click();
   await expect(page.getByRole('alert')).toHaveText('list changed; reload before saving');
   await expect(page.getByRole('status').filter({hasText:'Added to Evening queue.'})).toHaveCount(0);
   await page.unrouteAll();
   const saved=await (await page.request.get('/api/v1/lists/'+queue.id)).json();
   expect(saved.entries.map((entry:{item:{title:string}})=>entry.item.title)).toEqual(titles);
  }
  await page.getByRole('button',{name:'Close player',exact:true}).click();
 }
 await page.getByRole('button',{name:'Collections & playlists',exact:true}).click();await page.getByRole('button',{name:'Evening queue · queue',exact:true}).click();await expect(page.locator('ol li')).toHaveCount(2);
 await page.getByRole('button',{name:'Move '+titles[1]+' up',exact:true}).click();await expect(page.locator('ol li span').first()).toHaveText(titles[1]);
 await page.getByRole('button',{name:'Move '+titles[1]+' down',exact:true}).click();await expect(page.locator('ol li span').first()).toHaveText(titles[0]);
 await page.getByRole('button',{name:'Play queue from here',exact:true}).first().click();await expect(page.locator('.player h2')).toHaveText(titles[0]);await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
 await page.locator('video').evaluate((v:HTMLVideoElement)=>{v.currentTime=v.duration-.2;v.play();});await expect(page.locator('.player h2')).toHaveText(titles[1]);
 const saved=await (await page.request.get('/api/v1/lists/'+queue.id)).json();expect(saved.cursor).toBe(1);expect(saved.entries).toHaveLength(2);
 await page.reload();await page.getByRole('button',{name:'Collections & playlists',exact:true}).click();await page.getByRole('button',{name:'Evening queue · queue',exact:true}).click();await expect(page.locator('ol li')).toHaveCount(2);await expect(page.getByRole('button',{name:'Resume queue',exact:true})).toBeVisible();
});


test('queue preserves offline entries and saved positions',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 expect((await page.request.post('/api/v1/login',{headers,data:{Name:username,Password:password}})).status()).toBe(200);
 if(!(await (await page.request.get('/api/v1/libraries')).json()).length){
  expect((await page.request.post('/api/v1/libraries',{headers,data:{Name:'Queue TV',Path:media,Kind:'tv'}})).status()).toBe(201);await page.request.post('/api/v1/scan',{headers});
  await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 }
 const titles=(await (await page.request.get('/api/v1/items')).json()).sort((a:{episode:number},b:{episode:number})=>a.episode-b.episode);
 const created=await page.request.post('/api/v1/lists',{headers,data:{Kind:'queue',Name:'Offline queue',ItemIDs:titles.map((i:{id:number})=>i.id)}});expect(created.status()).toBe(201);const {id}=await created.json();
 const original=join(media,'Example.S01E01.mp4'),offline=original+'.offline';renameSync(original,offline);
 try{
  await page.goto('/');await page.getByRole('button',{name:'Collections & playlists',exact:true}).click();await page.getByRole('button',{name:'Offline queue · queue',exact:true}).click();
  await expect(page.locator('ol li')).toHaveCount(1);
  await page.getByRole('button',{name:'Remove',exact:true}).click();await expect(page.getByRole('alert')).toContainText('Some saved titles are unavailable');
  await page.getByRole('button',{name:'Play queue from here',exact:true}).click();await expect(page.locator('.player h2')).toHaveText(titles[1].title);
  const saved=await (await page.request.get('/api/v1/lists/'+id)).json();expect(saved.cursor).toBe(1);expect(saved.entry_count).toBe(2);expect(saved.entries[0].position).toBe(1);
  await page.getByRole('button',{name:'Close player',exact:true}).click();
 }finally{renameSync(offline,original);}
 const recovered=await (await page.request.get('/api/v1/lists/'+id)).json();expect(recovered.entries).toHaveLength(2);expect(recovered.cursor).toBe(1);
});

test('player lifecycle ignores delayed opening and closing',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 expect((await page.request.post('/api/v1/login',{headers,data:{Name:username,Password:password}})).status()).toBe(200);
 if(!(await (await page.request.get('/api/v1/libraries')).json()).length){
  expect((await page.request.post('/api/v1/libraries',{headers,data:{Name:'Lifecycle TV',Path:media,Kind:'tv'}})).status()).toBe(201);
  await page.request.post('/api/v1/scan',{headers});
  await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 }
 const items=(await (await page.request.get('/api/v1/items')).json()).sort((a:{id:number},b:{id:number})=>a.id-b.id).slice(0,2);
 await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
 const first=page.locator('.card').filter({has:page.getByRole('heading',{name:items[0].title,exact:true})}),second=page.locator('.card').filter({has:page.getByRole('heading',{name:items[1].title,exact:true})});
 let releaseProbe!:()=>void,probeStarted!:()=>void;
 const probeGate=new Promise<void>(r=>releaseProbe=r),probeWaiting=new Promise<void>(r=>probeStarted=r);
 await page.route('**/api/v1/items/'+items[0].id+'/probe',async route=>{probeStarted();await probeGate;await route.continue();});
 try {
  await first.click();await probeWaiting;
  await page.getByRole('button',{name:'Close player',exact:true}).click();
  await expect(page.locator('.player')).toHaveCount(0);
  await second.click();await expect(page.locator('.player h2')).toHaveText(items[1].title);
 } finally {releaseProbe();}
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
 await page.unrouteAll({behavior:'wait'});
 await expect(page.locator('.player h2')).toHaveText(items[1].title);
 await first.click();
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
 let releaseSave!:()=>void,saveStarted!:()=>void;
 const saveGate=new Promise<void>(r=>releaseSave=r),saveWaiting=new Promise<void>(r=>saveStarted=r);
 await page.route('**/api/v1/items/'+items[0].id+'/progress',async route=>{if(route.request().method()==='PUT'){saveStarted();await saveGate;}await route.continue();});
 try {
  await page.getByRole('button',{name:'Close player',exact:true}).click();await saveWaiting;
  await expect(page.locator('.player')).toHaveCount(0);
  await second.click();await expect(page.locator('.player h2')).toHaveText(items[1].title);
 } finally {releaseSave();}
 await expect(page.locator('.player h2')).toHaveText(items[1].title);
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
 await expect(page.getByRole('alert')).toHaveCount(0);
 await page.unrouteAll({behavior:'wait'});
 let firstCreations=0;page.on('request',request=>{if(request.method()==='POST'&&request.url().endsWith('/items/'+items[0].id+'/playback'))firstCreations++;});
 // A delayed Apply save must not restart a newer title.
 let releaseApply!:()=>void,applyStarted!:()=>void;
 const applyGate=new Promise<void>(r=>releaseApply=r),applyWaiting=new Promise<void>(r=>applyStarted=r);
 await page.route('**/api/v1/items/'+items[1].id+'/progress',async route=>{applyStarted();await applyGate;await route.continue();});
 try{
  await page.getByRole('button',{name:'Apply playback settings',exact:true}).click();await applyWaiting;
  await first.click();await expect(page.locator('.player h2')).toHaveText(items[0].title);
 }finally{releaseApply();}
 await page.unrouteAll({behavior:'wait'});
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
 expect(firstCreations).toBe(1);
 // A cursor update begun before Close must not reopen its queue afterwards.
 const created=await page.request.post('/api/v1/lists',{headers,data:{Kind:'queue',Name:'Delayed lifecycle queue',ItemIDs:[items[1].id]}});
 expect(created.status()).toBe(201);const queueID=(await created.json()).id;
 await page.getByRole('button',{name:'Collections & playlists',exact:true}).click();
 await page.getByRole('button',{name:'Delayed lifecycle queue · queue',exact:true}).click();
 let releaseCursor!:()=>void,cursorStarted!:()=>void;
 const cursorGate=new Promise<void>(r=>releaseCursor=r),cursorWaiting=new Promise<void>(r=>cursorStarted=r);
 await page.route('**/api/v1/lists/'+queueID+'/cursor',async route=>{cursorStarted();await cursorGate;await route.continue();});
 try{
  await page.getByRole('button',{name:'Resume queue',exact:true}).click();await cursorWaiting;
  await page.getByRole('button',{name:'Close player',exact:true}).click();await expect(page.locator('.player')).toHaveCount(0);
  const refreshed=page.waitForResponse(response=>response.url().endsWith('/api/v1/lists')&&response.request().method()==='GET');
  const completed=page.waitForResponse(response=>response.url().endsWith('/lists/'+queueID+'/cursor')&&response.request().method()==='PUT');
  releaseCursor();await completed;await refreshed;
  await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));
  await expect(page.locator('.player')).toHaveCount(0);
 }finally{releaseCursor();}
 await page.unrouteAll({behavior:'wait'});
 await page.getByRole('button',{name:'All media',exact:true}).click();await second.click();
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
 let releasePlayback!:()=>void,playbackStarted!:()=>void,lateSession='';
 const playbackGate=new Promise<void>(r=>releasePlayback=r),playbackWaiting=new Promise<void>(r=>playbackStarted=r);
 await page.route('**/api/v1/items/'+items[0].id+'/playback',async route=>{
  const response=await route.fetch();lateSession=(await response.json()).id;playbackStarted();
  await playbackGate;await route.fulfill({response});
 });
 await page.getByRole('button',{name:'Close player',exact:true}).click();
 await first.click();await page.getByLabel('Playback',{exact:true}).selectOption('transcode');
 // The held initial request may be direct; release it, then hold an explicit HLS request.
 releasePlayback();await playbackWaiting;
 await page.unrouteAll({behavior:'wait'});
 await expect(page.getByRole('button',{name:'Apply playback settings',exact:true})).toBeEnabled();
 const held=new Promise<void>(r=>releasePlayback=r),waiting=new Promise<void>(r=>playbackStarted=r);
 await page.route('**/api/v1/items/'+items[0].id+'/playback',async route=>{
  const response=await route.fetch();lateSession=(await response.json()).id;playbackStarted();
  await held;await route.fulfill({response});
 });
 try{
  await page.getByRole('button',{name:'Apply playback settings',exact:true}).click();await waiting;
  expect(lateSession).toBeTruthy();
  await page.getByRole('button',{name:'Close player',exact:true}).click();await second.click();
 }finally{releasePlayback();}
 await expect.poll(async()=>(await page.request.get('/api/v1/playback/'+lateSession+'/index.m3u8')).status()).toBe(404);
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);

 await expect(page.locator('.player h2')).toHaveText(items[1].title);
});

test('music library tags and decoded audio queue playback',async({page,browserName})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)
  expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 expect((await page.request.post('/api/v1/login',{headers,data:{Name:username,Password:password}})).status()).toBe(200);
 await page.goto('/');
 await page.getByText('Manage libraries',{exact:true}).click();
 await page.getByLabel('Name',{exact:true}).fill('Music fixture');
 await page.getByLabel('Server folder').fill(music);
 await page.getByLabel('Media type').selectOption('music');
 await page.getByRole('button',{name:'Add library',exact:true}).click();
 await expect(page.getByLabel('Library',{exact:true}).locator('option').filter({hasText:'Music fixture'})).toHaveCount(1);
 expect((await page.request.post('/api/v1/scan',{headers})).status()).toBe(202);
 await expect.poll(async()=> (await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 const libraries=await (await page.request.get('/api/v1/libraries')).json();
 const library=libraries.find((l:{name:string})=>l.name==='Music fixture');
 const tracks=await (await page.request.get('/api/v1/items?library='+library.id)).json();
 expect(tracks.map((t:{title:string})=>t.title)).toEqual(['First track','Second track']);
 await page.getByRole('button',{name:'Music albums',exact:true}).click();
 await page.getByRole('button',{name:/ALBUM Fixture Album Fixture Artist · 2 tracks/}).click();
 await expect(page.locator('h1')).toHaveText('Fixture Album');
 await expect(page.getByRole('button',{name:/MUSIC ▶ First track/})).toBeVisible();
 await page.getByRole('button',{name:'Back to albums',exact:true}).click();
 await page.getByRole('button',{name:'Music artists',exact:true}).click();
 await page.getByRole('button',{name:/ARTIST Fixture Artist 1 albums · 2 tracks/}).click();
 await expect(page.locator('h1')).toHaveText('Fixture Artist');
 await expect(page.getByRole('button',{name:/ALBUM Fixture Album Fixture Artist · 2 tracks/})).toBeVisible();
 await page.getByRole('button',{name:'All artist tracks',exact:true}).click();
 await expect(page.getByRole('button',{name:/MUSIC ▶ Second track/})).toBeVisible();
 await page.getByRole('button',{name:'Back to artists',exact:true}).click();
 await expect(page.getByRole('button',{name:/ARTIST Fixture Artist/})).toBeVisible();
 const cover=await page.evaluate(async id=>{
  const image=new Image();image.src='/api/v1/items/'+id+'/artwork';await image.decode();
  return [image.naturalWidth,image.naturalHeight];
 },tracks[0].id);
 expect(cover).toEqual([32,32]);
 expect((await page.request.post('/api/v1/lists',{headers,data:{Name:'Music queue',Kind:'queue',ItemIDs:tracks.map((t:{id:number})=>t.id)}})).status()).toBe(201);
 await page.getByRole('button',{name:'Collections & playlists',exact:true}).click();
 await page.getByRole('button',{name:'Music queue · queue',exact:true}).click();
 await page.getByRole('button',{name:'Resume queue',exact:true}).click();
 const player=page.getByLabel('Music player',{exact:true});
 await expect(page.locator('.player h2')).toHaveText('First track');
 await expect(page.getByText('Fixture Artist · Fixture Album',{exact:true})).toBeVisible();
 await expect.poll(()=>player.evaluate((a:HTMLAudioElement)=>a.currentTime)).toBeGreaterThan(.25);
 await page.getByRole('button',{name:'Show lyrics',exact:true}).click();
 await player.evaluate((a:HTMLAudioElement)=>a.pause());
 await page.getByRole('button',{name:'Jump here',exact:true}).click();
 await expect.poll(()=>player.evaluate((a:HTMLAudioElement)=>a.currentTime)).toBeCloseTo(2, 2);
 await player.evaluate(async(a:HTMLAudioElement)=>{a.currentTime=a.duration-.15;await a.play();});
 await expect(page.locator('.player h2')).toHaveText('Second track');
 await expect.poll(()=>page.locator('.player img').evaluate((image:HTMLImageElement)=>image.naturalWidth)).toBe(32);
 await expect(page.getByRole('list',{name:'Lyrics',exact:true})).toHaveCount(0);
 await expect.poll(()=>player.evaluate((a:HTMLAudioElement)=>a.currentTime)).toBeGreaterThan(.25);
 await page.getByRole('button',{name:'Close player',exact:true}).click();
 await expect(player).toHaveCount(0);
 if(browserName==='chromium'){
  const playback='**/api/v1/items/'+tracks[0].id+'/playback';
  await page.route(playback,async route=>{
   const body=route.request().postDataJSON();
   expect(body.Capabilities.containers).toContain('hls');
   body.Capabilities.containers=['hls'];
   await route.continue({postData:JSON.stringify(body)});
  },{times:1});
  const created=page.waitForResponse(response=>response.url().endsWith('/items/'+tracks[0].id+'/playback')&&response.request().method()==='POST');
  await page.getByRole('button',{name:'Play queue from here',exact:true}).first().click();
  const response=await created;
  expect(response.status()).toBe(201);
  const session=await response.json();
  await expect(page.getByText('AAC HLS music conversion',{exact:true})).toBeVisible();
  await expect.poll(()=>player.evaluate((a:HTMLAudioElement)=>a.currentTime)).toBeGreaterThan(.25);
  await page.getByRole('button',{name:'Close player',exact:true}).click();
  await expect.poll(async()=> (await page.request.get(session.url)).status()).toBe(404);
 }

});
