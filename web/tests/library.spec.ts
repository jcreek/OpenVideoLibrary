import {test,expect,type Route,type Page,type Cookie} from '@playwright/test';
import {spawn,execFileSync,type ChildProcess} from 'node:child_process';
import {mkdtempSync,mkdirSync,rmSync,writeFileSync,copyFileSync,renameSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
let server:ChildProcess,root:string,media:string,music:string,serverLog='';
test.afterEach(async({},testInfo)=>{if(testInfo.status!==testInfo.expectedStatus)await testInfo.attach('server-log',{body:serverLog,contentType:'text/plain'});});
test.beforeEach(async({browser},testInfo)=>{testInfo.annotations.push({type:'browser-version',description:browser.version()});});
const username='browser-admin',password='browser-test-password-123';
// The server allows 20 sign-ins a minute from one address, so tests reuse a session that is still valid.
let sessionCookie:Cookie|undefined;
async function signIn(page:Page){
 const headers={'X-OVL-Request':'1'};
 if(sessionCookie){await page.context().addCookies([sessionCookie]);if((await page.request.get('/api/v1/me',{headers})).ok())return;await page.context().clearCookies();}
 expect((await page.request.post('/api/v1/login',{headers,data:{Name:username,Password:password}})).status()).toBe(200);
 sessionCookie=(await page.context().cookies()).find(c=>c.name==='ovl_session');
}
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
test.afterAll(async()=>{
 try{
  // A server that already exited never emits another exit event.
  if(server&&server.exitCode===null&&server.signalCode===null){const exit=new Promise<void>(r=>server.once('exit',()=>r()));server.kill('SIGTERM');await exit;}
 }finally{rmSync(root,{recursive:true,force:true});}
});
test('household workflow and viewer authorization',async({page,browserName})=>{
 await page.goto('/');await page.getByLabel('Username',{exact:true}).fill(username);await page.getByLabel('Password',{exact:true}).fill(password);
 // A login failure after setup succeeded must retry the login, not setup.
 await page.route('**/api/v1/login',route=>route.fulfill({status:503,contentType:'application/json',body:JSON.stringify({error:'login unavailable'})}),{times:1});
 await page.getByRole('button',{name:'Create library',exact:true}).click();await expect(page.getByRole('alert')).toHaveText('login unavailable');
 await page.getByLabel('Password',{exact:true}).fill(password);await page.getByRole('button',{name:'Sign in',exact:true}).click();await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
 await page.getByText('Manage libraries',{exact:true}).click();await page.getByLabel('Name',{exact:true}).fill('Test TV');await page.getByLabel('Server folder').fill(media);await page.getByLabel('Media type').selectOption('tv');await page.getByRole('button',{name:'Add library',exact:true}).click();
 const scanStarted=page.waitForResponse(r=>r.url().endsWith('/api/v1/scan')&&r.request().method()==='POST');
 await page.getByRole('button',{name:'Scan libraries',exact:true}).click();expect((await scanStarted).status()).toBe(202);
 await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 await page.getByRole('button',{name:'Refresh library',exact:true}).click();await expect(page.getByRole('heading',{name:'Example S01E01',exact:true})).toBeVisible();
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
 // A progress save still in flight must not undo a later Mark watched.
 let releaseSave=()=>{};const saveGate=new Promise<void>(r=>{releaseSave=r;});let saveSeen=()=>{};const saveHeld=new Promise<void>(r=>{saveSeen=r;});let saveSent=()=>{};const saveDone=new Promise<void>(r=>{saveSent=r;});
 // The held save reaches the server only after release, so an unserialized mark would land first.
 await page.route('**/api/v1/items/*/progress',async route=>{saveSeen();await saveGate;const response=await route.fetch();await route.fulfill({response});saveSent();},{times:1});
 const marked=page.waitForResponse(r=>r.url().endsWith('/progress')&&r.request().method()==='PUT'&&r.request().postDataJSON().Watched===true);
 const applied=page.waitForRequest(r=>r.url().endsWith('/playback')&&r.method()==='POST');
 await page.getByRole('button',{name:'Apply playback settings',exact:true}).click();await saveHeld;
 await page.getByRole('button',{name:'Mark watched',exact:true}).click();await page.evaluate(()=>new Promise(r=>setTimeout(r,250)));releaseSave();await saveDone;await marked;await applied;await page.evaluate(()=>new Promise(r=>setTimeout(r,250)));
 // Check once, without retrying: the 12-second fixture ending later would also mark it watched.
 expect(await page.getByRole('button',{name:'Mark unwatched',exact:true}).count()).toBe(1);
 expect((await (await page.request.get('/api/v1/items')).json()).find((i:{title:string})=>i.title==='Example S01E01')?.watched).toBeTruthy();
 await page.getByRole('button',{name:'Mark unwatched',exact:true}).click();await expect(page.getByRole('button',{name:'Mark watched',exact:true})).toBeVisible();
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
 await page.getByText('Edit title and metadata',{exact:true}).click();await page.getByLabel('Title',{exact:true}).fill('Edited episode');await page.getByLabel('Overview',{exact:true}).fill('Local metadata survives a refresh.');await page.getByRole('button',{name:'Save metadata',exact:true}).click();await expect(page.getByRole('heading',{name:'Edited episode',exact:true}).first()).toBeVisible();
 await page.getByRole('button',{name:'Close player',exact:true}).click();
 await page.getByText('Manage accounts',{exact:true}).click();await page.getByLabel('Username',{exact:true}).fill('viewer');await page.getByLabel('Initial password').fill('viewer-test-password-123');await page.getByRole('button',{name:'Create viewer',exact:true}).click();await expect(page.getByRole('button',{name:'viewer',exact:true})).toBeVisible();
 const accounts=await page.request.get('/api/v1/users');const viewer=(await accounts.json()).find((u:{name:string})=>u.name==='viewer');expect(viewer.libraries).toEqual([]);
 await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();await page.getByLabel('Username',{exact:true}).fill('viewer');await page.getByLabel('Password',{exact:true}).fill('viewer-test-password-123');await page.getByRole('button',{name:'Sign in',exact:true}).click();await expect(page.getByRole('heading',{name:'Bring your collection.'})).toBeVisible();expect((await page.request.get('/api/v1/items/1/file')).status()).toBe(404);await expect(page.getByText('Manage accounts',{exact:true})).toHaveCount(0);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBeTruthy();
});

test('browse filters reset and account switches drop the previous account state',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
 if(!(await (await page.request.get('/api/v1/libraries')).json()).length){
  expect((await page.request.post('/api/v1/libraries',{headers,data:{Name:'Filter TV',Path:media,Kind:'tv'}})).status()).toBe(201);await page.request.post('/api/v1/scan',{headers});
  await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 }
 const libraryID=(await (await page.request.get('/api/v1/libraries')).json())[0].id;
 const item=(await (await page.request.get('/api/v1/items')).json())[0];
 const created=await page.request.post('/api/v1/lists',{headers,data:{Kind:'playlist',Name:'Private admin playlist',ItemIDs:[item.id]}});expect(created.status()).toBe(201);const listID=(await created.json()).id;
 const viewer='switch-viewer',viewerPassword='switch-viewer-password-123';
 if(!(await (await page.request.get('/api/v1/users')).json()).some((u:{name:string})=>u.name===viewer))expect((await page.request.post('/api/v1/users',{headers,data:{Name:viewer,Password:viewerPassword,Libraries:[]}})).status()).toBe(201);
 const second=await page.request.post('/api/v1/lists',{headers,data:{Kind:'collection',Name:'Second admin collection',ItemIDs:[]}});expect(second.status()).toBe(201);
 // A failed status check must not leave the page stuck while opening.
 await page.route('**/api/v1/status',route=>route.fulfill({status:500,contentType:'application/json',body:JSON.stringify({error:'status unavailable'})}),{times:1});
 await page.goto('/');await expect(page.getByRole('alert')).toContainText('status unavailable');await expect(page.getByText('Opening your library…')).toBeVisible();
 await page.getByRole('button',{name:'Retry',exact:true}).click();await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();await expect(page.getByRole('alert')).toHaveCount(0);
 // A failed startup load keeps the signed-in session and offers a retry.
 await page.route('**/api/v1/lists',route=>route.fulfill({status:500,contentType:'application/json',body:JSON.stringify({error:'lists unavailable'})}),{times:1});
 await page.goto('/');await expect(page.getByRole('alert')).toContainText('lists unavailable');await expect(page.getByRole('button',{name:'Sign out',exact:true})).toBeVisible();
 const reloaded=page.waitForResponse(r=>r.url().endsWith('/api/v1/lists')&&r.request().method()==='GET');
 await page.getByRole('button',{name:'Retry',exact:true}).click();expect((await reloaded).status()).toBe(200);
 await expect(page.getByRole('alert')).toHaveCount(0);await expect(page.getByRole('button',{name:'Retry',exact:true})).toHaveCount(0);
 await page.getByRole('button',{name:'Collections & playlists',exact:true}).click();await expect(page.getByRole('button',{name:'Private admin playlist · playlist',exact:true})).toBeVisible();
 await page.getByRole('button',{name:'All media',exact:true}).click();
 await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
 // Each filter control must load with its new value.
 const continued=page.waitForRequest(r=>r.url().includes('/api/v1/items?')&&new URL(r.url()).searchParams.get('continue')==='true');
 await page.getByLabel('Continue playing',{exact:true}).check();await continued;
 const uncontinued=page.waitForRequest(r=>r.url().includes('/api/v1/items?')&&new URL(r.url()).searchParams.get('continue')==='false');
 await page.getByLabel('Continue playing',{exact:true}).uncheck();await uncontinued;
 const chosen=page.waitForRequest(r=>r.url().includes('/api/v1/items?')&&new URL(r.url()).searchParams.get('library')===String(libraryID));
 // A library chosen from the selector must still match its series.
 await page.getByLabel('Library',{exact:true}).selectOption(String(libraryID));await chosen;
 await page.getByRole('button',{name:'TV series',exact:true}).click();await page.getByRole('button',{name:/TV SERIES Example/}).click();
 await expect(page.getByRole('heading',{name:'Example',exact:true})).toBeVisible();
 const season=page.getByLabel('Season',{exact:true});await season.fill('1');await season.press('Tab');
 const cleared=page.waitForRequest(r=>r.url().includes('/api/v1/items?')&&new URL(r.url()).searchParams.get('series')==='Example'&&new URL(r.url()).searchParams.get('season')!=='1');
 await season.fill('');await season.press('Tab');expect(new URL((await cleared).url()).searchParams.get('season')).toBe('');
 const unfiltered=page.waitForRequest(r=>r.url().includes('/api/v1/items?')&&new URL(r.url()).searchParams.get('series')==='');
 await page.getByRole('button',{name:'All media',exact:true}).click();await unfiltered;
 await expect(page.getByRole('heading',{name:'Your library',exact:true})).toBeVisible();
 await page.getByRole('button',{name:'Collections & playlists',exact:true}).click();
 // Only the most recently chosen list may be shown.
 let releaseFirst=()=>{};const firstGate=new Promise<void>(r=>{releaseFirst=r;});let firstSeen=()=>{};const firstHeld=new Promise<void>(r=>{firstSeen=r;});let firstSent=()=>{};const firstDone=new Promise<void>(r=>{firstSent=r;});
 await page.route('**/api/v1/lists/'+listID,async route=>{const response=await route.fetch();firstSeen();await firstGate;await route.fulfill({response});firstSent();},{times:1});
 await page.getByRole('button',{name:'Private admin playlist · playlist',exact:true}).click();await firstHeld;
 await page.getByRole('button',{name:'Second admin collection · collection',exact:true}).click();await expect(page.getByRole('heading',{name:'Second admin collection',exact:true})).toBeVisible();
 releaseFirst();await firstDone;await page.evaluate(()=>new Promise(r=>setTimeout(r,250)));
 await expect(page.getByRole('heading',{name:'Second admin collection',exact:true})).toBeVisible();await expect(page.getByRole('heading',{name:'Private admin playlist',exact:true})).toHaveCount(0);
 await page.getByRole('button',{name:'Private admin playlist · playlist',exact:true}).click();
 await expect(page.getByRole('heading',{name:'Private admin playlist',exact:true})).toBeVisible();await expect(page.getByText(item.title,{exact:true})).toBeVisible();
 await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();
 // A failed load straight after signing in keeps the session and offers a retry.
 await page.route('**/api/v1/libraries',route=>route.fulfill({status:500,contentType:'application/json',body:JSON.stringify({error:'libraries unavailable'})}),{times:1});
 await page.getByLabel('Username',{exact:true}).fill(viewer);await page.getByLabel('Password',{exact:true}).fill(viewerPassword);await page.getByRole('button',{name:'Sign in',exact:true}).click();
 await expect(page.getByRole('alert')).toContainText('libraries unavailable');await expect(page.getByRole('button',{name:'Sign out',exact:true})).toBeVisible();
 const librariesReloaded=page.waitForResponse(r=>r.url().endsWith('/api/v1/libraries')&&r.request().method()==='GET');
 await page.getByRole('button',{name:'Retry',exact:true}).click();expect((await librariesReloaded).status()).toBe(200);
 await expect(page.getByRole('heading',{name:'Bring your collection.'})).toBeVisible();await expect(page.getByRole('alert')).toHaveCount(0);
 await expect(page.getByText('Private admin playlist')).toHaveCount(0);await expect(page.getByText(item.title,{exact:true})).toHaveCount(0);
 // A response requested by the previous account must not restore its list after the switch.
 await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();
 await page.getByLabel('Username',{exact:true}).fill(username);await page.getByLabel('Password',{exact:true}).fill(password);await page.getByRole('button',{name:'Sign in',exact:true}).click();
 await page.getByRole('button',{name:'Collections & playlists',exact:true}).click();
 let release=()=>{};const gate=new Promise<void>(r=>{release=r;});let fetched=()=>{};const held=new Promise<void>(r=>{fetched=r;});let sent=()=>{};const delivered=new Promise<void>(r=>{sent=r;});
 await page.route('**/api/v1/lists/'+listID,async route=>{const response=await route.fetch();fetched();await gate;await route.fulfill({response});sent();},{times:1});
 await page.getByRole('button',{name:'Private admin playlist · playlist',exact:true}).click();await held;
 await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();
 await page.getByLabel('Username',{exact:true}).fill(viewer);await page.getByLabel('Password',{exact:true}).fill(viewerPassword);await page.getByRole('button',{name:'Sign in',exact:true}).click();
 await expect(page.getByRole('button',{name:'Sign out',exact:true})).toBeVisible();
 release();await delivered;await page.evaluate(()=>new Promise(r=>setTimeout(r,250)));
 await expect(page.getByText('Private admin playlist')).toHaveCount(0);await expect(page.getByText(item.title,{exact:true})).toHaveCount(0);await expect(page.getByRole('alert')).toHaveCount(0);
});

