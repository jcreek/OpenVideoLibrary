# Rust preview validation — 9 November 2026

Current evidence is summarized in **Guarded VAAPI HDR10 increment** below.
Source hashes and platform scope belong to each captured artifact; historical
application/prototype reports do not certify this implementation. The migration
is published as draft PR #1; no registry image or stable release is published.

## Initial preview checkpoint (historical)

The following table and notes record the initial shared macOS ARM64 preview.
Later continuation checkpoints supersede their counts and unrun statuses; they
are retained to distinguish historical runs from current-source acceptance.

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
and packaging definitions existed; at that checkpoint GitHub CI had not run.
Later Linux CI evidence is recorded separately below.

The web build retains lazy HLS loading; its HLS chunk produces Vite's >500 kB
warning. npm reports three low-severity development dependency findings. No
zero-finding audit, full parity or stable-release certification is claimed.

## Migration continuation audit (chronological checkpoints)

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

An earlier source snapshot recorded 16 passing non-media integration tests, two
passing CPU-media tests, a passing physical VAAPI graph, formatting and all-feature
Clippy. The nine browser workflows were rerun successfully after the playback
negotiation change.

The shutdown audit now drains and joins the SQLite owner, rejects retained Db
clones after shutdown and tracks password hashing. Future schema refusal preserves
database bytes and does not switch its journal mode. Browser container negotiation
no longer treats FFprobe's Matroska/WebM demuxer aliases as interchangeable.

The initial N100 warm API diagnostic completed 80 trials / 80,000 requests.
It recorded high-load errors and latency gate failures; the raw report is
`benchmarks/rust-production-n100-initial.json`. Some trials overlap acceptance builds/tests;
it must not be used as clean production acceptance. The bundled-SQLite local
profile `benchmarks/rust-sql-local-initial.json` identifies a full-library sort in
unfiltered browse, motivating an order-preserving title-index change. A bounded census now selects the index for broad, unfiltered search while sparse
searches retain their selective plan. Correlated FTS was profiled and rejected.

Docker's existing VM filesystem was inspected through a read-only container:
16 GiB total, 100% used, zero free. No pruning was performed. CI now builds and
runs disposable image acceptance for both Linux architectures, retaining result
artifacts. Those jobs are not yet evidence of passing builds or deployment.

The expanded local CPU corpus now passes MP4/AAC, Matroska/AAC, WebM/Opus,
MPEG-TS/MP2, AVI/MP3 and MOV/PCM negotiation, decoded test-pattern pixels and audio
RMS checks. The synthetic software HDR test now exercises both PQ and HLG inputs
and verifies BT.709 output tags. This does not establish HDR visual fidelity,
Dolby Vision, surround passthrough, bitmap subtitles or font coverage.

A real cached ARM64 Nginx 1.30.5 container passed TLS, secure cookies, CSRF,
byte-range/content-range, request-size limits, live WAL backup/fresh restore,
restored-session revocation, queue preservation and clean Rust SIGTERM checks.
`scripts/test_nginx.py` reproduces the disposable drill; `validation-nginx.json`
records evidence. The backend for the local run is macOS Rust; this is not Linux
image or production-host deployment certification. Nginx startup initially failed
because fastcgi/uwsgi/scgi temporary paths used the read-only root; all temporary
paths are now explicitly on the bounded tmpfs. CI runs the same drill on Linux.

Stopping a session now cancels its owned work even when the source disappears or
permissions are lost; the API still returns 404 when current access is absent.
Segment waits also require the session's original library on every check.

A later expanded N100 CPU run exposed a session idle-expiry failure while an
actively authorized request waited for its playlist during concurrent diagnostic
load. This is recorded as a failure, not a pass. Segment waits now refresh their
activity timestamp; a clean N100 rerun is still required for that source. The
normal local suite passes 17 tests; three explicit CPU-media tests pass locally.

An isolated rootless-runtime check on N100 found no container daemon or uidmap
helper, and `unshare -Ur true` failed with `Operation not permitted`. No namespace
policy, OS package, service or shared mount configuration was changed. Pinned
Jellyfin image and Linux deployment acceptance need an authorized container
runtime or another isolated equivalent environment.

