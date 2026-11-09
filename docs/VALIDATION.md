# Rust preview validation — 9 November 2026

These results apply to this Rust preview on the shared macOS ARM64 development
host. Historical application/prototype reports do not certify this implementation.

| Check | Result |
| --- | --- |
| Rust workspace check and optimized release build | Passed, Rust 1.99.1 |
| rustfmt and Clippy with warnings denied | Passed |
| Rust integration suite including explicitly enabled generated-media tests | 14 passed, zero ignored in that run |
| Transactional v1→v2 migration, legacy `$2a$` bcrypt, sessions and watch state | Passed |
| Future schema refusal, exclusive data-directory lock | Passed |
| SQLite admission backpressure / responsive runtime under overload | Passed |
| Authorization, CSRF, range requests, grant/session revocation and rate limits | Passed |
| Media symlink escape rejection and offline NAS record preservation | Passed |
| Incremental scans, NFO import and manual metadata preservation | Passed |
| Personal list ownership, ordering, revision conflicts and access filtering | Passed |
| Subprocess timeout, stdout limit, dropped-request cancellation and reaping | Passed |
| Live WAL backup/restore, preserved lists and restored-session revocation | Passed |
| Real FFmpeg remux, adaptive HLS, alternate audio, text subtitles and burn-in | Passed with local FFmpeg 6.0 |
| Synthetic PQ HEVC software conversion to BT.709 | Passed; not HDR visual-quality or GPU acceptance |
| Svelte/TypeScript diagnostics and production web build | Passed; zero diagnostics |
| Generated frontend contract types current | Passed |
| Python backup and query regression tests | 3 passed |
| Chrome 156.0.8078.80 household and persistent-queue workflows | Both passed |
| Firefox 159.0 household and persistent-queue workflows | Both passed |
| Playwright WebKit 27.2 household and persistent-queue workflows | Both passed |
| Separate Rust benchmark prototype check/format | Passed |
| Production authenticated API smoke benchmark | 12 trials / 3,600 requests; zero errors; warm loopback, 10k items, concurrency 1/10 |
| Rust native ARM64 Docker image build | Attempted; failed on Docker storage: `no space left on device` |
| Rust amd64 image / multi-architecture deployment / real Nginx | Unrun |
| N100 physical VAAPI SDR graph | Short synthetic quality/audio slice passed; see continuation evidence |
| Hardware HDR / sustained N100 playback / other GPUs | Unrun for Rust |
| N100 cold-cache / sustained mixed workload / pinned Jellyfin comparison | Unrun for production Rust |
| Real iOS/Android/TV devices and external security review | Pending |
| Registry or GitHub publication | Not performed |

Compact browser-version/case evidence is in `validation-browser.json`. Queue
coverage includes create/add/reorder, automatic next episode, persisted cursor
and reload. Desktop engine automation does not certify real mobile/TV devices or
every Safari release. The media tests use a small synthetic corpus and do not
establish complete audio, subtitle, HDR or container coverage.

The API smoke report `benchmarks/rust-production-local.json` includes hardware,
compiler, bundled SQLite version, source/binary hashes, errors, resource samples
and repeated-run latency variation. A shared warm development-host smoke run is
not a benchmark of Jellyfin or a stream-capacity guarantee.

Runtime GPU smoke detection and pre-publication software fallback are implemented,
and the N100 VAAPI SDR graph has a short physical test. Hardware HDR tone mapping remains
missing. Software HDR has not passed a visual-quality corpus.

The container build was stopped by the existing Docker storage limit before the
Rust image could be verified. No images/volumes were deleted to make room. Build
and packaging definitions exist; GitHub CI has not run on this unpublished branch.

The web build retains lazy HLS loading; its HLS chunk produces Vite's >500 kB
warning. npm reports three low-severity development dependency findings. No
zero-finding audit, full parity or stable-release certification is claimed.

## Migration continuation audit

The supplied preview was imported on `rust-migration`; the remote had no branches.
The initial audit re-ran formatting and the normal Rust suite (12 passed, two media
tests ignored). The cursor regression adds simultaneous-write, offline-file,
revoked-grant and restart coverage: 13 non-media tests pass on the Intel N100 over
SSH. Both explicitly enabled software-media tests also pass there with FFmpeg
6.1.1, and locally using the existing FFmpeg 6.0 tool installation. These are CPU
checks, separate from physical acceleration acceptance.

The queue client now uses a revision-checked cursor endpoint, retains hidden
entries, uses persisted positions and refuses replacement edits of a filtered
list. Nine browser cases passed (the original two plus offline-entry preservation
in each of Chrome, Firefox and Playwright WebKit). Python regressions: three
passed. Web check/build and normal Clippy passed; the lazy HLS chunk warning and
three low-severity npm findings remain.

The N100 workspace is isolated from the historical application. Its SMB mount is
host-mounted read/write; the Rust tests do not write media, and deployment must
still expose it read-only. No host remount, shared cache eviction, unrelated
Docker cleanup, or household service restart was performed.

Physical acceptance is deliberately separate:

```sh
python3 scripts/validate_n100.py --host USER@HOST --key KEY --output results/n100-validation.json
OVL_ACCELERATOR=vaapi cargo test --locked --features gpu-tests --test hardware -- --ignored --nocapture
```

The short N100 VAAPI H.264 SDR decode/scale/encode fixture measured mean SSIM
0.973485 across 84 decoded frames and selected 880 Hz alternate audio. It executes
the application's graph without CPU fallback. This is synthetic SDR evidence,
not HEVC/HDR quality, sustained capacity, AMD/NVIDIA or a visual review. The first
run's zero-crossing audio assertion failed because AAC ringing distorted that
measurement; the accepted test uses dominant spectral frequency and signal RMS.
`docs/validation-n100.json` records commands, source identity, environment and
individual exit codes. Vainfo is not installed; kernel driver and package metadata
are recorded alongside the actual graph execution.

Current-source SSH validation records 16 passing non-media integration tests, two
passing CPU-media tests, a passing physical VAAPI graph, formatting and all-feature
Clippy. The nine browser workflows were rerun successfully after the playback
negotiation change.

The shutdown audit now drains and joins the SQLite owner, rejects retained Db
clones after shutdown and tracks password hashing. Future schema refusal preserves
database bytes and does not switch its journal mode. Browser container negotiation
no longer treats FFprobe's Matroska/WebM demuxer aliases as interchangeable.

The initial N100 warm API diagnostic is still running. It has recorded high-load
errors and latency gate failures. Some trials overlap acceptance builds/tests;
it must not be used as clean production acceptance. The bundled-SQLite local
profile `benchmarks/rust-sql-local-initial.json` identifies a full-library sort in
unfiltered browse, motivating an order-preserving title-index change. Search
remains unchanged because forcing that index regresses selective queries.

Docker's existing VM filesystem was inspected through a read-only container:
16 GiB total, 100% used, zero free. No pruning was performed. CI now builds and
runs disposable image acceptance for both Linux architectures, retaining result
artifacts. Those jobs are not yet evidence of passing builds or deployment.