test('persistent queue ordering and automatic next episode',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
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
 await signIn(page);
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

test('HLS timeline follows the server-reported stream start',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
 if(!(await (await page.request.get('/api/v1/libraries')).json()).length)expect((await page.request.post('/api/v1/libraries',{headers,data:{Name:'Timeline TV',Path:media,Kind:'tv'}})).status()).toBe(201);
 // Earlier scenarios can leave titles offline until the next scan.
 await page.request.post('/api/v1/scan',{headers});await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 const item=(await (await page.request.get('/api/v1/items')).json()).sort((a:{id:number},b:{id:number})=>a.id-b.id).at(-1);
 await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
 await page.getByRole('button',{name:'All media',exact:true}).click();await page.getByRole('button',{name:new RegExp('TV ▶ '+item.title)}).click();
 await page.getByLabel('Playback',{exact:true}).selectOption('remux');await page.getByRole('button',{name:'Apply playback settings',exact:true}).click();
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
 // Remux streams can begin on an earlier keyframe than requested. Report the
 // stream as beginning 2 s before the requested position.
 let actual=-1;
 await page.route('**/api/v1/items/'+item.id+'/playback',async route=>{
  // Only the seek to 6 s is altered; settings-driven restarts pass through.
  if(route.request().method()!=='POST'||route.request().postDataJSON().Start!==6)return route.fallback();
  const response=await route.fetch();const body=await response.json();
  actual=body.start;body.start=Math.max(0,body.start-2);await route.fulfill({response,json:body});
 });
 const created=page.waitForResponse(r=>r.url().endsWith('/items/'+item.id+'/playback')&&r.request().method()==='POST'&&r.request().postDataJSON().Start===6);
 await page.getByLabel(/Jump to/).fill('6');
 const response=await created;
 const reported=(await response.json()).start;
 // The real remux start is the measured stream start, at most one GOP early.
 expect(actual).toBeLessThanOrEqual(6);expect(actual).toBeGreaterThan(4.9);expect(reported).toBeCloseTo(actual-2,6);
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
 // The client skips the 2 s lead-in to land on the requested position.
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.currentTime)).toBeGreaterThanOrEqual(1.9);
 const saved=page.waitForRequest(r=>r.url().endsWith('/items/'+item.id+'/progress')&&r.method()==='PUT');
 const current=await page.locator('video').evaluate((v:HTMLVideoElement)=>{v.pause();return v.currentTime;});
 const position=(await saved).postDataJSON().Position;
 expect(Math.abs(position-(reported+current))).toBeLessThan(0.25);
 expect(position).toBeGreaterThanOrEqual(5.9);
 await page.getByRole('button',{name:'Close player',exact:true}).click();
});
test('an expired session stops playback and every timer for the previous account',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
 if(!(await (await page.request.get('/api/v1/libraries')).json()).length)expect((await page.request.post('/api/v1/libraries',{headers,data:{Name:'Expiry TV',Path:media,Kind:'tv'}})).status()).toBe(201);
 await page.request.post('/api/v1/scan',{headers});await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 // Earlier scenarios rename the first title; the last one keeps its TV label.
 const item=(await (await page.request.get('/api/v1/items')).json()).sort((a:{id:number},b:{id:number})=>a.id-b.id).at(-1);
 // The client cannot delete its playback session once signed out, so the test removes it afterwards.
 const sessions=new Set<string>();
 page.on('request',r=>{const m=new URL(r.url()).pathname.match(/^\/api\/v1\/playback\/([A-Za-z0-9_-]+)\//);if(m)sessions.add(m[1]);});
 try {
 await page.clock.install();
 await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
 await page.getByRole('button',{name:'All media',exact:true}).click();await page.getByRole('button',{name:new RegExp('TV ▶ '+item.title)}).click();
 await page.getByLabel('Playback',{exact:true}).selectOption('remux');await page.getByRole('button',{name:'Apply playback settings',exact:true}).click();
 await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
 // The server session ends; the next request from the page receives 401.
 expect((await page.request.post('/api/v1/logout',{headers})).status()).toBeLessThan(300);
 // Any request can receive the 401 first; the click only makes sure one is sent.
 await page.getByRole('button',{name:'All media',exact:true}).click({timeout:2000}).catch(()=>{});
 await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();
 await expect(page.getByRole('alert')).toHaveText('Your session has ended. Sign in again.');
 // Nothing from the previous account keeps running: no progress saves, playlist or segment loads.
 const after:string[]=[];
 page.on('request',r=>{const u=new URL(r.url());if(u.pathname.startsWith('/api/v1/'))after.push(r.method()+' '+u.pathname);});
 await page.clock.runFor(31000);
 await page.waitForTimeout(1000);
 expect(after).toEqual([]);
 expect(sessions.size).toBeGreaterThan(0);
 } finally {
  // Cleanup must not replace the test's own failure, so it never throws.
  try{await page.request.post('/api/v1/login',{headers,data:{Name:username,Password:password}});
   for(const id of sessions)await page.request.delete('/api/v1/playback/'+id,{headers});}catch{}
 }
});

test('a 401 to an HLS playlist or a subtitle request also ends the session',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
 if(!(await (await page.request.get('/api/v1/libraries')).json()).length)expect((await page.request.post('/api/v1/libraries',{headers,data:{Name:'Expiry TV',Path:media,Kind:'tv'}})).status()).toBe(201);
 await page.request.post('/api/v1/scan',{headers});await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 const item=(await (await page.request.get('/api/v1/items')).json()).sort((a:{id:number},b:{id:number})=>a.id-b.id).at(-1);
 const sessions=new Set<string>();
 page.on('response',async r=>{if(r.request().method()==='POST'&&new URL(r.url()).pathname==='/api/v1/items/'+item.id+'/playback'&&r.ok())sessions.add((await r.json()).id);});
 // Destroying hls.js from inside its own error handler must not throw.
 const pageErrors:string[]=[];page.on('pageerror',e=>pageErrors.push(e.message));
 const unauthorized={status:401,contentType:'application/json',body:JSON.stringify({error:'authentication required'})};
 const open=async()=>{
  await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  await page.getByRole('button',{name:'All media',exact:true}).click();await page.getByRole('button',{name:new RegExp('TV ▶ '+item.title)}).click();
 };
 try {
  // Only the HLS loader sees this 401; no api() request does.
  await page.route('**/api/v1/playback/*/**',route=>route.request().method()==='GET'?route.fulfill(unauthorized):route.fallback());
  await open();
  // If automatic playback already uses HLS, the 401 can end the session before these steps.
  await page.getByLabel('Playback',{exact:true}).selectOption('remux',{timeout:2000}).catch(()=>{});
  // The click only makes sure playback starts.
  await page.getByRole('button',{name:'Apply playback settings',exact:true}).click({timeout:2000}).catch(()=>{});
  await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();
  await expect(page.getByRole('alert')).toHaveText('Your session has ended. Sign in again.');
  await page.unroute('**/api/v1/playback/*/**');
  // The subtitle request uses its own fetch.
  await page.route('**/api/v1/items/*/subtitles',route=>route.fulfill(unauthorized));
  await open();
  await page.getByLabel('Subtitles',{exact:true}).selectOption('external');
  await page.getByRole('button',{name:'Apply playback settings',exact:true}).click({timeout:2000}).catch(()=>{});
  await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();
  await expect(page.getByRole('alert')).toHaveText('Your session has ended. Sign in again.');
  expect(pageErrors).toEqual([]);
 } finally {
  await page.unrouteAll({behavior:'ignoreErrors'});
  try{for(const id of sessions)await page.request.delete('/api/v1/playback/'+id,{headers});}catch{}
 }
});

test('a request that fails after the session ends keeps the session-ended message',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
 const unauthorized={status:401,contentType:'application/json',body:JSON.stringify({error:'authentication required'})};
 try {
  await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  // A library load is still in flight when another request finds the session has ended.
  const held:Route[]=[];
  await page.route(u=>u.pathname==='/api/v1/items',route=>{held.push(route);});
  await page.getByRole('button',{name:'All media',exact:true}).click();
  await expect.poll(()=>held.length).toBeGreaterThan(0);
  await page.route(u=>u.pathname==='/api/v1/music/albums',route=>route.fulfill(unauthorized));
  await page.getByRole('button',{name:'Music albums',exact:true}).click();
  await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();
  // The earlier request then fails with a network error.
  for(const route of held)await route.abort('failed');
  await page.waitForTimeout(1000);
  await expect(page.getByRole('alert')).toHaveText('Your session has ended. Sign in again.');
 } finally {
  await page.unrouteAll({behavior:'ignoreErrors'});
 }
});

