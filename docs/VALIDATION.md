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
| Physical GPU / hardware HDR / sustained N100 playback | Unrun for Rust |
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
but no physical GPU path was exercised here. Hardware HDR tone mapping remains
missing. Software HDR has not passed a visual-quality corpus.

The container build was stopped by the existing Docker storage limit before the
Rust image could be verified. No images/volumes were deleted to make room. Build
and packaging definitions exist; GitHub CI has not run on this unpublished branch.

The web build retains lazy HLS loading; its HLS chunk produces Vite's >500 kB
warning. npm reports three low-severity development dependency findings. No
zero-finding audit, full parity or stable-release certification is claimed.