The VAAPI HDR10 feasibility probe succeeded on N100 using mastering-display/CLL
metadata and hardware decode, `tonemap_vaapi`, scale and H.264 encode, followed by
software decode of the result. `validation-n100-hdr-probe.json` records the probe.
The server still uses software HDR: the probe does not establish visual quality,
HLG/Dolby Vision handling or a tested production HDR negotiation/fallback path.

Direct/range/artwork/HLS response bodies now end on server cancellation, so a
stalled media peer cannot retain the response for the rest of a movie during
shutdown. The local regression holds an unread direct response across shutdown
and verifies that it ends within one second.

The clean N100 rerun after the active-wait fix passed all recorded checks, including
three CPU-media tests. The earlier concurrent failure remains in
`validation-n100-concurrent-failure.json`; it has not been erased or relabeled.
A subsequent local storage audit rejects symlinked database/cache/workspace roots
before SQLite or startup cleanup runs, separates library paths from application
data, and confines cached artwork to its owned directory. Those newest guards
still require the next source-specific N100 validation after the API profile ends.

Real TCP SIGTERM regression coverage now holds both an incomplete request body and
an unread 64MiB sparse direct-media response. The listener cancels blocked socket
reads/writes while preserving actual client addresses for login rate limiting.
Both cases exit cleanly within the five-second test deadline; response-body-only
cancellation is not treated as sufficient proof of stalled-peer shutdown.

The latest local stabilization snapshot passes 21 API integration tests plus the
real-socket shutdown test (two peer-stall scenarios). Three explicit CPU-media
tests, all-feature Clippy, web install/check/build and nine desktop browser cases
also pass. The newest storage/transport changes are newer than the last captured
N100 validation artifact; they are not silently attributed to that earlier run.

The latest real Nginx/TLS restore drill also passes against the cancellation-aware
server, recorded by binary hash. Docker's cached tag/manifest lookup failed, so
the rerun used the exact already-cached native Nginx image ID; no image pull,
prune or unrelated data deletion was used to obtain that result.

## Bounded search cache increment

The current local source passes web install/check/build, format, all-target/all-
feature Clippy, 23 normal API tests plus the real-socket shutdown test, all three
explicit CPU-media tests, three Python regression tests, contract regeneration,
the separate benchmark crate check/format and nine Chrome/Firefox/WebKit cases.
The real cached Nginx/TLS backup/restore drill also passes, with the binary hash
in `validation-nginx.json`. The first local CPU-media invocation failed because
FFmpeg was absent from PATH; rerunning with the existing ffmpeg-static and
ffprobe-static binaries passed. It was an environment failure, not a skipped test.

New coverage checks live progress/grant changes, metadata invalidation, external
writes against populated cache membership, eight-term eviction, genuine SQLite
page-quota failure, and complete tail results beyond the 100,000-match cache cap.
Independent read-only adversarial review found two coverage gaps and a cross-PR
CI cancellation issue; all three were fixed and the reviewer confirmed no
remaining blocking finding in this increment's scope. This is not a full external
security review of the product.

`validation-n100.json` captures a complete SSH Linux validation of the current
cache implementation including both added review regressions: 23 normal API
tests, shutdown, format/Clippy, three CPU-media tests and the physical VAAPI SDR
test pass. The earlier 22-test cache snapshot was superseded after performance
measurement finished. Software PQ/HLG success is separate from
physical SDR evidence; hardware HDR acceptance remains missing.

GitHub run 42647295810 at source `e76268b` passed Linux verification and amd64
container smoke acceptance (`validation-linux-amd64-initial.json`). Its emulated ARM64
job was still live at that observation and was later cancelled as superseded.
A subsequent workflow revision removes
duplicate branch-push/PR runs, retains main push and manual execution, groups
supersession by PR number, bounds job runtime and uses native ARM runners.
That revised workflow has passed actionlint and launched as run 42651686557;
all native image and verification jobs subsequently passed.