test('a signed-in 401 ends the session even when its body cannot be read',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
 const albumsURL=(u:URL)=>u.pathname==='/api/v1/music/albums';
 try {
  await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  // A proxy or truncated transfer can leave a 401 without a JSON body.
  await page.route(albumsURL,route=>route.fulfill({status:401,contentType:'application/json',body:'<html>'}));
  await page.getByRole('button',{name:'Music albums',exact:true}).click();
  await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();
  await expect(page.getByRole('alert')).toHaveText('Your session has ended. Sign in again.');
 } finally {
  await page.unrouteAll({behavior:'ignoreErrors'});
 }
});

// Signing out can be held up by player cleanup or by the logout request itself. If the
// session ends meanwhile and the user signs in again, the old sign-out must not touch them.
for(const held of ['cleanup','logout'] as const)test('an interrupted sign-out does not sign out the next account ('+held+' held)',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
 if(!(await (await page.request.get('/api/v1/libraries')).json()).length)expect((await page.request.post('/api/v1/libraries',{headers,data:{Name:'Sign-out TV',Path:media,Kind:'tv'}})).status()).toBe(201);
 await page.request.post('/api/v1/scan',{headers});await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 const item=(await (await page.request.get('/api/v1/items')).json()).sort((a:{id:number},b:{id:number})=>a.id-b.id).at(-1);
 const unauthorized={status:401,contentType:'application/json',body:JSON.stringify({error:'authentication required'})};
 const routes:Route[]=[];let holding=true;const responses=new Map<Route,ReturnType<Route['fetch']>>();
 // A delayed logout is answered with the server's real response; a held DELETE is sent on.
 const release=async(route:Route)=>{const response=responses.get(route);if(response)await route.fulfill({response:await response});else await route.continue();};
 const deleteURL=(u:URL)=>/^\/api\/v1\/playback\/[^/]+$/.test(u.pathname),logoutURL=(u:URL)=>u.pathname==='/api/v1/logout',albumsURL=(u:URL)=>u.pathname==='/api/v1/music/albums';
 const logouts:string[]=[];page.on('request',r=>{if(logoutURL(new URL(r.url())))logouts.push(r.method());});
 try {
  await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  if(held==='cleanup'){
   // A remux session makes closing the player send a DELETE, which is held.
   const automatic=page.waitForResponse(r=>r.request().method()==='POST'&&new URL(r.url()).pathname==='/api/v1/items/'+item.id+'/playback');
   await page.getByRole('button',{name:'All media',exact:true}).click();await page.getByRole('button',{name:new RegExp('TV ▶ '+item.title)}).click();await automatic;
   await expect(page.getByRole('button',{name:'Apply playback settings',exact:true})).toBeEnabled();
   await page.getByLabel('Playback',{exact:true}).selectOption('remux');
   const created=page.waitForResponse(r=>r.request().method()==='POST'&&new URL(r.url()).pathname==='/api/v1/items/'+item.id+'/playback'&&r.ok());
   await page.getByRole('button',{name:'Apply playback settings',exact:true}).click();await created;
   await page.route(deleteURL,route=>{if(!holding||route.request().method()!=='DELETE')return route.fallback();routes.push(route);});
  }else{
   // The server handles the logout at once; only its response is delayed.
   await page.route(logoutURL,route=>{if(!holding)return route.fallback();responses.set(route,route.fetch());routes.push(route);});
  }
  await page.getByRole('button',{name:'Sign out',exact:true}).click();
  await expect.poll(()=>routes.length).toBe(1);
  // Another request finds the session has ended, and the user signs in again.
  await page.route(albumsURL,route=>route.fulfill(unauthorized));
  await page.getByRole('button',{name:'Music albums',exact:true}).click();
  await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();
  await page.unroute(albumsURL);
  // The old sign-out finishes after the new sign-in has returned.
  const signedIn=page.waitForResponse(r=>new URL(r.url()).pathname==='/api/v1/login');
  await page.getByLabel('Username').fill(username);await page.getByLabel('Password').fill(password);await page.getByRole('button',{name:'Sign in',exact:true}).click();
  await signedIn;
  holding=false;for(const route of routes)await release(route).catch(()=>{});
  await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  await page.waitForTimeout(1000);
  // The new account is still signed in, and its cookie still works.
  const items=page.waitForResponse(r=>new URL(r.url()).pathname==='/api/v1/items');
  await page.getByRole('button',{name:'All media',exact:true}).click();
  expect((await items).status()).toBe(200);
  await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0);
  expect(logouts.length).toBe(held==='logout'?1:0);
 } finally {
  holding=false;for(const route of routes)await release(route).catch(()=>{});
  await page.unrouteAll({behavior:'ignoreErrors'});
 }
});

