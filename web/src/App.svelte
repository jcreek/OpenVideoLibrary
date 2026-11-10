<script lang="ts">
import {onMount,tick} from 'svelte';
import type HlsType from 'hls.js';
import type {Item,Library,Account,Series,MediaInfo,MediaList,MediaListSummary,MediaListEntry,ClientCapabilities} from './api.generated';
let ready=false,setup=false,logged=false,admin=false,name='',password='',error='',notice='',query='',library='',offset=0,continuing=false,section='library',seriesFilter='',season='';
let items:Item[]=[],libraries:Library[]=[],shows:Series[]=[],users:Account[]=[],selected:Item|null=null,info:MediaInfo|null=null;
let video:HTMLVideoElement,hls:HlsType|undefined,session='',mode='auto',actualMode='',startOffset=0,audio=0,subtitle='off',adaptive=false,busy=false,transition=false,captionURL='',duration=0,seekTo=0;
let libraryName='',path='',kind='movie',newName='',newPassword='',grants:number[]=[],editUser:Account|null=null,resetPassword='';
let playerGeneration=0;
let playerCleanup:Promise<void>=Promise.resolve();
let pendingPlayback:Promise<void>=Promise.resolve();
let frameRequestedAt=0,waitingAt=0,seekRequestedAt=0;
let savedLists:MediaListSummary[]=[],activeList:MediaList|null=null,playingQueue:MediaList|null=null,newListName='',newListKind:'collection'|'playlist'|'queue'='playlist',targetListID='',playbackReason='';
let editTitle='',editOverview='',tmdbID='',tmdbKind='movie',scanning=false;
async function api(url:string,method='GET',body?:unknown){const r=await fetch('/api/v1/'+url,{method,headers:{'Content-Type':'application/json','X-OVL-Request':'1'},body:body===undefined?undefined:JSON.stringify(body)});const data=await r.json();if(!r.ok){if(r.status===401)logged=false;throw Error(data.error||'Request failed');}return data;}
async function guard(fn:()=>Promise<void>){error='';try{await fn()}catch(e){error=e instanceof Error?e.message:String(e)}}
async function load(){items=await api('items?'+new URLSearchParams({q:query,library,offset:String(offset),continue:String(continuing),series:seriesFilter,season}));}
async function refresh(){const me=await api('me');admin=me.Admin;logged=true;libraries=await api('libraries');shows=await api('series');savedLists=await api('lists');if(admin){users=await api('users');scanning=(await api('scan')).running;}await load();}
onMount(()=>{guard(async()=>{setup=(await api('status')).setup;if(!setup){try{await refresh()}catch{logged=false;error='';}}ready=true;});const interval=setInterval(()=>{if(selected&&!transition)guard(save);if(admin&&scanning)guard(async()=>{scanning=(await api('scan')).running;if(!scanning){notice='Scan finished.';await refresh();}});},15000);return()=>{clearInterval(interval);hls?.destroy();if(captionURL)URL.revokeObjectURL(captionURL);};});
async function login(){await guard(async()=>{if(setup)await api('setup','POST',{Name:name,Password:password});await api('login','POST',{Name:name,Password:password});password='';setup=false;await refresh();});}
async function save(){
 const item=selected,element=video;
 if(item&&element&&!transition&&element.readyState>0){
  const position=element.currentTime+startOffset,watched=element.ended||item.watched;
  await api('items/'+item.id+'/progress','PUT',{Position:position,Watched:watched});
  item.position=position;item.watched=watched;
 }
}
function stopPlayer(){
 transition=true;hls?.destroy();hls=undefined;
 if(video){video.pause();video.removeAttribute('src');video.load();}
 const old=session;session='';
 if(captionURL){URL.revokeObjectURL(captionURL);captionURL='';}
 const cleanup=old?api('playback/'+old,'DELETE').catch(()=>{}):Promise.resolve();
 playerCleanup=Promise.all([playerCleanup,cleanup]).then(()=>{});return playerCleanup;
}
async function close(){
 const generation=++playerGeneration,saving=save();
 const cleanup=stopPlayer();selected=null;info=null;busy=false;
 await Promise.all([saving,cleanup]);
 if(generation===playerGeneration){transition=false;await load();}
}
async function open(item:Item,fromQueue=false){
 const generation=++playerGeneration;if(!fromQueue)playingQueue=null;
 const saving=save(),cleanup=stopPlayer();
 selected={...item};info=null;busy=true;audio=0;subtitle='off';editTitle=item.title;editOverview=item.overview;tmdbID='';
 try{
  await Promise.all([saving,cleanup]);if(generation!==playerGeneration)return;
  const probed=await api('items/'+item.id+'/probe');if(generation!==playerGeneration)return;
  info=probed;duration=Number(info?.format.duration)||0;
  await start(item.watched?0:item.position,generation);
 }catch(e){if(generation===playerGeneration)error=e instanceof Error?e.message:String(e);}
 finally{if(generation===playerGeneration){busy=false;transition=false;}}
}
async function start(position:number,generation=++playerGeneration){
 const item=selected;if(!item)return;
 if(duration>0&&position>=duration-.25)position=0;
 busy=true;frameRequestedAt=performance.now();waitingAt=0;
 try{
  await stopPlayer();await pendingPlayback;if(generation!==playerGeneration)return;
  await tick();if(generation!==playerGeneration)return;
  const probe=document.createElement('video');
  const request={Capabilities:clientCapabilities(),Mode:subtitle.startsWith('burn:')?'transcode':mode,Start:position,Audio:audio,Adaptive:adaptive,CanPlayMP4:!!probe.canPlayType('video/mp4; codecs="avc1.42E01E, mp4a.40.2"'),...(subtitle.startsWith('burn:')?{Subtitle:Number(subtitle.slice(5))}:{})};
  // Keep the response observable: a cancelled POST may still create a server session.
  const creation=api('items/'+item.id+'/playback','POST',request);
  pendingPlayback=creation.then(async p=>{if(generation!==playerGeneration&&p.id)await api('playback/'+p.id,'DELETE').catch(()=>{});},()=>{});
  const p=await creation;await pendingPlayback;if(generation!==playerGeneration)return;
  session=p.id||'';actualMode=p.mode;playbackReason=p.reason||'';startOffset=p.mode==='direct'?0:position;seekTo=position;
  await tick();if(generation!==playerGeneration)return;
  if(p.mode!=='direct'){
   const {default:Hls}=await import('hls.js');if(generation!==playerGeneration)return;
   if(Hls.isSupported()){
    const instance=new Hls({startPosition:0});hls=instance;
    instance.on(Hls.Events.ERROR,(_event,data)=>{if(hls===instance&&data.fatal)error='Playback failed: '+data.details;});
    instance.loadSource(p.url);instance.attachMedia(video);
   }else video.src=p.url;
  }else video.src=p.url;
  transition=false;if(subtitle.startsWith('text:')||subtitle==='external')await captions(generation);
 }finally{if(generation===playerGeneration){busy=false;transition=false;}}
}
async function captions(generation=playerGeneration){if(!selected)return;const offset=startOffset;const r=await fetch('/api/v1/items/'+selected.id+'/subtitles',{method:'POST',headers:{'Content-Type':'application/json','X-OVL-Request':'1'},body:JSON.stringify({Index:subtitle==='external'?0:Number(subtitle.slice(5)),External:subtitle==='external'})});if(!r.ok)throw Error((await r.json()).error);let vtt=await r.text();if(generation!==playerGeneration)return;if(offset){vtt=vtt.replace(/(\d{2,}):(\d{2}):(\d{2})\.(\d{3})/g,(_all,h,m,s,ms)=>{const t=Math.max(0,(Number(h)*3600+Number(m)*60+Number(s))*1000+Number(ms)-offset*1000);return String(Math.floor(t/3600000)).padStart(2,'0')+':'+String(Math.floor(t/60000)%60).padStart(2,'0')+':'+String(Math.floor(t/1000)%60).padStart(2,'0')+'.'+String(Math.floor(t)%1000).padStart(3,'0');});}if(captionURL)URL.revokeObjectURL(captionURL);captionURL=URL.createObjectURL(new Blob([vtt],{type:'text/vtt'}));await tick();if(generation!==playerGeneration)return;const track=video.querySelector('track');if(track){track.src=captionURL;track.track.mode='showing';}}
function reportPlayback(event:'first_frame'|'seek'|'stall',duration:number){if(selected)api('items/'+selected.id+'/telemetry','POST',{Event:event,DurationMs:duration,Mode:actualMode}).catch(()=>{});}
function playing(){const now=performance.now();if(frameRequestedAt){reportPlayback('first_frame',now-frameRequestedAt);frameRequestedAt=0;}if(waitingAt){reportPlayback('stall',now-waitingAt);waitingAt=0;}if(seekRequestedAt){reportPlayback('seek',now-seekRequestedAt);seekRequestedAt=0;}}
function clientCapabilities():ClientCapabilities {
 const v=document.createElement('video');const video_codecs:string[]=[],audio_codecs:string[]=[];
 for(const [codec,mime] of [['h264','video/mp4; codecs="avc1.42E01E"'],['hevc','video/mp4; codecs="hvc1.1.6.L93.B0"'],['av1','video/mp4; codecs="av01.0.08M.08"'],['vp9','video/webm; codecs="vp9"']])if(v.canPlayType(mime))video_codecs.push(codec);
 for(const [codec,mime] of [['aac','audio/mp4; codecs="mp4a.40.2"'],['mp3','audio/mpeg'],['opus','audio/webm; codecs="opus"']])if(v.canPlayType(mime))audio_codecs.push(codec);
 // Display HDR alone does not establish browser decoding support. Stay conservative.
 return {containers:['mp4','webm'],video_codecs,audio_codecs,video_pixel_formats:Object.fromEntries(video_codecs.map(codec=>[codec,['yuv420p','yuvj420p']])),hdr:false};
}
async function loadLists(){savedLists=await api('lists');}
async function selectList(id:number){activeList=await api('lists/'+id);section='lists';}
async function writeList(list:MediaList,entries:MediaListEntry[],cursor=list.cursor){
 if(list.entries.length!==list.entry_count)throw Error('Some saved titles are unavailable. Restore access before editing this list.');
 const result=await api('lists/'+list.id,'PUT',{Kind:list.kind,Name:list.name,ItemIDs:entries.map(e=>e.item.id),Revision:list.revision,Cursor:cursor});
 const updated={...list,revision:result.revision,cursor,entry_count:entries.length,entries:entries.map((e,position)=>({...e,position}))};
 if(activeList?.id===list.id)activeList=updated;if(playingQueue?.id===list.id)playingQueue=updated;await loadLists();return updated;
}
async function createList(){const result=await api('lists','POST',{Kind:newListKind,Name:newListName,ItemIDs:[]});newListName='';await loadLists();await selectList(result.id);}
async function addToList(){if(!selected||!targetListID)return;const list:MediaList=await api('lists/'+targetListID);if(list.kind==='collection'&&list.entries.some(e=>e.item.id===selected?.id)){notice='Already in this collection.';return;}await writeList(list,[...list.entries,{position:list.entries.length,item:selected}]);notice='Added to '+list.name+'.';}
async function moveEntry(index:number,direction:number){if(!activeList)return;const entries=[...activeList.entries],other=index+direction;if(other<0||other>=entries.length)return;[entries[index],entries[other]]=[entries[other],entries[index]];await writeList(activeList,entries,0);}
async function removeEntry(index:number){if(activeList)await writeList(activeList,activeList.entries.filter((_,i)=>i!==index),0);}
async function moveCursor(list:MediaList,cursor:number){
 const result=await api('lists/'+list.id+'/cursor','PUT',{Revision:list.revision,Cursor:cursor});
 const updated:MediaList=await api('lists/'+list.id);
 if(updated.revision!==result.revision)throw Error('Queue changed in another client. Resume the queue to continue.');
 if(activeList?.id===list.id)activeList=updated;
 await loadLists();return updated;
}
async function playEntry(list:MediaList,entry:MediaListEntry){const generation=++playerGeneration;const queue=list.kind==='queue'?await moveCursor(list,entry.position):null;if(generation!==playerGeneration)return;playingQueue=queue;await open(entry.item,list.kind==='queue');}
async function applyPlayback(){const generation=playerGeneration;await save();if(generation===playerGeneration)await start(selected?.position||0);}
async function ended(){if(transition)return;const generation=playerGeneration,playing=playingQueue;await save();if(generation!==playerGeneration||!playing||playingQueue!==playing)return;const queue:MediaList=await api('lists/'+playingQueue.id);if(generation!==playerGeneration||playingQueue!==playing)return;if(queue.revision!==playing.revision)throw Error('Queue changed in another client. Resume the queue to continue.');const next=queue.entries.find(e=>e.position>queue.cursor);const updated=await moveCursor(queue,next?.position??queue.cursor+1);if(generation!==playerGeneration||playingQueue!==playing)return;playingQueue=updated;if(next)await open(next.item,true);else notice='Queue finished.';}
function textSubtitle(codec:string){return ['subrip','ass','ssa','webvtt','mov_text','text'].includes(codec);}
async function browseShow(show:Series){section='library';seriesFilter=show.name;library=String(show.library_id);season='';offset=0;await load();}
async function mark(item:Item){await api('items/'+item.id+'/progress','PUT',{Position:item.position,Watched:!item.watched});item.watched=!item.watched;await load();}
function formatTime(seconds:number){return Math.floor(seconds/3600)+':'+String(Math.floor(seconds/60)%60).padStart(2,'0')+':'+String(Math.floor(seconds)%60).padStart(2,'0');}
</script>