The cache's N100 diagnostic completed all 80 trials/80,000 requests with zero
errors through concurrency 50 and 7,523 HTTP 503s at concurrency 100. The harness
exited nonzero as required. At 100k, search p95 median is 7.36 ms at concurrency
10 and 36.92 ms at concurrency 50 (worst per-run 42.10 ms). See
`benchmarks/rust-production-n100-cached.json` and `BENCHMARKS.md` for per-run
uncertainty, resources, source identities and limits. This does not complete
cold-cache, LAN, mixed-load, playback or pinned-Jellyfin acceptance.

The native amd64 and ARM64 image jobs in run 42651686557 subsequently both passed
at source `97bc011`. `validation-linux-amd64.json` and
`validation-linux-arm64.json` retain image IDs, architectures, source/run
provenance and API/range/read-only-mount/grant/queue/SIGTERM checks. This resolves
the multi-architecture build/smoke gap without deleting Docker data. It does not
validate production-host Nginx, rollback to the prior application or physical GPU
behavior inside those images. The separate verification job also passed,
including Linux browser/software-media checks and the disposable Nginx/TLS
backup/restore drill; `validation-nginx-linux.json` retains its artifact with
source provenance. It remains an isolated CI drill, not the N100 deployment.

## Guarded VAAPI HDR10 increment

The current source passes local web install/check/build, format, all-target/all-
feature Clippy, 24 normal API tests plus the real-socket shutdown test, three
original CPU-media tests and the new CPU-only simulated-HDR-fault test, three
Python regressions, contract regeneration, benchmark-crate check/format, nine
Chrome/Firefox/WebKit cases and the disposable real Nginx/TLS restore drill.
The latter records its binary hash in `validation-nginx.json`. The existing
three low-severity development dependency findings and HLS bundle-size warning
remain; no clean dependency audit is claimed.

`validation-n100-hdr-full-driver.json` records the final-source SSH N100 run:
format, Clippy, 24 normal API tests, socket shutdown, all four explicit CPU-media
checks, physical SDR and physical HDR pass. Physical checks explicitly select the
isolated full Intel driver; its path and SHA-256 are recorded. The HDR test checks
actual PQ luminance/color tiles, first/middle/final frames, two output sizes,
seeking, alternate audio, BT.709 tags, the runtime picture guard and authenticated
playback. The API session records VAAPI rather than CPU fallback.

`validation-n100-hdr-driver-rejection.json` separately passes the negative physical
check against the installed free Intel media driver. `validation-n100-hdr-graphs.json`
records seven successful encodes with black decoded output under that driver;
`validation-n100-hdr-full-driver-graphs.json` records the same graphs with distinct
luminance values using the isolated full driver. The initial metadata-guard failure
is retained in `validation-n100-hdr-first.json`. An earlier exit/decode-only HDR
feasibility probe is insufficient acceptance and is superseded by these picture
checks. No system driver, service, namespace policy or shared mount was changed.

Runtime HDR remains opt-in, requires successful frame-side metadata inspection
and known color/range properties, and rejects silent-black drivers. CPU override
and subtitle software conversion remain intact. New real HTTP fault coverage
checks forced CPU, probe rejection, hardware failure before master publication,
no CPU replacement after publication, and cancellation during probe initialization.
DELETE must reap the child/remove the workspace within the test deadline while
the server is still alive; a separate SIGTERM case verifies shutdown cleanup.

Independent adversarial review identified missing range/frame-probe guards,
first-frame-only checks and fallback/cancellation coverage gaps. The implementation
and regressions address them, including a follow-up correction preventing SIGTERM
from masking ineffective DELETE cancellation. This scoped review is not a complete
external product security audit.

See `HDR.md` for exact admission rules, driver provenance and reproducible commands.
The calibrated synthetic slice does not establish real-film perceptual quality,
HLG hardware, Dolby Vision, other physical GPUs, real displays, sustained HDR
performance or GPU-in-container deployment. Existing native Linux image and Linux
Nginx artifacts still apply to their recorded earlier source; current-source CI
has not yet run at this checkpoint. The full parity roadmap remains incomplete.