test('a late logout response does not sign out the next account',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
 const logoutURL=(u:URL)=>u.pathname==='/api/v1/logout';
 const routes:Route[]=[];let holding=true;const responses=new Map<Route,ReturnType<Route['fetch']>>();
 const release=async(route:Route)=>{await route.fulfill({response:await responses.get(route)!});};
 try {
  await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  // Two clicks send two logout requests. The server handles both at once, and the second
  // response arrives first.
  await page.route(logoutURL,route=>{if(!holding)return route.fallback();responses.set(route,route.fetch());routes.push(route);});
  await page.getByRole('button',{name:'Sign out',exact:true}).click();
  await expect.poll(()=>routes.length).toBe(1);
  await page.getByRole('button',{name:'Sign out',exact:true}).click();
  await expect.poll(()=>routes.length).toBe(2);
  await release(routes[1]);
  await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();
  // The first logout response is still pending when the user signs in again.
  const signedIn=page.waitForResponse(r=>new URL(r.url()).pathname==='/api/v1/login');
  await page.getByLabel('Username').fill(username);await page.getByLabel('Password').fill(password);await page.getByRole('button',{name:'Sign in',exact:true}).click();
  await signedIn;
  holding=false;await release(routes[0]);
  await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  // The new account's cookie survived the late logout response.
  const items=page.waitForResponse(r=>new URL(r.url()).pathname==='/api/v1/items');
  await page.getByRole('button',{name:'All media',exact:true}).click();
  expect((await items).status()).toBe(200);
  await expect(page.getByRole('alert')).toHaveCount(0);
 } finally {
  holding=false;for(const route of routes)await release(route).catch(()=>{});
  await page.unrouteAll({behavior:'ignoreErrors'});
 }
});

