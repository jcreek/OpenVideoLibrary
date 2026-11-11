# Benchmark protocol and evidence

Rust is the product backend by decision, not by a demonstrated language win.
The authoritative performance target remains the Intel N100 with read-only SMB
media, local application/transcode storage and a separate LAN load client.

## Production API smoke benchmark

Build the application first, then run:

```sh
cargo build --locked --release --workspace
python3 scripts/bench_application.py --items 10000 100000 \
  --requests 10000 --repetitions 5 --concurrency 1 10 25 50 100 \
  --output results/rust-production.json
```

The script creates a fresh disposable data directory/server for each trial,
creates an account through the API, seeds the migrated production schema, warms
the application, randomizes trial order and validates all returned IDs/order.
It reports successful RPS, errors, p50/p95/p99, core CPU/RSS, binary hash and
repeated-run p95 median/standard deviation. The seeded media are synthetic names;
this does not benchmark media I/O. Credentials and session tokens are not recorded.

`docs/benchmarks/rust-production-local.json` records a short 10k-item shared-macOS
smoke run: 12 trials, 3,600 authenticated requests, concurrency 1/10, three repeats,
zero errors. This is warm loopback browse/search, without scanning or playback.
It does not establish N100 capacity, cold-cache behavior or a Jellyfin speedup.
The Python load driver and local network stack can set the ceiling.

## Reference acceptance

- Record CPU/model/RAM, OS/kernel, compiler flags, source/binary revision, SQLite
  engine/driver versions/options, GPU runtime/driver, FFmpeg build, NAS/SMB mount
  settings and measured network throughput.
- Use identical read-only media and metadata fixtures, permissions, output quality,
  scan/background work, library scale and changed-file ratios across comparisons.
- Measure browse/search separately at 10k/100k and realistic library shapes. Report
  warm, process-cold and genuinely OS-cold trials separately. Never evict shared
  host caches; reserve a dedicated test host for deliberate cold-cache acceptance.
- Use a separate load host and a proven saturation client. Repeat five or more
  times and report per-run results, p50/p95/p99, successful RPS, errors, CPU/RSS,
  network throughput and uncertainty. Keep server and FFmpeg resource use separate.
- Run idle and simultaneous scanning/direct/remux/transcode workloads. Record
  codec/profile, resolution, bitrate, audio, subtitles, HDR path, client and mode.
- Measure first decoded frame on LAN, seeking, stalls and sustained simultaneous
  playback separately. Playlist readiness and client-reported timing logs are not
  independently verified decoded-frame measurements.
- Initial gates remain browse/search p95 <150 ms and LAN direct first frame <2 s.
  Determine sustainable concurrency only for the stated workload. Establish
  remux/transcode/HDR/subtitle budgets from measurements before release acceptance.
- Profile bottlenecks before tuning. Repeat equivalent workloads after changes;
  report overlapping uncertainty as inconclusive. Do not sacrifice quality or
  work equivalence to claim a performance gain.

## Jellyfin baseline

