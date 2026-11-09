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