test('a progress save that never answers does not hold up later saves',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
 if(!(await (await page.request.get('/api/v1/libraries')).json()).length)expect((await page.request.post('/api/v1/libraries',{headers,data:{Name:'Progress TV',Path:media,Kind:'tv'}})).status()).toBe(201);
 await page.request.post('/api/v1/scan',{headers});await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 const item=(await (await page.request.get('/api/v1/items')).json()).sort((a:{id:number},b:{id:number})=>a.id-b.id).at(-1);
 const progressURL=(u:URL)=>u.pathname==='/api/v1/items/'+item.id+'/progress';
 const held:Route[]=[];const bodies:{Client:string;Sequence:number;Watched:boolean;Position:number}[]=[];
 page.on('request',r=>{if(r.method()==='PUT'&&progressURL(new URL(r.url())))bodies.push(r.postDataJSON());});
 await page.clock.install();
 try {
  await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  const automatic=page.waitForResponse(r=>r.request().method()==='POST'&&new URL(r.url()).pathname==='/api/v1/items/'+item.id+'/playback');
  await page.getByRole('button',{name:'All media',exact:true}).click();await page.getByRole('button',{name:new RegExp('TV ▶ '+item.title)}).click();await automatic;
  // The first save never gets an answer.
  await page.route(progressURL,route=>{if(held.length===0){held.push(route);return;}return route.fallback();});
  const mark=page.getByRole('button',{name:/^Mark (un)?watched$/});
  await mark.click();
  await expect.poll(()=>held.length).toBe(1);
  await page.clock.runFor(10000);
  await expect(page.getByRole('alert')).toHaveText('Progress may not have been saved: the server did not answer in time.');
  // The next save is sent and stored even though the first never answered.
  const saved=page.waitForResponse(r=>r.request().method()==='PUT'&&progressURL(new URL(r.url())));
  await mark.click();
  const response=await saved;expect(response.status()).toBe(200);expect((await response.json()).applied).toBe(true);
  const first=bodies[0],last=bodies.at(-1)!;
  expect(last.Client).toBe(first.Client);expect(last.Sequence).toBeGreaterThan(first.Sequence);
  // If the abandoned save reaches the server late, it is ignored.
  const late=await page.request.put('/api/v1/items/'+item.id+'/progress',{headers,data:first});
  expect(late.status()).toBe(200);expect((await late.json()).applied).toBe(false);
 } finally {
  for(const route of held)await route.abort().catch(()=>{});
  await page.unrouteAll({behavior:'ignoreErrors'});
  // Later tests find this title by its card label, which changes once it is watched.
  try{await page.request.put('/api/v1/items/'+item.id+'/progress',{headers,data:{Position:item.position,Watched:item.watched}});}catch{}
 }
});