`docs/jellyfin-baseline.json` records direct verification on 9 November 2026 of
[the v12.4 release](https://github.com/jellyfin/jellyfin/releases/tag/v12.4)
and the amd64/arm64 multi-architecture manifest. `bench/jellyfin.compose.yaml`
pins `jellyfin/jellyfin:12.4` to its verified immutable digest.

Run it only with isolated config/cache and sufficient Docker space. Never mount
an existing Jellyfin config. Complete scans and configure equivalent media,
metadata, clients and GPU access. The pin is verified; the comparison has not
been run in this migration. Full cold-cache/mixed-load/playback orchestration
remains acceptance work; the synthetic API runner is not a substitute.

## Historical prototype evidence

The original Go/Rust N100 results and environment reports remain in
`docs/benchmarks/` and are described in `docs/benchmarks/N100.md`. They predate
production Rust and retain their original candidate names. No Go implementation
or Go build is included in this repository.

The separate Rust microbenchmark remains in `bench/rust`, outside the production
workspace. Its `/items`, range `/media` and `/scan` endpoints have no production
authentication/playback. The retained `bench_suite.py`, `remote_bench.py` and
`reference-bench.sh` now run Rust only and must not be mistaken for a current
cross-language comparison. Prior response hashes refer to the earlier protocol.
Do not silently rewrite or relabel old measurement files.

## Production query profiling

`cargo run --locked --release --example profile_queries -- 100000` runs the actual
production SQL builder against disposable in-memory data with bundled SQLite. It
records seven per-query timings, query plans and result IDs for browse, broad,
selective and no-match searches. This isolates SQL behavior; it is not HTTP/LAN,
SMB or release acceptance. The initial macOS ARM64 profile is retained separately
in `benchmarks/rust-sql-local-initial.json`. Unfiltered browse now uses the existing
title index to avoid sorting the whole library. Unfiltered broad FTS searches
use the same index after a census capped at 2,049 matches; sparse searches and
filtered/progress queries retain their selective plans. The FTS expression and
ordering are unchanged. A correlated-FTS experiment was rejected after it
substantially regressed all measured search shapes. N100 end-to-end measurements
must be repeated after these changes.

## N100 production diagnostics and bounded search cache

`benchmarks/rust-production-n100-indexed.json` records five repeats at 10k/100k,
1,000 requests per trial and concurrency 1/10/50/100: 80,000 authenticated requests,
10,148 errors. Unlike the initial contaminated diagnostic, no acceptance builds,
tests or harness FFmpeg overlapped the measured trials. The household host and
warm loopback/synthetic limitations remain. At 100k, search p95 median was
373.54 ms (run standard deviation 65.27 ms) at concurrency 10 and 1,199.61 ms
(276.53 ms) at concurrency 50. These miss the initial latency gate; concurrency
100 errors prevent a capacity claim. Reports include per-trial errors, CPU/RSS,
source/binary hashes and runtime versions.

The SQL prototype in `benchmarks/rust-sql-cache-prototype-local.json` motivated
caching full membership for repeated broad searches, retaining live permission
and progress checks. Sparse queries retain their selective plan. The production
cache has explicit term/row/page limits and invalidation, documented in ADR 001.
The profiler records planning/fill and execution separately so a warm lookup
cannot hide fill cost. `benchmarks/rust-production-n100-cached.json` records the equivalent 80-trial,
80,000-request workload after the cache. At 100k, broad-search p95 median is
7.36 ms (run standard deviation 0.13 ms) at concurrency 10 and 36.92 ms
(2.45 ms) at concurrency 50. Worst per-run p95 at concurrency 50 is 42.10 ms.
All browse/search trials through concurrency 50 have zero errors at both scales.
Concurrency 100 has 7,523 HTTP 503 errors across workloads; the harness correctly
exits nonzero. This is overload evidence, not sustainable capacity acceptance.
Peak sampled core RSS reaches 14.05 MiB across these trials, with no harness
FFmpeg processes. Full per-trial CPU/RSS and binary/source identities are retained.

Both indexed and cached trials use the same seeded schema, media names, response
ID/order checks, randomized order, request counts and concurrency on the same
shared N100. They were sequential rather than interleaved; household/background
variation and the loopback Python client remain limitations. The measured broad
search improvement exceeds repeated-run variation, but does not establish LAN,
OS-cold, mixed scanning/streaming, real-library or Jellyfin-relative performance.
The first measured request in each search trial includes the cache fill; SQL profiling
separately records the fill because aggregate p95 can hide that single request.

`benchmarks/rust-sql-cached-production-n100.json` separately records the actual
production SQL on N100: the first repeated broad query, including cache fill,
takes 21.55 ms, followed by six total query timings of 0.64–0.66 ms. Initial
term classification costs 3.03 ms before that repeat. This seven-sample SQL
profile has disposable in-memory data and does not include HTTP/auth/LAN latency;
it complements rather than replaces the five-repeat application measurement.

## Bitmap seek diagnostic (2026-11-10)

Run `python3 scripts/profile_bitmap_seek.py --ffmpeg /path/to/ffmpeg --output /path/to/report.json`
on the selected host. A generated 64-second black 1280x720/12fps H.264/AAC+PGS
source with one-second keyframes is sought to 60 seconds, encoding two seconds
at equivalent quality. Correct sequential video/subtitle preroll is compared
with indexed video seeking plus subtitle-only preroll. Both check visible and
cleared cues. Three randomized warm-file repetitions have zero errors.
This FFmpeg-only diagnosis is not Rust/API, LAN, SMB, cold-cache, saturation,
Jellyfin or production acceptance. MPEG-TS sequential fallback is not profiled.

| Host / strategy | Wall median (min–max; sample σ), seconds | FFmpeg CPU median (min–max; sample σ), seconds |
| --- | --- | --- |
| Apple M4 / decode_video_preroll | 0.4473 (0.4446–0.4473; 0.0015) | 0.9675 (0.9637–0.9685; 0.0025) |
| Apple M4 / subtitle_only_preroll | 0.0665 (0.0659–0.0670; 0.0005) | 0.1174 (0.1172–0.1182; 0.0005) |
| N100 / decode_video_preroll | 1.2666 (1.2547–1.3307; 0.0409) | 2.6079 (2.6074–2.7088; 0.0584) |
| N100 / subtitle_only_preroll | 0.2644 (0.2558–0.2860; 0.0155) | 0.4316 (0.4075–0.4463; 0.0196) |

Reports `benchmarks/bitmap-seek-local.json` and `benchmarks/bitmap-seek-n100.json`
retain hardware/RAM, FFmpeg version/binary identity, fixture/script hashes,
quality controls and individual trials. Child peak RSS includes fixture
generation and quality runs; it is not per-strategy RAM. Fixture storage is
capped at 8 MiB; output and duration are bounded, with kill/reap on failure.
This supports avoiding video preroll for the indexed synthetic PGS case.
Long real-media/SMB seek latency and I/O remain unaccepted.

## N100 first authenticated query after process restart

Build the release binary, then run on the N100:

```sh
cargo build --locked --release -p ovl-server
python3 scripts/bench_application.py --process-cold --items 10000 100000 \
  --requests 100 --repetitions 5 --concurrency 1 --output process-cold.json
```

Each trial creates and seeds a disposable database, logs in, stops the server,
and starts a fresh process against the same data. After readiness polling, the
first authenticated query is timed and checked for correct result IDs and order.
It also supplies the single prior query for the follow-up load; the canonical
response hash is collected afterward. The first-query stats are separate from
follow-up p95. OS filesystem caches are left intact.

[Recorded N100 run](benchmarks/rust-production-n100-process-cold.json): five
repeats per size/workload, 20 first queries and 2,000 follow-up requests, zero
errors. The release binary and harness identities are in the report. Linux
x86_64, Intel N100 (four logical CPUs), approximately 15.4 GiB RAM; Rust server
0.1.0, SQLite 3.54.1. No harness FFmpeg workloads ran.

| Items | Workload | First query median (min–max; sample σ), ms | Follow-up p95 median, ms |
| --- | --- | --- | --- |
| 10k | Browse | 1.78 (1.26–2.26; 0.36) | 1.65 |
| 10k | Search | 6.04 (3.86–7.05; 1.55) | 1.66 |
| 100k | Browse | 1.94 (1.71–2.37; 0.24) | 1.70 |
| 100k | Search | 25.21 (24.15–30.05; 2.79) | 1.51 |

This measures a fresh application process with persisted authentication and
a recently seeded database, on a shared host over loopback. It does not
include startup time, OS-cold disk reads, LAN/SMB, saturation, scanning or a
Jellyfin comparison. The first request and the later cache fill are distinct;
the short follow-up p95 can hide the single fill.

## Timed loopback saturation on N100

Use the release server and run this on the N100 with no harness builds, scans
or playback running alongside it:

```sh
cargo build --locked --release -p ovl-server
python3 scripts/bench_application.py --items 100000 --seconds 20 \
  --requests 200000 --repetitions 5 --concurrency 50 100 \
  --output sustained-20s.json
```

Timed mode keeps a fixed number of closed-loop clients offering the same
authenticated and result-checked requests as the count-based mode. It stops
starting new requests at the deadline, then drains in-flight requests.
Throughput uses full elapsed time including drain. Percentiles cover successful
requests only; judge them alongside error counts. The request cap bounds memory
and is reported per trial and in the summary. A capped trial or any request
error exits nonzero even when the report is complete.

[Recorded SSH run](benchmarks/rust-production-n100-sustained-20s.json):
20 trials, 1,020,782 authenticated requests and no cap hits. The same N100
release binary as the process-cold run was used. All 374,886 errors were HTTP
503 at concurrency 100; the harness exited 1 after writing the complete report.

| Clients | Workload | Requests | HTTP 503 | Successful RPS median (sample σ) | Success p95 median (sample σ), ms |
| --- | --- | --- | --- | --- | --- |
| 50 | Browse | 202,141 | 0 | 2,028 (23) | 26.09 (0.42) |
| 50 | Search | 136,219 | 0 | 1,357 (30) | 39.24 (4.61) |
| 100 | Browse | 287,068 | 99,694 | 1,890 (67) | 61.98 (5.98) |
| 100 | Search | 395,354 | 275,192 | 1,185 (21) | 77.54 (1.16) |

At 50 clients the worst per-run p95 was 26.81 ms for browse and 49.02 ms
for search. At 100 clients, successful throughput fell and error rates rose
to 34.7% and 69.6% respectively. The successful-request p95 does not qualify
either overloaded case. The artifact records per-run core CPU and sampled
RSS (maximum 16.42 MiB), source, binary and harness hashes and runtime versions.
No harness FFmpeg processes ran. These are 20-second windows with a Python
client on the shared server, not a long soak, an independently driven LAN
capacity test, a mixed-media workload or a Jellyfin comparison.

## LAN browser direct startup and buffered seek

Stage `scripts/lan_playback_worker.py` into the source checkout on the N100
and build its release binary with `cargo build --locked --release -p ovl-server`.
On a separate LAN client with the web dependencies and Playwright Chromium
installed, run:

```sh
node web/bench-lan-playback.mjs --host USER@N100 --key /path/to/ssh-key \
  --workspace /path/to/remote/source --listen N100_PRIVATE_IPV4 \
  --output lan-direct-browser.json
```

SSH controls setup and shutdown. The browser uses direct HTTP to the N100
private address and verifies the media response peer IP and port, HTTP 206,
and the fixture size in Content-Range. The worker creates a disposable
database and 61,980-byte, six-second H.264/AAC MP4 on local storage. Setup uses
loopback, revokes the administrator session, and grants the browser only a
disposable viewer with access to the fixture. This plaintext test listener is
temporary and is intended only for a trusted LAN. It never uses the NAS.

Each trial uses a fresh browser context and a minimal page served by the N100.
Startup timing begins before the authenticated playback POST and ends at the
first browser video frame callback. The test samples the decoded pixels,
checks the frame dimensions/time and repeats after a seek to three seconds.
The first frame is red; the seek frame is blue. This measures playback after
the page has loaded, without the production Svelte UI or login in the timer.

[Recorded run](benchmarks/rust-n100-lan-direct-browser.json): five repetitions
from a separate Apple M4 client using Chromium 158.0.8127.18. All five startup
samples passed the two-second gate for this fixture. Median was **16.30 ms**
(range 11.30–134.00 ms; sample σ 53.14 ms), retaining the slower first sample.
Buffered seek median was **10.30 ms** (7.90–16.90 ms; σ 3.48 ms); the target
was already buffered in all five trials. No failures occurred in this run
and the N100 worker recorded a clean shutdown. The report records hardware,
runtime, source, binary, client and worker identities. No server FFmpeg jobs
ran during direct playback.

This is a LAN/local-storage diagnostic. It does not certify SMB, unbuffered
seeking, long titles, stalls, mobile devices, other browsers, sustained
concurrency, or a Jellyfin comparison. LAN bandwidth was not measured.

### LAN seek outside the browser buffer

Use the same command with `--unbuffered` (the N100 worker also needs FFprobe).
It generates a 60-second, 75,532,229-byte MP4 with CBR padding and seeks to
55 seconds. This makes a far seek practical without relying on traffic
shaping or writing to SMB. The worker rejects truncated/oversized fixtures
and independently locates the 55-second video keyframe with FFprobe.

The browser rejects a target already in its buffered ranges. It requires a
new HTTP 206 from the N100 peer, with a matching requested offset and a returned
range spanning the target keyframe's start position. It verifies that the
request starts after the seek and the response begins before the decoded blue
frame callback (with 1 ms clock tolerance). This establishes a network-backed
seek, without claiming that a particular response delivered every decoded byte.

[Recorded run](benchmarks/rust-n100-lan-unbuffered-seek.json): five trials,
zero errors, identical N100 binary and client browser version to the buffered
diagnostic. Every trial sought to 55 seconds with the target initially
unbuffered. The response started at byte 69,206,016; the independently probed
keyframe starts at byte 69,238,623.

| Metric | Median, ms | Min–max, ms | Sample σ, ms |
| --- | --- | --- | --- |
| First decoded frame | 14.80 | 12.10–150.30 | 60.71 |
| Unbuffered seek to decoded frame | 10.10 | 7.60–11.00 | 1.51 |

The first sample is retained. This is a synthetic local-storage LAN diagnostic:
padding stresses byte-range delivery with a simple picture, not demanding
video decoding. It does not qualify SMB, long-title stalls, real media,
sustained playback concurrency or a Jellyfin comparison.

## Concurrent browser playback windows over LAN

Stage the worker as above and copy the locked web dependency
`web/node_modules/hls.js/dist/hls.min.js` to
`scripts/bench-hls.min.js` in the **remote test workspace** (do not commit the
bundle). Run the LAN browser command separately with
`--sustained direct`, `--sustained remux` and `--sustained transcode`.
The report includes the bundle hash, client/worker hashes, source/binary
identity, and fixture identity.

Each mode uses the same generated local-storage 36-second 720p/24 H.264/AAC
clip, starting red and then showing a moving test pattern. Two isolated
Chromium contexts share the fixture viewer credential. Each watches for
20 seconds after the first decoded frame; the harness requires at least
18 seconds of overlap, correct first-frame pixels/dimensions and at least
18 seconds of decoded media progress per player. It checks media peer
identity and deletes the HLS sessions. Browser waiting events and frame
callback gaps are recorded separately; the latter include client scheduling.

The worker samples core RSS and aggregate FFmpeg RSS/worker count every
100 ms. FFmpeg CPU is the sum of maximum sampled CPU time for observed
processes and is a lower bound. Resources are reported across each mode run
and include gaps between browser cohorts. CPU transcode is forced. Browser
playback overlap does not imply sustained simultaneous FFmpeg activity:
event HLS can encode ahead. The muted frame checks do not certify audio
quality or equivalent perceptual quality. No SMB, mixed workload or Jellyfin
comparison is inferred from these diagnostics.

Five cohorts per mode (ten player windows each) passed on N100, controlled by
SSH with an Apple M4/Chromium LAN client. All runs used the same source and
release binary, a byte-identical 13,940,091-byte fixture, and hls.js 1.7.4.
There were zero browser waiting events and zero frame gaps over 250 ms.
The minimum overlap was 19.90 seconds, and the smallest decoded media advance
was 19.875 seconds. All HLS session deletes and worker shutdowns succeeded.

| Mode | First frame median (min–max; sample σ), ms | Max callback gap, ms | Sampled core peak RSS, MiB | Peak FFmpeg workers / RSS, MiB | FFmpeg sampled CPU lower bound, s |
| --- | --- | --- | --- | --- | --- |
| [Direct](benchmarks/rust-n100-lan-sustained-direct.json) | 26.95 (14.60–167.10; 57.70) | 83.30 | 10.73 | 0 / 0 | 0 |
| [Remux](benchmarks/rust-n100-lan-sustained-remux.json) | 418.75 (402.30–567.20; 63.00) | 150.00 | 12.27 | 2 / 108.21 | 35.83 |
| [CPU transcode](benchmarks/rust-n100-lan-sustained-transcode.json) | 1,526.15 (1,452.80–1,650.30; 69.19) | 150.00 | 12.03 | 2 / 361.32 | 229.21 |

Startup statistics are descriptive across ten player samples per mode;
the two players in each cohort are paired, not independent repetitions.
FFmpeg RSS is the sum across live workers at each sample. These observations
do not establish a capacity limit, long-soak behavior, hardware transcode,
other clients, or mixed scan/playback capacity.
