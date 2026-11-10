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
