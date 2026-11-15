#!/usr/bin/env python3
"""Render checked-in measurements; never substitute missing data with estimates."""
import json,pathlib,statistics
root=pathlib.Path(__file__).resolve().parents[1]
folder=root/'docs/benchmarks'
lines=['# N100 reference measurements','', 'Measurements use the Intel N100 (four cores, 16 GB RAM), local SQLite/application storage and read-only SMB media. Existing household services remained running. These are prototype and playback checks, not complete release acceptance or a final backend selection.','', '## Matched SQLite prototypes','', 'Both candidates use SQLite 3.54.1, identical fresh 10k/100k fixtures, case-insensitive indexed prefix queries, one database connection and 100-row JSON responses. The HTTP load client runs off-host through one persistent SSH tunnel. Candidate order is randomized. Each case has five repetitions of 2,000 requests, after readiness and ten warm-up requests. Caches are not evicted. The query mix is browsing and a prefix near the end of the library.','', 'A measured query-plan bottleneck was fixed in both prototypes: matching the index and ordering collation enables a covering range search rather than a full index scan. The pre-fix baseline and optimized matrix are separate datasets. Neither comparison establishes a language-only effect.','', 'The mixed workload adds incremental scans, 1 MiB HTTP range reads and one real-time VAAPI HEVC 1080p → H.264 720p / 3 Mbps transcode from the NAS. Core and FFmpeg CPU/RSS are recorded separately.','']
loaded={}
for filename,label in [('n100-matched-optimized.json','Pure-Go SQLite versus Rust'),('n100-native-sqlite.json','Native SQLite Go versus Rust')]:
 path=folder/filename
 if not path.exists():continue
 p=json.loads(path.read_text());rows=p['results'];errors=sum(r['errors']+r['child_errors']+r['background']['errors'] for r in rows);loaded[filename]=(p,rows,errors)
 lines += ['### '+label,'',f"[{filename}]({filename}): {len(rows)} trials, {sum(r['requests'] for r in rows):,} requests, {errors} errors. Complete: {p.get('complete',False)}.",'', 'The following values are medians across the browse and search repetitions for 100,000 items. The p95 range shows the minimum–maximum per-trial p95, not a confidence interval.','', '| Mode | Candidate | Concurrent requests | p95 ms (range) | p99 ms | Successful RPS | Core peak RSS MiB | Core CPU µs/request |','| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |']
 groups={}
 for r in rows:
  if r['items']==100000 and r['concurrency']>=100:groups.setdefault((r['mode'],r['candidate'],r['concurrency']),[]).append(r)
 for (mode,candidate,concurrency),rs in sorted(groups.items()):
  # Trials with no successful request carry null percentiles; show n/a rather than crash.
  med=lambda key:statistics.median(r[key] for r in rs)
  values=lambda key:[r[key] for r in rs if r[key] is not None]
  fmt=lambda v:'n/a' if v is None else f'{v:.2f}'
  p95=values('p95_ms');p99=values('p99_ms');per=[r['core_cpu_seconds']/(r['requests']-r['errors'])*1e6 for r in rs if r['requests']>r['errors']]
  p95_cell=f"{fmt(statistics.median(p95))} ({min(p95):.2f}–{max(p95):.2f})" if p95 else 'n/a'
  cpu=f'{statistics.median(per):.1f}' if per else 'n/a'
  lines.append(f"| {mode} | {candidate} | {concurrency} | {p95_cell} | {fmt(statistics.median(p99)) if p99 else 'n/a'} | {med('successful_rps'):.0f} | {med('core_peak_rss_bytes')/2**20:.1f} | {cpu} |")
 lines+=['']
lines += ['## Application playback','', 'Intel VAAPI hardware decode/scale/encode passed using FFmpeg 6.1.1. QSV initialization failed with the installed runtime. AMD and NVIDIA remain unverified. The existing household media services were not changed.','', 'The application scanned a small real NAS fixture and played a 1920×1080 HEVC BT.709 SDR source at a 60-second offset. Each mode ran three times with a loopback FFmpeg client, timing session creation through the first decoded frame. All nine checks passed.','', '| Mode | First decoded frame range |','| --- | ---: |']
p=json.loads((folder/'n100-playback.json').read_text())
for mode,adaptive,label in [('direct',False,'Direct'),('transcode',False,'Single VAAPI HLS'),('transcode',True,'Adaptive VAAPI HLS (three renditions)')]:
 rs=[r['first_decoded_frame_ms']/1000 for r in p['results'] if r['mode']==mode and r['adaptive']==adaptive];lines.append(f'| {label} | {min(rs):.2f}–{max(rs):.2f} s |')