test('a failed progress save does not stop the next title from opening',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
 if(!(await (await page.request.get('/api/v1/libraries')).json()).length)expect((await page.request.post('/api/v1/libraries',{headers,data:{Name:'Progress TV',Path:media,Kind:'tv'}})).status()).toBe(201);
 await page.request.post('/api/v1/scan',{headers});await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 const items=(await (await page.request.get('/api/v1/items')).json()).sort((a:{id:number},b:{id:number})=>a.id-b.id).slice(0,2);
 const card=(title:string)=>page.locator('.card').filter({has:page.getByRole('heading',{name:title,exact:true})});
 const progressURL=(u:URL)=>/^\/api\/v1\/items\/\d+\/progress$/.test(u.pathname);
 try {
  await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  await card(items[0].title).click();
  await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(1);
  // Saving the first title's progress fails, but the second title still opens.
  await page.route(progressURL,route=>route.fulfill({status:500,contentType:'application/json',body:JSON.stringify({error:'progress store unavailable'})}));
  await card(items[1].title).click();
  await expect(page.getByRole('alert')).toHaveText('progress store unavailable');
  await expect(page.locator('.player h2')).toHaveText(items[1].title);
  await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(1);
  await page.unroute(progressURL);
  await page.getByRole('button',{name:'Close player',exact:true}).click();
 } finally {
  await page.unrouteAll({behavior:'ignoreErrors'});
 }
});