## Local-only verification policy — 10 November 2026

Hosted workflow 412257201 is disabled by user direction, and automatic push/PR
triggers have been removed. No run was active when disabled. `make verify-local`
preserves the verification inventory without hosted execution or image pulls;
physical N100 and image checks remain separate. See `LOCAL-VERIFICATION.md`.
The workflow syntax passes actionlint, the local script passes `bash -n`, and an
independent read-only review found no issue in this configuration change.

The last hosted run, 42657120330 at `9572feb`, passed native amd64/ARM64 image
checks but failed the Chromium persistent-queue case; eight browser cases passed.
The Linux Nginx step was therefore unrun. This failure is preserved and remains
under local investigation. The earlier macOS browser pass does not override it.
Local Docker storage and N100 container-runtime constraints still block fresh
local multi-architecture image acceptance. No unrelated data was deleted and no
hosted replacement run will be started without explicit authorization.

## Playback lifecycle continuation — 2026-11-10

Generation guards now prevent stale probe, progress, close, Apply and queue-cursor
continuations from replacing newer player intent. Late playback POST responses
remain observable and their obsolete server sessions are deleted before another
creation proceeds. Completed cleanup chains discard historical result arrays.
The browser harness drains stdout/stderr into a bounded 64 KiB tail and attaches
it on failure.

Local evidence: npm ci/check/build, Rust format, Clippy (including all features),
24 API tests plus socket shutdown, three Python regressions, generated-contract
consistency, benchmark crate check/format and four CPU media/fault tests passed.
The final frontend was checked/built again after review fixes. All twelve browser
scenarios passed across installed Chrome, Firefox and Playwright WebKit, including
delayed probe, close-save, Apply-save, queue cursor and late HLS-session cleanup.
Earlier attempts exposed a WATCHED-badge selector defect and unsafe in-flight
route teardown in the new test; both were corrected before this passing run.
The original Linux Chromium failure remains historical evidence, not a claimed
Linux rerun. Hosted Actions remain disabled.

`validation-n100-player.json` records all eight checks passing over SSH in an
isolated workspace: environment, driver identity, format, Clippy, integration,
software media, physical SDR and physical HDR. Physical checks use the existing
process-local full Intel driver; they are distinct from CPU media tests and do
not establish production stream capacity. `validation-nginx-player.json` records
a passing cached ARM64 Nginx TLS/range/CSRF/backup/restore/shutdown drill against
the local macOS Rust binary. It is not Linux production deployment acceptance.

Independent read-only adversarial review found and verified fixes for delayed
queue intent, delayed Apply saves and retained cleanup history; no scoped blocking
finding remains. Fresh local images remain blocked by Docker storage. No
unrelated data was deleted; no hosted run was dispatched. Broader platform,
real-device, LAN/cold/mixed-load, pinned-Jellyfin and parity requirements remain
open as documented in STATUS/PARITY.

The optional macOS bundled Chromium headless-shell attempt failed all four cases
because its cached bundle cannot find `icudtl.dat` and its GPU process aborts.
This is not a passing browser result; installed Chrome's four cases pass. The full
bundled Chromium then passed all four scenarios (13.6 seconds); Linux Chromium
remains unrun. The headless-shell failure is retained as a separate environment
defect, not waived or relabelled.

Final end-to-end confirmation: `make verify-local` exited zero using the installed
full Chromium channel, Firefox, WebKit, FFmpeg/FFprobe and cached Nginx image
`b972f831f200`. All twelve browser scenarios passed in 37.3 seconds. The explicit
environment was `PLAYWRIGHT_CHROMIUM_CHANNEL=chromium`,
`PLAYWRIGHT_BROWSERS_PATH=/Volumes/256 NVMe/Development/ovl-browser-cache`,
`OVL_NGINX_IMAGE=b972f831f200`, with local FFmpeg/FFprobe tools on PATH.
`validation-browser-player.json` preserves the terminal browser report.