lines += ['', '[Playback results](n100-playback.json), [hardware smoke checks](n100-hardware-smoke.json), [environment](n100-environment.json).','', 'These timings include the FFmpeg client’s buffering. They are not browser/LAN startup measurements. HLS startup needs further investigation before making a user-visible startup claim.','', '## Interpretation and outstanding acceptance','']
# Dataset conclusions appear only when their measurements were loaded and support them.
def clean(name,candidates):
 if name not in loaded:return None
 p,rows,errors=loaded[name]
 return rows if p.get('complete') is True and errors==0 and {r['candidate'] for r in rows}==candidates else None
def supports_target(name):
 rows=clean(name,{'go','rust'})
 if rows is None or len(rows)!=400:return False
 # The claim covers both candidates, browse and search, at every concurrency through 100, in every mode and size.
 cases={(r['mode'],r['items'],r['query']) for r in rows};seen={(r['candidate'],r['mode'],r['items'],r['query'],r['concurrency']) for r in rows}
 return ({q=='' for _,_,q in cases}=={True,False} and all((c,*case,n) in seen for c in ('go','rust') for case in cases for n in (1,10,25,50,100))
         and all(r['p95_ms'] is not None and r['p95_ms']<150 for r in rows))
def supports_native(name):
 rows=clean(name,{'go-cgo','rust'})
 if rows is None or any(r['p95_ms'] is None for r in rows):return False
 # Every concurrency-200 trial pairs with the other candidate's trial of the same mode, query and repetition.
 arm=lambda c:{(r['mode'],r['query'],r['repetition']):r for r in rows if r['candidate']==c and r['concurrency']==200}
 go,rust=arm('go-cgo'),arm('rust')
 lower=bool(go) and go.keys()==rust.keys() and all(rust[k][m]<go[k][m] for k in go for m in ('p95_ms','core_cpu_seconds','core_peak_rss_bytes'))
 return (lower and any(r['candidate']=='go-cgo' and r['mode']=='idle' and r['p95_ms']>150 for r in rows)
         and all(r['p95_ms']<150 for r in rows if r['candidate']=='rust'))
if supports_target('n100-matched-optimized.json'):lines += ['- The 400-trial optimized matrix meets the 150 ms browse/search p95 target at every tested concurrency, through 100 concurrent requests, for both candidates. This is a bounded workload result, not a sustainable capacity limit.']
if supports_native('n100-native-sqlite.json'):lines += ['- In the native SQLite comparison, Rust had lower paired p95 latency, CPU cost and RSS at concurrency 200. Go exceeded 150 ms p95 in some idle trials; Rust stayed below it in all tested trials. Trial medians alone conceal this variation.', '- The native Go driver removes the Go-translated SQLite implementation as a confound. Driver bindings, scheduling, JSON encoding and build options still differ; results compare stacks rather than isolate language cost.']
lines += ['- The load generator, SSH tunnel, short trial duration, warm caches, synthetic ASCII metadata and shared household services limit inference. Establish longer steady-state saturation and repeat on separate days before selecting a backend.', '- Cold-cache browsing, full-library scan throughput, browser playback startup, seeking/stalls, direct/remux/transcode concurrency, upload limits, real Nginx deployment and a pinned isolated Jellyfin comparison remain pending.', '- No Jellyfin speedup, maximum stream capacity, AMD/NVIDIA support validation or production Rust feature parity is claimed.', '', 'Reproduce with `scripts/remote_worker.py` staged in the private benchmark workspace, then `scripts/remote_bench.py --host USER@HOST --key KEY --workspace /ABSOLUTE/WORKSPACE --requests 2000 --repetitions 5 --concurrency 1 10 25 50 100 --modes idle mixed-vaapi --output RESULT.json`. The harness in this repository runs only the Rust candidate. The Go prototypes are not included, so the recorded Go rows cannot be rerun from it. The host and private media paths are deliberately absent from published reports.','']
(folder/'N100.md').write_text('\n'.join(lines))