<svelte:head><meta name="theme-color" content="#101820"/></svelte:head>
<header><a href="/">◈ OpenVideoLibrary</a><span>Your collection. Ready to play.</span>{#if logged}<button onclick={()=>guard(async()=>{await close();await api('logout','POST');logged=false;admin=false;scanning=false;name='';password='';notice='';})}>Sign out</button>{/if}</header>
<main>
{#if error}<p role="alert" class="error">{error}</p>{/if}
{#if notice}<p role="status" class="notice">{notice}</p>{/if}
{#if !ready}<p>Opening your library…</p>{:else if !logged}
<section class="login"><p class="eyebrow">YOUR OWN MEDIA SERVER</p><h1>{setup?'Make yourself at home.':'Welcome back.'}</h1><p>{setup?'Create your administrator account to get started.':'Sign in to your library.'}</p><form onsubmit={(e)=>{e.preventDefault();login();}}><label>Username<input bind:value={name} autocomplete="username" required/></label><label>Password<input bind:value={password} type="password" autocomplete={setup?'new-password':'current-password'} minlength={setup?12:1} required/></label><button class="primary">{setup?'Create library':'Sign in'}</button></form></section>
{:else}
<div class="heading"><div><p class="eyebrow">AT HOME, ANYWHERE</p><h1>{seriesFilter||'Your library'}</h1></div><div class="toolbar"><button class:active={section==='library'} onclick={()=>{section='library';}}>Movies & episodes</button><button class:active={section==='lists'} onclick={()=>{section='lists';guard(loadLists);}}>Collections & playlists</button><button class:active={section==='series'} onclick={()=>{section='series';}}>TV series</button></div></div>
<form class="filters" onsubmit={(e)=>{e.preventDefault();offset=0;guard(load);}}><input aria-label="Search" placeholder="Search movies and episodes…" bind:value={query}/><select aria-label="Library" bind:value={library} onchange={()=>{offset=0;seriesFilter='';season='';guard(load);}}><option value="">All libraries</option>{#each libraries as l}<option value={l.id}>{l.name}{l.offline?' · offline':''}</option>{/each}</select><label class="inline"><input type="checkbox" bind:checked={continuing} onchange={()=>{offset=0;guard(load);}}/>Continue watching</label><button>Search</button></form>
{#if seriesFilter}<div class="toolbar"><button onclick={()=>guard(async()=>{seriesFilter='';season='';offset=0;await load();})}>All titles</button><label>Season<input type="number" min="0" bind:value={season} onchange={()=>{offset=0;guard(load);}} placeholder="All"/></label></div>{/if}
{#if selected}<section class="player"><div class="heading"><h2>{selected.title}</h2><button onclick={()=>guard(close)}>Close player</button></div><video bind:this={video} controls autoplay onplaying={playing} onwaiting={()=>{if(!frameRequestedAt&&!transition&&!video.paused)waitingAt=performance.now();}} onseeked={()=>{if(seekRequestedAt&&actualMode==='direct'){reportPlayback('seek',performance.now()-seekRequestedAt);seekRequestedAt=0;}}} onloadedmetadata={()=>{if(selected&&actualMode==='direct')video.currentTime=seekTo;}} onpause={()=>{if(!transition)guard(save);}} onended={()=>guard(ended)} onerror={()=>{if(!transition)error='This media could not be played. Try transcoding.';}}><track kind="subtitles" label="Selected subtitles" srclang="en"/></video>
<div class="toolbar"><label>Playback<select aria-label="Playback" bind:value={mode}><option value="auto">Automatic</option><option value="direct">Direct play</option><option value="remux">Remux</option><option value="transcode">Transcode to SDR</option></select></label><label>Audio<select aria-label="Audio" bind:value={audio}>{#each info?.streams.filter(t=>t.codec_type==='audio')||[] as t,i}<option value={i}>{t.tags?.language||'Track '+(i+1)} · {t.codec_name}</option>{/each}</select></label><label>Subtitles<select aria-label="Subtitles" bind:value={subtitle}><option value="off">Off</option><option value="external">External sibling SRT</option>{#each info?.streams.filter(t=>t.codec_type==='subtitle')||[] as t,i}<option value={(textSubtitle(t.codec_name)?'text:':'burn:')+i}>{t.tags?.language||'Track '+(i+1)} · {t.codec_name}{textSubtitle(t.codec_name)?' (text)':' (burn in)'}</option>{#if textSubtitle(t.codec_name)}<option value={'burn:'+i}>{t.tags?.language||'Track '+(i+1)} · {t.codec_name} (burn in{['ass','ssa'].includes(t.codec_name)?', preserve styling':''})</option>{/if}{/each}</select></label><label class="inline"><input type="checkbox" bind:checked={adaptive}/>Adaptive renditions (transcoding)</label><button disabled={busy} onclick={()=>guard(applyPlayback)}>{busy?'Preparing…':'Apply playback settings'}</button><button onclick={()=>guard(async()=>{if(selected)await mark(selected);})}>{selected.watched?'Mark unwatched':'Mark watched'}</button></div>
{#if duration>0}<label>Jump to {formatTime(Number(seekTo))} / {formatTime(duration)}<input type="range" min="0" max={duration} step="1" bind:value={seekTo} onchange={()=>guard(async()=>{seekRequestedAt=performance.now();if(actualMode==='direct')video.currentTime=Number(seekTo);else await start(Number(seekTo));})}/></label>{/if}<p>{selected.overview}</p><p role="status">{playbackReason}</p><div class="toolbar"><label>Add to list<select aria-label="Add to list" bind:value={targetListID}><option value="">Choose a collection, playlist or queue</option>{#each savedLists as l}<option value={l.id}>{l.name} · {l.kind}</option>{/each}</select></label><button disabled={!targetListID} onclick={()=>guard(addToList)}>Add title</button></div>
{#if admin}<details><summary>Edit title and metadata</summary><form onsubmit={(e)=>{e.preventDefault();guard(async()=>{if(!selected)return;await api('items/'+selected.id,'PATCH',{Title:editTitle,Overview:editOverview});selected.title=editTitle;selected.overview=editOverview;await load();notice='Metadata saved.';});}}><label>Title<input bind:value={editTitle} required/></label><label>Overview<textarea bind:value={editOverview}></textarea></label><button>Save metadata</button></form><form onsubmit={(e)=>{e.preventDefault();guard(async()=>{if(!selected)return;const result=await api('items/'+selected.id+'/match','POST',{ID:Number(tmdbID),Kind:tmdbKind});selected.title=editTitle=result.title;selected.overview=editOverview=result.overview;await load();notice='TMDB metadata imported.';});}}><label>TMDB ID<input bind:value={tmdbID} type="number" min="1" required/></label><select bind:value={tmdbKind} aria-label="TMDB media type"><option value="movie">Movie</option><option value="tv">TV series</option></select><button>Match TMDB title</button></form></details>{/if}
</section>{/if}
{#if section==='lists'}<section><h2>Collections, playlists & queues</h2><form class="toolbar" onsubmit={(e)=>{e.preventDefault();guard(createList);}}><label>List name<input bind:value={newListName} maxlength="200" required/></label><label>List type<select aria-label="List type" bind:value={newListKind}><option value="collection">Collection</option><option value="playlist">Playlist</option><option value="queue">Playback queue</option></select></label><button>Create list</button></form><div class="toolbar">{#each savedLists as l}<button onclick={()=>guard(()=>selectList(l.id))}>{l.name} · {l.kind}</button>{/each}</div>
{#if activeList}<h3>{activeList.name}</h3>{#if activeList.kind==='queue'}<button disabled={!activeList.entries.some(e=>e.position>=activeList!.cursor)} onclick={()=>guard(async()=>{if(!activeList)return;const entry=activeList.entries.find(e=>e.position>=activeList!.cursor);if(entry)await playEntry(activeList,entry);})}>Resume queue</button>{/if}<p>Only currently accessible titles are shown.</p><button onclick={()=>guard(async()=>{if(!activeList)return;const list=activeList;await api('lists/'+list.id,'DELETE');activeList=null;if(playingQueue?.id===list.id)playingQueue=null;await loadLists();})}>Delete list</button>{#if activeList.entries.length===0}<p>Open a title from your library and use “Add to list”.</p>{/if}<ol>{#each activeList.entries as entry,index}<li class="toolbar"><span>{entry.item.title}</span><button onclick={()=>guard(async()=>{if(activeList)await playEntry(activeList,entry);})}>Play{activeList.kind==='queue'?' queue from here':''}</button><button aria-label={'Move '+entry.item.title+' up'} disabled={index===0} onclick={()=>guard(()=>moveEntry(index,-1))}>↑</button><button aria-label={'Move '+entry.item.title+' down'} disabled={index===activeList.entries.length-1} onclick={()=>guard(()=>moveEntry(index,1))}>↓</button><button onclick={()=>guard(()=>removeEntry(index))}>Remove</button></li>{/each}</ol>{/if}</section>
{:else if section==='series'}<div class="grid">{#each shows.filter(s=>!library||String(s.library_id)===library).filter(s=>s.name.toLowerCase().includes(query.toLowerCase())) as show}<button class="show" onclick={()=>guard(()=>browseShow(show))}><span class="eyebrow">TV SERIES</span><h2>{show.name}</h2><p>{show.seasons} seasons · {show.episodes} episodes · {show.watched} watched</p></button>{/each}</div>{:else}<div class="grid">{#each items as item}<button class="card" onclick={()=>open(item)}><div class="poster" style:background-image={"linear-gradient(0deg,rgba(0,0,0,.6),transparent),url(/api/v1/items/"+item.id+"/artwork)"}><span>{item.series?'TV':'FILM'}</span><b>▶</b>{#if item.watched}<span>WATCHED</span>{/if}</div><h2>{item.title}</h2>{#if item.series}<p>{item.series} · S{item.season} E{item.episode}</p>{:else}<p>{item.position>0?'Continue watching':'Ready to play'}</p>{/if}</button>{/each}</div>
{#if items.length===0}<section class="empty"><h2>{libraries.length?'No titles found.':'Bring your collection.'}</h2><p>{libraries.length?'Try another search, or scan your libraries.':'Add a media folder below, then start a scan.'}</p></section>{/if}
<nav><button disabled={offset===0} onclick={()=>{offset=Math.max(0,offset-100);guard(load);}}>Previous</button><button disabled={items.length<100} onclick={()=>{offset+=100;guard(load);}}>Next</button></nav>{/if}
{#if admin}<details><summary>Manage libraries</summary><form onsubmit={(e)=>{e.preventDefault();guard(async()=>{await api('libraries','POST',{Name:libraryName,Path:path,Kind:kind});libraryName='';path='';await refresh();});}}><label>Name<input bind:value={libraryName} required/></label><label>Server folder<input bind:value={path} placeholder="/media/movies" required/></label><select bind:value={kind} aria-label="Media type"><option value="movie">Movies</option><option value="tv">TV shows</option></select><button>Add library</button></form><button disabled={scanning} onclick={()=>guard(async()=>{await api('scan','POST');scanning=true;notice='Scan started.';})}>{scanning?'Scanning…':'Scan libraries'}</button><button onclick={()=>guard(refresh)}>Refresh library</button></details>
<details><summary>Manage accounts</summary><form onsubmit={(e)=>{e.preventDefault();guard(async()=>{await api('users','POST',{Name:newName,Password:newPassword,Libraries:grants});newName='';newPassword='';grants=[];users=await api('users');notice='Viewer created.';});}}><label>Username<input bind:value={newName} autocomplete="off" required/></label><label>Initial password<input type="password" bind:value={newPassword} minlength="12" required autocomplete="new-password"/></label><fieldset><legend>Allowed libraries</legend>{#each libraries as l}<label class="inline"><input type="checkbox" bind:group={grants} value={l.id}/>{l.name}</label>{/each}</fieldset><button>Create viewer</button></form><div class="toolbar">{#each users as u}<button onclick={()=>{editUser={...u,libraries:[...u.libraries]};resetPassword='';}}>{u.name}{u.admin?' (administrator)':''}</button>{/each}</div>
{#if editUser}<h3>{editUser.name}</h3>{#if !editUser.admin}<form onsubmit={(e)=>{e.preventDefault();guard(async()=>{if(!editUser)return;await api('users/'+editUser.id+'/access','PUT',{Libraries:editUser.libraries});users=await api('users');notice='Library access updated.';});}}><fieldset><legend>Allowed libraries</legend>{#each libraries as l}<label class="inline"><input type="checkbox" bind:group={editUser.libraries} value={l.id}/>{l.name}</label>{/each}</fieldset><button>Save access</button></form>{/if}<form onsubmit={(e)=>{e.preventDefault();guard(async()=>{if(!editUser)return;await api('users/'+editUser.id+'/password','POST',{Password:resetPassword});resetPassword='';notice='Password changed; existing sessions revoked.';});}}><label>New password<input type="password" bind:value={resetPassword} minlength="12" required autocomplete="new-password"/></label><button>Reset password</button></form>{/if}
</details>{/if}
{/if}
</main>
<style>
:global(*){box-sizing:border-box}:global(body){margin:0;background:#101820;color:#eaf0f5;font:15px system-ui,sans-serif}:global(button),:global(input),:global(select),:global(textarea){font:inherit;border:1px solid #364651;border-radius:8px;background:#1c2933;color:inherit;padding:12px}:global(button){cursor:pointer}:global(button:hover){border-color:#6de0bd}:global(button:disabled){opacity:.4;cursor:default}:global(input:focus){outline:2px solid #6de0bd}:global(a){color:inherit;text-decoration:none}header{display:flex;gap:30px;align-items:center;padding:24px 5vw;border-bottom:1px solid #293741}header a{font-weight:750;font-size:22px}header span{flex:1;color:#91a6b4}main{max-width:1500px;margin:auto;padding:40px 5vw}h1{font-size:clamp(32px,5vw,52px);letter-spacing:-2px;margin:8px 0 28px}.eyebrow{font-size:11px;font-weight:700;letter-spacing:3px;color:#6de0bd}.heading{display:flex;justify-content:space-between;align-items:center;gap:20px}.filters{display:flex;gap:12px;flex-wrap:wrap;margin:20px 0 32px}.filters>input{flex:1;min-width:220px}.inline{display:flex;align-items:center;gap:8px}.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(180px,1fr));gap:24px}.card{text-align:left;padding:0;background:none;border:none}.poster{aspect-ratio:2/3;background:linear-gradient(140deg,#284b52,#18232f 70%);border-radius:12px;display:flex;flex-direction:column;justify-content:space-between;padding:20px;color:#7fbaa9}.poster{background-size:cover;background-position:center}.poster b{align-self:center;font-size:35px;color:#b1d8cd}.poster span{font-size:11px;letter-spacing:2px}.card h2{font-size:15px;margin-bottom:5px}.card p{font-size:12px;color:#91a6b4;margin-top:0}.login{max-width:440px;margin:8vh auto}label{display:block;margin:12px 0}label input{display:block;width:100%;margin-top:8px}.inline input{width:auto;margin:0}.primary{background:#6de0bd;color:#10251d;width:100%;font-weight:700;margin-top:16px}.error{background:#493031;padding:16px;border-radius:8px}.player{padding:20px;background:#17232c;margin-bottom:30px;border-radius:12px}video{width:100%;max-height:70vh;background:#000}details{border-top:1px solid #364651;padding-top:24px;margin-top:40px}details form{display:flex;flex-wrap:wrap;align-items:end;gap:12px;margin-bottom:20px}nav{display:flex;gap:12px;margin-top:30px}.empty{padding:60px 0;color:#91a6b4}@media(max-width:600px){header span{display:none}header{justify-content:space-between}.heading{align-items:start;flex-direction:column}.grid{grid-template-columns:repeat(2,1fr)}main{padding-top:24px}}
.toolbar{display:flex;flex-wrap:wrap;align-items:center;gap:12px}.toolbar label{font-size:13px}.toolbar select{display:block;margin-top:8px}.notice{padding:12px;border-left:3px solid #6de0bd;background:#19302c}.show{padding:24px;text-align:left}.show h2{font-size:20px}.show p{font-size:13px;color:#91a6b4}.active{border-color:#6de0bd}textarea{display:block;min-height:100px;width:100%;margin-top:8px}fieldset{border:1px solid #364651;border-radius:8px}input[type=range]{width:100%}.player details form label{flex:1;min-width:200px}</style>
