import {test,expect} from '@playwright/test';
import {spawn,execFileSync,type ChildProcess} from 'node:child_process';
import {mkdtempSync,mkdirSync,rmSync,writeFileSync,copyFileSync,renameSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
let server:ChildProcess,root:string,media:string;
test.beforeEach(async({browser},testInfo)=>{testInfo.annotations.push({type:'browser-version',description:browser.version()});});
const username='browser-admin',password='browser-test-password-123';
test.beforeAll(async()=>{
 root=mkdtempSync(join(process.env.OVL_TEST_TMP||tmpdir(),'ovl-browser-'));media=join(root,'media');mkdirSync(media);
 execFileSync('ffmpeg',['-nostdin','-v','error','-f','lavfi','-i','testsrc2=size=640x360:rate=24','-f','lavfi','-i','sine=frequency=440','-t','12','-c:v','libx264','-preset','ultrafast','-g','24','-c:a','aac',join(media,'Example.S01E01.mp4')]);
 copyFileSync(join(media,'Example.S01E01.mp4'),join(media,'Example.S01E02.mp4'));
 writeFileSync(join(media,'Example.S01E01.srt'),'1\n00:00:00,000 --> 00:00:04,000\nA local subtitle\n');
 server=spawn(resolve('../bin/ovl'),[],{env:{...process.env,OVL_DATA:join(root,'data'),OVL_WEB:resolve('dist'),OVL_MIN_FREE_BYTES:'1048576',OVL_TRANSCODE_BYTES:'16777216',OVL_LISTEN:'127.0.0.1:18096',OVL_ACCELERATOR:'cpu',OVL_SECURE_COOKIES:'false'},stdio:'pipe'});
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
 const lists=await (await page.request.get('/api/v1/lists')).json();const queue=lists.find((l:{name:string})=>l.name==='Evening queue');expect(queue).toBeTruthy();
 for(const title of titles){
  await page.getByRole('button',{name:'Movies & episodes',exact:true}).click();await page.getByRole('button',{name:new RegExp('TV ▶ '+title)}).click();await expect(page.locator('.player h2')).toHaveText(title);
  await page.getByLabel('Add to list',{exact:true}).selectOption(String(queue.id));await page.getByRole('button',{name:'Add title',exact:true}).click();await expect(page.getByRole('status').filter({hasText:'Added to Evening queue.'})).toBeVisible();await page.getByRole('button',{name:'Close player',exact:true}).click();
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