test('player cleanup that never answers does not block the next title or signing out',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
 if(!(await (await page.request.get('/api/v1/libraries')).json()).length)expect((await page.request.post('/api/v1/libraries',{headers,data:{Name:'Cleanup TV',Path:media,Kind:'tv'}})).status()).toBe(201);
 await page.request.post('/api/v1/scan',{headers});await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 const items=(await (await page.request.get('/api/v1/items')).json()).sort((a:{id:number},b:{id:number})=>a.id-b.id).slice(0,2);
 const card=(title:string)=>page.locator('.card').filter({has:page.getByRole('heading',{name:title,exact:true})});
 const deleteURL=(u:URL)=>/^\/api\/v1\/playback\/[^/]+$/.test(u.pathname);
 const held:Route[]=[];const sessions:string[]=[];
 const remux=async(id:number)=>{
  await expect(page.getByRole('button',{name:'Apply playback settings',exact:true})).toBeEnabled();
  await page.getByLabel('Playback',{exact:true}).selectOption('remux');
  const created=page.waitForResponse(r=>r.request().method()==='POST'&&new URL(r.url()).pathname==='/api/v1/items/'+id+'/playback'&&r.ok());
  await page.getByRole('button',{name:'Apply playback settings',exact:true}).click();
  sessions.push((await (await created).json()).id);
 };
 const opened=(id:number)=>page.waitForResponse(r=>r.request().method()==='POST'&&new URL(r.url()).pathname==='/api/v1/items/'+id+'/playback');
 await page.clock.install();
 try {
  await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  let automatic=opened(items[0].id);await card(items[0].title).click();await automatic;
  await remux(items[0].id);
  await page.route(deleteURL,route=>{if(route.request().method()!=='DELETE')return route.fallback();held.push(route);});
  // Opening another title sends a DELETE that never answers; the new title starts anyway,
  // in remux mode like the first, so it also has a session for sign-out to delete.
  automatic=opened(items[1].id);await card(items[1].title).click();
  await expect.poll(()=>held.length).toBe(1);
  await page.clock.runFor(10000);
  const second=await automatic;expect(second.status()).toBe(201);sessions.push((await second.json()).id);
  await expect(page.locator('.player h2')).toHaveText(items[1].title);
  // Signing out also sends a DELETE that never answers; the sign-out finishes anyway.
  await page.getByRole('button',{name:'Sign out',exact:true}).click();
  await expect.poll(()=>held.length).toBe(2);
  await page.clock.runFor(10000);
  await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0);
 } finally {
  for(const route of held)await route.abort().catch(()=>{});
  await page.unrouteAll({behavior:'ignoreErrors'});
  // The held DELETEs never reached the server, so remove their sessions to free the stream slots.
  try{await signIn(page);for(const id of sessions)await page.request.delete('/api/v1/playback/'+id,{headers});}catch{}
 }
});

// The old DELETE comes either from close() itself or, when the player closes while the
// playback request is in flight, from the late cleanup once that request returns.
for(const whileCreating of [false,true])test('the next account does not wait for the previous account\'s playback cleanup'+(whileCreating?' (closed while starting)':''),async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
 if(!(await (await page.request.get('/api/v1/libraries')).json()).length)expect((await page.request.post('/api/v1/libraries',{headers,data:{Name:'Cleanup TV',Path:media,Kind:'tv'}})).status()).toBe(201);
 await page.request.post('/api/v1/scan',{headers});await expect.poll(async()=>(await (await page.request.get('/api/v1/scan')).json()).running).toBe(false);
 const item=(await (await page.request.get('/api/v1/items')).json()).sort((a:{id:number},b:{id:number})=>a.id-b.id).at(-1);
 const unauthorized={status:401,contentType:'application/json',body:JSON.stringify({error:'authentication required'})};
 // DELETEs are held only until the new account's player is checked, so later cleanup is not leaked.
 const held:Route[]=[];let holding=true;
 const openItem=async()=>{await page.getByRole('button',{name:'All media',exact:true}).click();await page.getByRole('button',{name:new RegExp('TV ▶ '+item.title)}).click();};
 const isCreation=(u:URL,method:string)=>method==='POST'&&u.pathname==='/api/v1/items/'+item.id+'/playback';
 try {
  await page.goto('/');await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  // Opening a title starts automatic playback first; wait for it so later steps see only the remux request.
  const automatic=page.waitForResponse(r=>isCreation(new URL(r.url()),r.request().method()));
  await openItem();await automatic;
  await expect(page.getByRole('button',{name:'Apply playback settings',exact:true})).toBeEnabled();
  await page.getByLabel('Playback',{exact:true}).selectOption('remux');
  await page.route(u=>/^\/api\/v1\/playback\/[^/]+$/.test(u.pathname),route=>{if(!holding||route.request().method()!=='DELETE')return route.fallback();held.push(route);});
  let target='';
  if(whileCreating){
   const creations:Route[]=[];
   // unroute() matches the same predicate object, so keep it.
   const creationURL=(u:URL)=>u.pathname==='/api/v1/items/'+item.id+'/playback';
   await page.route(creationURL,route=>{if(route.request().method()!=='POST')return route.fallback();creations.push(route);});
   await page.getByRole('button',{name:'Apply playback settings',exact:true}).click();
   await expect.poll(()=>creations.length).toBe(1);
   await page.getByRole('button',{name:'Close player',exact:true}).click();
   await expect(page.getByRole('button',{name:'Close player',exact:true})).toHaveCount(0);
   // The request returns after the player closed, so the client deletes the new session itself.
   const response=await creations[0].fetch();const body=await response.json();target=body.id;
   await creations[0].fulfill({response,json:body});
   await page.unroute(creationURL);
  }else{
   const created=page.waitForResponse(r=>isCreation(new URL(r.url()),r.request().method())&&r.ok());
   await page.getByRole('button',{name:'Apply playback settings',exact:true}).click();
   target=(await (await created).json()).id;
   await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(2);
   await page.getByRole('button',{name:'Close player',exact:true}).click();
  }
  // That DELETE never gets an answer.
  expect(target).toBeTruthy();
  await expect.poll(()=>held.some(r=>r.request().url().endsWith('/playback/'+target))).toBe(true);
  // The session then ends, and the user signs in again.
  const albumsURL=(u:URL)=>u.pathname==='/api/v1/music/albums';
  await page.route(albumsURL,route=>route.fulfill(unauthorized));
  await page.getByRole('button',{name:'Music albums',exact:true}).click();
  await expect(page.getByRole('heading',{name:'Welcome back.'})).toBeVisible();
  await page.unroute(albumsURL);
  await page.getByLabel('Username').fill(username);await page.getByLabel('Password').fill(password);await page.getByRole('button',{name:'Sign in',exact:true}).click();
  await expect(page.getByRole('heading',{name:'Your library'})).toBeVisible();
  // The new account's player starts while the old DELETE is still unanswered.
  await openItem();
  await expect.poll(()=>page.locator('video').evaluate((v:HTMLVideoElement)=>v.readyState)).toBeGreaterThanOrEqual(1);
  expect(held.some(r=>r.request().url().endsWith('/playback/'+target))).toBe(true);
  holding=false;
  await page.getByRole('button',{name:'Close player',exact:true}).click();
 } finally {
  holding=false;
  // Let the held DELETEs reach the server so their sessions are removed.
  for(const route of held)await route.continue().catch(()=>{});
  await page.unrouteAll({behavior:'ignoreErrors'});
 }
});

test('player lifecycle ignores delayed opening and closing',async({page})=>{
 const headers={'X-OVL-Request':'1'};
 if((await (await page.request.get('/api/v1/status')).json()).setup)expect((await page.request.post('/api/v1/setup',{headers,data:{Name:username,Password:password}})).status()).toBe(201);
 await signIn(page);
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
 await signIn(page);
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
 // [00:01.50] with [offset:+500]: a positive offset shows the line 0.5 s sooner.
 await expect.poll(()=>player.evaluate((a:HTMLAudioElement)=>a.currentTime)).toBeCloseTo(1, 2);
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
