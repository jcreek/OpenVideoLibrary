# Rust preview validation — 10 November 2026

Current evidence is recorded in the dated increments below; later source-matched
artifacts supersede earlier counts without expanding their platform scope.
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

## Pixel-format negotiation continuation — 2026-11-10

Automatic playback now checks source pixel format as well as codec. The optional
OpenAPI `Capabilities.video_pixel_formats` map permits explicit per-codec native
client reports. Absent reports retain a conservative 8-bit 4:2:0 baseline; empty
allowlists reject copying and unknown source pixel formats require conversion.
Manual modes remain overrides. The generated frontend contract and Svelte
capability reports were updated together; see `PLAYBACK.md`.

The expanded generated software corpus has eight cases across six containers,
including actual H.264 `yuv420p10le` and `yuv444p` sources. Conservative automatic
requests convert these to H.264 `yuv420p`. Tests probe that output, compare decoded
source/output RGB (mean absolute byte error <12), and retain decoded audio checks.
Unit regressions cover old request payloads, legacy `CanPlayMP4`, unknown formats,
empty per-codec lists and explicit native-client formats for direct/remux. This
is a synthetic quality slice, not a full profile/level/real-device certification.

The full local `make verify-local` command exited zero: npm ci/check/build; Rust
format and Clippy including all features; 25 API tests and socket shutdown; three
Python regressions; generated-type consistency; benchmark crate check/format;
four explicitly run CPU media/fault tests; twelve full Chromium/Firefox/WebKit
scenarios (37.4 seconds, no failures/skips/retries); cached ARM64 Nginx TLS/range/
CSRF/backup/restore/shutdown acceptance against the local macOS binary. Browser
and Nginx reports are `validation-browser-pixel-formats.json` and
`validation-nginx-pixel-formats.json`. This does not rerun Linux images, Linux
Chromium, real mobile devices, production deployment or performance acceptance.
The cached headless-shell defect from the previous slice remains separate.

Independent read-only review requested probing the converted output format; that
regression was added and passed, with no remaining scoped blocking findings.
Hosted Actions remain disabled. Publishing remains subject to the pending user
approval; no remote push or PR edit was retried.

`validation-n100-pixel-formats.json` is terminal with all eight SSH checks passing:
environment/driver identity, format, Clippy, integration, software media, physical
SDR and physical HDR. The current source hash is recorded in the artifact. The
positive physical tests use the existing process-local full Intel driver and do
not change the system driver or services. The separate negative system-driver
check is recorded in `validation-n100-pixel-driver-rejection.json`; its result
must be read independently from positive GPU acceptance. These checks do not
certify sustained stream capacity, other GPUs, GPU containers or full HDR quality.

## Bitmap subtitle continuation — 2026-11-10

The previous burn-in graph passed all codecs to a text filter. Running that graph
on the original generated PGS fixture fails with FFmpeg's “Only text based
subtitles are currently supported”. Bitmap codec routing now composes the decoded
subtitle once before adaptive scaling, with two complex-filter threads. HDR base
video receives software SDR tone mapping before bitmap composition. Existing
process admission, cancellation/reaping, workspace quotas and permission checks
remain in force; selected subtitles still use CPU encoding.

The new explicitly ignored software test generates original two-track PGS
display sets, then tests sixteen sessions across SDR and tagged PQ black-base
HEVC, single/adaptive output and starts at 0, 1.5 and 3.5 seconds. Decoded pixels
verify opaque and half-transparent cues, wrong-track absence and explicit cue
clearing in 720p/480p outputs. Audio power, BT.709/yuv420p output for the HDR slice,
invalid-track rejection without session allocation and DELETE cleanup are checked.
This is PGS acceptance for a generated slice, not DVD/DVB/XSUB/font/style or broad
HDR quality acceptance. The frame sampler decodes forward using output-side
FFmpeg seeking; input-side HLS seeking was found to shift sampled timestamps.

Final local `make verify-local` exited zero: npm ci/check/build; Rust format and
Clippy including all features; 25 API tests plus socket shutdown; three Python
regressions; generated-contract consistency; benchmark crate check/format; five
explicit CPU media/fault tests; twelve full Chromium/Firefox/WebKit scenarios
(37.4 seconds, no failures/skips/retries); cached ARM64 Nginx TLS/range/CSRF/backup/
restore/shutdown drill against the local macOS binary. Browser/Nginx reports are
`validation-browser-bitmap.json` and `validation-nginx-bitmap.json`.

An initial full run failed Clippy on fixed-size audio chunk iteration in the new
test. It was corrected to the pinned toolchain's array-chunk API. The terminal
initial SSH snapshot is retained as `validation-n100-bitmap-initial.json`; its
Clippy check failed even though its other checks passed. It is not acceptance
of the final source. The corrected source is validated separately.

Independent read-only adversarial review requested distinguishing unverified
bitmap codecs and the combined HDR branch. Documentation retains the codec
limits, and the PQ black-base plus subtitle test was added. Alpha and single-
rendition coverage also pass. No scoped blocking review findings remain.
Hosted Actions remain disabled; no remote publication was retried. Linux images,
Linux Chromium, real devices, production deployment, sustained/cold/mixed/LAN
performance and broader parity still require their recorded remaining evidence.

The corrected `validation-n100-bitmap.json` is terminal: all eight SSH checks
passed, including format/Clippy, integration, software media and separate physical
VAAPI SDR/HDR tests using the existing isolated full Intel driver. Its recorded
Rust source hash matches the final checkout. This does not establish physical
GPU bitmap rendering; subtitle composition/encoding remains the software path.
The physical tests revalidate the existing independent SDR/HDR pipeline.

The separate current-source system-driver rejection also passed; see
`validation-n100-bitmap-driver-rejection.json`. No driver was installed or changed.

## Styled text subtitle continuation — 2026-11-10

The new generated ASS regression reproduced missing cues when starting playback
at 1.5 seconds. Restoring the requested seek around rendering fixed zero-based
sources; an additional real Matroska fixture with a five-second timestamp origin
then reproduced missing cues at playback start zero. The final fix restores both
the source origin and rounded input seek only around text rendering, then removes
that offset before encoding. The additive OpenAPI-owned probe field
`format.start_time` preserves the raw origin; generated frontend types and actual
probe endpoint assertions were updated. Raw duration semantics are unchanged.

Twelve generated software sessions cover two source origins, two ASS tracks and
starts before/during/after a 1–3 second cue. An original 1,240-byte font renders
red bars with a deliberately empty center; a fontless control must not match.
Decoded RGB verifies font geometry, styling, position, selection and cue clearing.
A gated audio tone verifies the corresponding audible/silent intervals after
seeking. The saved font regenerates byte-identically with fontTools 4.60.2; no
system font is copied/installed and no generator dependency is needed by the
server or test runner. This is one static ASS/font/style slice, not universal
SSA/font/language/effect/VFR/delayed-track/discontinuous-timestamp acceptance.

The Svelte client offers text tracks as browser text or burn-in, with an ASS/SSA
styling-preservation label. The browser fixture includes a real embedded text
track; its workflow selects manual direct mode, then burn-in, and verifies a
transcode request with the selected subtitle plus decoded media readiness.
This browser scenario is not claimed as a complete ASS visual device matrix.

Final local `make verify-local` exited zero: npm ci/check/build; Rust format and
Clippy including all features; 25 API tests plus socket shutdown; three Python
regressions; generated-contract consistency; benchmark crate check/format; six
explicit CPU media/fault tests; twelve full Chromium/Firefox/WebKit scenarios
(38.8 seconds, no failures/skips/retries); cached ARM64 Nginx TLS/range/CSRF/backup/
restore/shutdown drill against the local macOS binary. Reports are
`validation-browser-styled.json` and `validation-nginx-styled.json`.

Independent read-only review found no scoped blocking findings and highlighted
nonzero origins and broader timestamp cases. The origin counterexample was
implemented and fixed; the other limits remain explicit in `PLAYBACK.md`.
Linux images/Linux Chromium, real devices, deployment, full performance and
broader parity remain open. Hosted Actions remain disabled; no publication was
retried while approval is pending.

`validation-n100-styled.json` is terminal with all eight checks passing: environment/
driver identity, format, Clippy, integration, software media and separate physical
VAAPI SDR/HDR. Its source hash matches the checkout, including the saved font
fixture. `validation-n100-styled-driver-rejection.json` separately records a
passing system-driver rejection. The isolated full driver is used only for the
positive physical tests; no system driver or service was changed. These GPU
checks do not claim hardware subtitle rendering or production stream capacity.

## Expanded bitmap corpus and keyframe seek regression (2026-11-10)

The current native generated PGS/DVD/DVB/XSUB test exercises 70 application
playback sessions inside one ignored API test. Frequent source keyframes first
reproduced a lost active cue after seeking; native controls also exposed XSUB
stale-overlay behavior and divergent MPEG-TS input origins. Indexed Matroska/AVI
now prerolls subtitles independently; MPEG-TS retains sequential decode-and-trim.
Native alpha/timing controls, decoded output pixels/audio, cue clearing, two
tracks, single/adaptive outputs and session cleanup pass. PQ black-base covers
PGS/DVD/DVB; XSUB is SDR/opaque DXSB-style only. Extended alpha, real-language
coverage and long-file/SMB seek I/O/latency remain unaccepted.

`make verify-local` exited zero with npm ci/check/build, workspace fmt/Clippy/tests,
all-features Clippy, generated-type consistency and benchmark-crate checks;
25 ordinary API tests and the socket shutdown test; six explicitly ignored CPU
media/failure tests; five Python regressions; 12 browser scenarios across
Chromium/Firefox/WebKit (38.7 seconds); and cached real Nginx TLS/range/CSRF/queue/
backup/restore/shutdown acceptance. Browser and Nginx results are retained in
`validation-browser-bitmap-expanded.json` and
`validation-nginx-bitmap-expanded.json`. Software is distinct from GPU acceptance.

`validation-n100-bitmap-expanded.json` is terminal with eight successful SSH
checks: environment, isolated driver override, formatting, Clippy, integration,
software media, physical SDR and physical HDR. Its source hash matches the
current crates snapshot. The separate installed-driver negative guard also
passes in `validation-n100-bitmap-expanded-driver-rejection.json`; no system
driver was changed. These results do not certify production concurrency, LAN
first-frame latency, SMB seeking, native clients or full Jellyfin parity.

The diagnostic subprocess runner additionally has timeout and output-limit
regressions verifying killed children are reaped. See `BENCHMARKS.md` for the
bounded synthetic seek profile and its limitations. Hosted Actions remain
manual-only; these checks used local hardware and SSH, not Actions minutes.

## Audio channel negotiation (2026-11-11)

Automatic direct playback now checks the selected audio stream's channel count.
The generated AAC 5.1 regression exercises six sessions, including mono and
stereo downmix, alternate audio, seeked conversion and explicit six-channel
direct selection. Decoded tones verify routing, center and surround
contributions, track isolation and absence of clipping. This does not certify
surround passthrough or physical device decoding.

Local verification passed: 27 ordinary API tests, socket shutdown, seven
explicit CPU media and failure tests, 12 browser scenarios and disposable
Nginx acceptance. The Python suite passed all seven tests after independent
review corrected the OpenAPI 3.1 integer/null declaration and type generator.
Generated types were regenerated and checked for byte consistency.
See `validation-audio-channels-local.json`, `validation-browser-audio-channels.json`
and `validation-nginx-audio-channels.json`.

All eight checks in `validation-n100-audio-channels.json` passed over SSH and
its source hash matches the Rust checkout. CPU media and physical VAAPI SDR/HDR
are separate checks. This run did not repeat the installed-driver negative
test, production benchmarks, multi-architecture builds or real-device acceptance.
No hosted Actions minutes were used.

## Automatic fallback capability checks (2026-11-11)

A generated H.264/AAC fixture verifies that an incompatible automatic
fallback returns HTTP 422 without allocating a session. The regression checks
clients that omit H.264 or AAC support and clients that explicitly reject all
H.264 pixel formats. Existing successful direct and conversion cases still
pass. Independent review confirmed the distinction between remuxed source
formats and encoded output, and the scope of the AAC check.

`make verify-local` passed: frontend checks/build, Rust formatting/Clippy,
27 ordinary API tests plus socket shutdown, seven explicit CPU media/failure
tests, seven Python tests, type consistency, benchmark-crate checks,
12 Chromium/Firefox/WebKit scenarios and disposable real Nginx acceptance.
Browser and proxy evidence is in `validation-browser-fallback.json` and
`validation-nginx-fallback.json`. No hosted Actions ran.

All eight source-matched checks in `validation-n100-fallback.json` passed
over SSH, including separate CPU media and physical VAAPI SDR/HDR runs.
Installed-driver rejection, production benchmarks, multi-architecture images
and real-device acceptance were not rerun for this increment.

## Embedded artwork and video stream selection (2026-11-11)

Generated MP4 tests confirm audio with attached artwork alone returns 422
without creating a video session. A mixed MP4 remuxes the 320×180 H.264 movie,
with the cover excluded. MP4 may expose cover art after the movie, so a separate
probe-metadata regression checks artwork and thumbnails before the movie and
asserts the absolute video mapping. Independent review found no blockers and
confirmed that the mixed MP4 does not independently exercise cover-first HDR
enrichment or bitmap mapping.

Local `make verify-local` passed: frontend build/check, formatting and Clippy,
28 ordinary API tests and socket shutdown, eight explicit CPU media/failure
tests, seven Python tests, generated types and benchmark-crate checks, 12
browser scenarios and disposable Nginx acceptance. The browser and proxy
artifacts are `validation-browser-artwork.json` and
`validation-nginx-artwork.json`. No hosted Actions ran.

All eight source-matched SSH checks passed on N100; see
`validation-n100-artwork.json`. The report separates software media from
physical VAAPI SDR/HDR. The installed-driver negative test, production
benchmarks, multi-architecture images and real-device checks were not rerun.

## Process-cold browse and search diagnostic (2026-11-11)

The benchmark harness now times and validates the first authenticated query
after a clean restart. A local smoke run completed six disposable restarts;
six first queries and 12 follow-up requests passed. Follow-up independent
review confirmed comparable pre-load query counts, child-liveness checks and
preservation of a failed first query in an incomplete report.

N100 release build and benchmark completed over SSH with source and harness
hashes verified: 20 first queries and 2,000 follow-up requests, zero errors.
See `BENCHMARKS.md` for measurements and limits. Rust source was unchanged
from the eight-check N100 artwork acceptance. Full `make verify-local`
passed again, including 28 API tests plus socket shutdown, eight explicit CPU
media/failure tests, seven Python tests, frontend checks, generated types,
12 browser scenarios and disposable Nginx. No GitHub Actions ran.

## Timed N100 saturation diagnostic (2026-11-11)

Timed load now retains the authenticated page/result checks, bounds request
count and drains outstanding work. Six local timed smoke trials completed
without errors; a separate cap smoke verified six cap hits and exit status 1
with no HTTP errors. Independent review found no blockers. Full
`make verify-local` passed, including frontend, Rust, Python, contract,
CPU media, 12 browser scenarios and disposable Nginx. No Actions ran.

The N100 timed matrix ran over SSH with the same source and release binary as
the process-cold run. All 20 trials completed with no cap hits; the expected
nonzero exit records 374,886 overload responses at 100 clients. At 50 clients
all 338,360 requests succeeded across the 100k browse/search trials. See
`BENCHMARKS.md` for latency, resource and scope limits. Rust code is unchanged
from the eight-check N100 artwork acceptance; the timed run did not rerun
physical GPU tests or certify LAN, SMB, mixed load or the pinned baseline.

## Browser decoded-frame diagnostic over LAN (2026-11-11)

The initial synthetic browser page failed to fetch and produced no timing
samples. Independent review also identified administrator token exposure as
a blocker. The accepted harness uses a real N100 page, revokes the bootstrap
admin session on loopback and verifies that the fixture viewer receives 403
on an admin route before binding to the LAN. Follow-up review found no
remaining blockers.

The accepted run verified five decoded red startup frames and five blue
seek frames, actual HTTP 206 fixture responses from the expected direct peer,
and a clean N100 shutdown. SSH controlled the worker; it did not carry the
browser media. The artifact's source and release binary match prior N100
acceptance and its script hashes match the harness. No worker temp directories
remained. See `BENCHMARKS.md` for timing and the buffered-seek/local-storage
limitations. Rust code and the prior physical GPU evidence are unchanged.
Full local verification passed again, including frontend, Rust, Python,
CPU media, generated types, 12 browser scenarios and disposable Nginx.
No GitHub Actions ran.

## Unbuffered LAN seek diagnostic (2026-11-11)

The LAN harness now has a bounded padded fixture and a far seek. All five
unbuffered trials fetched a new same-peer HTTP 206 whose returned range spans
the FFprobe-identified target keyframe. The recorded response start precedes
the decoded blue frame callback in each trial. Follow-up independent review
confirmed the timing and scope; this measures local-storage LAN seeking.
All five default short-fixture regressions also passed and both disposable
N100 servers shut down cleanly. The Rust source and binary are unchanged;
GPU, SMB and sustained streaming were not rerun. See `BENCHMARKS.md`.

`make verify-local` passed again: frontend/Rust checks, API and socket tests,
explicit CPU media/failure tests, seven Python tests, type consistency,
benchmark-crate checks, 12 browser scenarios and disposable Nginx.
No hosted Actions ran.

## Concurrent browser playback over LAN (2026-11-11)

N100 SSH-controlled tests passed five two-player cohorts each for direct,
remux and CPU transcode, with byte-identical generated local-storage media.
All 30 player windows verified decoded output and progress; minimum overlap
was 19.90 seconds and there were no browser waiting events or gaps over
250 ms. Core and FFmpeg resource samples are separate. Independent review
confirmed the results and scope. The HLS runs used two live FFmpeg workers
at sampled peaks; the playback windows do not imply continuous encoding.
See `BENCHMARKS.md` and the three sustained-run artifacts.

Both prior LAN seek variants passed five regression trials with the updated
worker. Full `make verify-local` passed: frontend, Rust, API/socket,
explicit CPU media/failure tests, Python, generated types, benchmark-crate
checks, 12 browser scenarios and disposable Nginx. Rust source and N100
binary are unchanged. Physical GPU, long soak, mixed scanning, SMB and
Jellyfin-relative acceptance remain separate. No hosted Actions ran.

## Mixed scan and playback over LAN (2026-11-11)

Review caught that the proposed two-second schedule was below the server's
60-second minimum. The preliminary run was stopped; its results were not used.
The corrected harness aligns cohorts with supported scheduled ticks, brackets
actual scan completions over SSH and verifies the API window falls inside
both decoded playback windows. It keeps admin authentication on loopback.

Five scan and five idle N100 cohorts completed with identical fixture and
release binary. All 2,000 authenticated API requests passed their page
checks, and all 20 player windows decoded with at least 19.92 seconds of
overlap. There was one observed completion per scan cohort, none per idle
cohort, and both workers shut down cleanly. Waiting events and repeated-run
variation remain in the artifacts; see `BENCHMARKS.md`. Independent review
confirmed the counts and limits. The test covers paced LAN load and an
unchanged-file rescan on local storage; no SMB, cold, saturation or
Jellyfin comparison is claimed.

Buffered and unbuffered SSH regressions each passed five trials. Full
`make verify-local` passed, including frontend, Rust, CPU media, Python,
generated types, 12 browser scenarios and disposable Nginx. Rust source
and physical GPU evidence are unchanged. No GitHub Actions ran.

## Native N100 Nginx and snapshot rollback (2026-11-11)

The N100 has no Docker/Podman/nerdctl runtime and user namespace creation
failed. Docker Desktop's separate VM remains at 16 GiB used, zero free.
No unrelated Docker objects were deleted. The existing writable SMB mount
was not used or altered. Current multi-architecture image, in-container GPU,
production TLS/mount and retained-binary rollback acceptance remain open.
See `ROLLOUT.md` for reproducible commands and prerequisites.

An unprivileged Nginx 1.24.0 Ubuntu package was downloaded and extracted
in the N100 test workspace over SSH; its package hash matched the APT index.
The native drill binds Nginx and Rust to loopback and renders the repository's
Nginx locations with temporary TLS and ports. It passed trusted TLS cookies,
CSRF, range bytes, proxy-origin 413/429, live WAL backup and session revocation.
After editing the restored queue, a second restore rolled it back to the
saved cursor. All three Rust processes and the native Nginx master exited
cleanly and the proxy listener closed. This is data rollback with the same
binary and a read-only fixture file, not a binary downgrade or mount test.

`validation-nginx-n100-native.json` retains package, binary, source, config
and harness identities. The cached Docker Desktop proxy passed the updated
drill with the Rust backend also restricted to loopback; see
`validation-nginx-cached-loopback.json`. The Linux cached Docker host-network
branch was not run. Independent review found no remaining blockers for the
tested scopes. Full `make verify-local` passed, including 12 browser scenarios,
CPU media, Rust/frontend checks, Python and generated types. Rust source
and physical GPU evidence are unchanged. No hosted Actions ran.

## Music direct playback preview (2026-11-11)

A generated six-format API test verifies music-only extension scanning,
episode-like track names staying out of TV series, bounded tag display,
grant revocation, correct byte ranges and no conversion allocation.
Container and audio-stream custom tags are excluded from the audio-only
probe response. Review caught raw stream tags, and the generated test caught the legacy
library-kind constraint; the final code clears the raw audio-only tags and records
music classification in an additive table with a transactional create.

Full local verification passed: 28 API tests plus socket shutdown, nine
explicit CPU media/failure tests, seven Python tests, frontend/Rust checks,
type consistency, benchmark-crate checks, 15 browser scenarios and cached
Nginx. Chromium, Firefox and WebKit each exercised actual MP3 playback,
embedded tag display and automatic music queue advancement. The six-format
API test does not certify every codec in each browser. A separate live
backup/restore smoke preserved music classification, the track queue and
password login; the old session was rejected. See
`validation-browser-music.json` and `validation-nginx-music.json`.

All eight source-matched N100 checks passed over SSH, including separate
software media and physical VAAPI SDR/HDR checks; see
`validation-n100-music.json`. Installed-driver rejection, current container
images, production benchmarks and native devices were not rerun. Music
conversion, album/artist grouping and lyrics remain open. No hosted Actions ran.

## Local music lyrics (2026-11-11)

The music endpoint reads local LRC/TXT and returns bounded timed or plain
lines. The API regression covers repeated timestamps and ordering, UTF-8 BOM,
plain-text fallback, sidecar symlinks, a replaced library-root symlink,
oversized input/expanded text, line limits and revoked grants. Review found
that timed-out filesystem work could outlive its permit and that a replaced
root could redirect a canonicalized read. The final worker holds its lease
until it exits and traverses the registered path with no-follow directory
handles. Follow-up review found no remaining blockers in this slice.

Full local verification passed: 29 API tests plus socket shutdown, nine
explicit CPU media/failure tests, seven Python tests, frontend/Rust checks,
type consistency, benchmark-crate checks, 15 browser scenarios and cached
Nginx. All three browser engines exercised click-to-seek lyrics and clearing
the previous lyrics on queue advance. See `validation-browser-lyrics.json`
and `validation-nginx-lyrics.json`. LRC offsets, embedded/enhanced lyrics,
external providers and real-device checks remain open.

All eight final-source N100 checks passed over SSH, including separate
software media and physical VAAPI SDR/HDR checks. See
`validation-n100-lyrics.json`. The installed-driver negative guard,
production benchmarks, container images and real devices were not rerun.
No hosted Actions ran.

## Registered library root replacement (2026-11-11)

The shared resolver now compares the resolved library root with the
registered path rather than trusting a new target. Scanning makes the
same check and does not follow a root symlink. The regression replaces
the registered directory with a link to an outside tree, verifies denial
on file, probe, artwork, playback and subtitles, and verifies a saved
queue hides the entry without losing it. Scanning marks the library
offline without importing the outside item. Restoring the directory
recovers the original item and queue entry.

Independent review confirmed this scope. Pathname opens can still race
a concurrent replacement after validation; the guarantee here concerns
a root that is already redirected when checked. Legacy noncanonical
paths fail closed and retain their records; see `ROLLOUT.md`.

Full local verification passed: 30 API tests plus socket shutdown, nine
explicit CPU media/failure tests, seven Python tests, frontend/Rust checks,
type consistency, benchmark-crate checks, 15 browser scenarios and cached
Nginx. No GitHub Actions ran.

All eight source-matched N100 checks passed over SSH, including separate
software media and physical VAAPI SDR/HDR. See
`validation-n100-library-root.json`, `validation-browser-library-root.json`
and `validation-nginx-library-root.json`. Installed-driver rejection,
production benchmarks, current containers and real devices were not rerun.

## Offline legacy library path repair (2026-11-11)

The repair command works on a fresh restore of a verified v2 snapshot.
Three Python regressions use the real schema and verify preserved file/item
IDs, grants, progress, queues, credentials and music classification, plus
session revocation, source snapshot preservation, rejected symlink/size
mismatches and refusal to overwrite a destination. Independent review
found no blocker for the documented offline, trusted-directory scope.
The command verifies paths and sizes, not content identity.

All three regressions passed over SSH on N100. A disposable restored Rust
runtime also accepted the repaired paths: login worked, the old token was
rejected, rescan preserved the original item ID and progress, the queue
resolved the item, byte ranges returned the expected content, and shutdown
was clean. No household data or media were used. See
`validation-n100-path-repair.json` for source and tool identity.

Full local verification passed with ten Python tests, 30 ordinary API
tests plus socket shutdown, nine explicit CPU media/failure tests,
frontend/Rust checks, type consistency, benchmark-crate checks, 15 browser
scenarios and cached Nginx. The Rust source is unchanged from the eight-check
N100 root validation. GPU and production deployment were not rerun, and no
hosted Actions ran.

## Global LRC timing offsets (2026-11-11)

The parser applies the last valid global signed offset to each timed line.
Positive values delay lines, following the convention used by
[Rockbox](https://github.com/Rockbox/rockbox/blob/master/apps/plugins/lrcplayer.c).
The unit regression covers a tag after cues, both signs, zero clamping,
invalid offsets and unchanged plain text. Review identified a browser
test that could pass through normal playback after an incorrect seek.
The revised check pauses the player, selects an offset lyric and verifies
the corrected time before resuming. All three browser engines passed it.

Full local verification passed, including 15 browser scenarios, the
parser unit test, 30 API tests plus socket shutdown, nine explicit CPU
media/failure tests, ten Python tests, frontend/Rust checks, generated
types, benchmark-crate checks and cached Nginx. See
`validation-browser-lyrics-offset.json`. No GitHub Actions ran.

All eight source-matched N100 checks passed over SSH, including the parser
test, software media and separate physical VAAPI SDR/HDR. See
`validation-n100-lyrics-offset.json`. Production benchmarks, current images,
installed-driver rejection and real-device checks were not rerun.

## Embedded music lyrics (2026-11-11)

The generated music fixture verifies embedded timed text in MP3, FLAC,
Vorbis and Opus, local sidecar precedence, empty fallback for unsupported
M4A/WAV and no lyric text in ordinary probe responses. The lyrics probe
consumes the already-open source descriptor through stdin and holds the
inspection permit through process cleanup. Independent review found no
blockers in this scope. See `PLAYBACK.md` for demuxer, timeout and output
limits.

Full local verification passed: parser unit test, 30 API tests plus
socket shutdown, nine explicit CPU media/failure tests, ten Python tests,
frontend/Rust checks, type consistency, benchmark-crate checks, 15 browser
scenarios and cached Nginx. The browser scenarios exercise the existing
music/sidecar UI, while the embedded format coverage comes from the API
fixture. See `validation-browser-embedded-lyrics.json`. No Actions ran.

All eight source-matched N100 checks passed over SSH, with generated software
media and physical VAAPI SDR/HDR recorded separately. See
`validation-n100-embedded-lyrics.json`. Installed-driver rejection, current
container builds, real devices and production benchmarks were not rerun.

## Local album covers (2026-11-11)

Music artwork checks conventional cover and folder image names in the
track directory. Review identified that a symlink to a same-directory
non-image could pass containment and receive an active media type. The
final route requires the canonical image path to match the expected path
and sets an explicit JPEG, PNG or WebP content type. The regression covers
same-directory HTML and outside-directory symlinks, local cover precedence,
preserved video fallback and grant revocation. Follow-up review found no
blockers. This does not certify arbitrary image bytes or eliminate all
concurrent pathname-open races.

Full final-source local verification passed: parser unit test, 31 API
tests plus socket shutdown, nine explicit CPU media/failure tests, ten
Python tests, frontend/Rust checks, type consistency, benchmark-crate
checks, 15 browser scenarios and cached Nginx. Chromium, Firefox and
WebKit each decoded a generated PNG through the authenticated artwork
endpoint. See `validation-browser-music-cover.json`. No hosted Actions ran.

All eight final-source N100 checks passed over SSH, with software media
and physical VAAPI SDR/HDR measured separately. The earlier run is not
used for this claim. See `validation-n100-music-cover.json`. Installed-driver
rejection, production benchmarks, current images and real devices were not
rerun.

## Embedded music covers (2026-11-11)

Generated MP3 and FLAC tracks with attached 640×320 images verify opt-in
fallback to a 512×256 PNG, image MIME, unchanged default browsing and no
extraction for other tested formats. The ordinary music probe, lyrics,
range and grant tests still pass. Chromium, Firefox and WebKit each decode
an embedded cover in the music player after queue advancement. Independent
review found no blocking findings. See `PLAYBACK.md` for bounds and scope.

Full local verification passed: parser unit test, 31 API tests plus socket
shutdown, nine explicit CPU media/failure tests, ten Python tests,
frontend/Rust checks, generated types, benchmark-crate checks, 15 browser
scenarios and cached Nginx. See `validation-browser-embedded-cover.json`.
No hosted Actions ran.

All eight source-matched N100 checks passed over SSH, including the
generated MP3/FLAC software cases and separate physical VAAPI SDR/HDR
checks. See `validation-n100-embedded-cover.json`. Installed-driver rejection,
production benchmarks, current images and real devices were not rerun.

## Indexed music album browsing (2026-11-11)

Generated music fixtures verify album-artist grouping, embedded display
titles, numeric track ordering, query bounds and immediate grant revocation.
A disguised playlist is admitted as a filename by the scanner but yields
no indexed album. Independent review caught unrestricted automatic probing;
the final scanner uses an anchored file, a pipe-only protocol, the expected
demuxer and a held inspection permit. It bounds probe admission per scan and
does not rewrite manual item metadata. Follow-up review found no blockers.

Full local verification passed: parser unit test, 31 API tests plus socket
shutdown, nine explicit CPU media/failure tests, ten Python tests,
frontend/Rust checks, type consistency, benchmark-crate checks, 15 browser
scenarios and cached Nginx. Chromium, Firefox and WebKit each opened a scanned
album and showed its tracks. See `validation-browser-music-albums.json`.
Large-library indexing cost, seek-dependent M4A layouts and standalone
artist browsing remain open. No GitHub Actions ran.

The initial N100 album snapshot passed the music case but failed the existing
styled-subtitle media case with an intermittent 502. Its report is retained
as `validation-n100-music-albums-initial.json`. A playlist could be published
between the request's failed filesystem lookup and its observation of
encoder completion. The handler now samples completion before checking
the file, so publication during the lookup cannot produce that false
missing-output result. Review confirmed the ordering fix; the original
failure is consistent with this race, but its cause was not proven.

Final full local verification and all eight source-matched N100 checks
passed. The N100 software run includes both the music case and the
styled-subtitle case that failed initially. Physical VAAPI SDR/HDR were
verified separately. See `validation-n100-music-albums.json`.
Installed-driver rejection, production benchmarks, current container
images and real devices were not rerun.

## Indexed music artist browsing (2026-11-11)

The generated music regression checks artist grouping, all-track listing and
empty artist and track results after revocation. The browser music workflow
opens an artist, displays its album, opens all tracks and returns to the artist
directory. Independent review found stale rows during navigation. The client
now clears the previous results before fetching the next view.

Local verification passed the frontend checks and build, Rust format and
Clippy, parser unit test, 31 API tests, socket shutdown, nine explicit CPU
media/failure tests, ten Python tests, contract generation and benchmark
crate checks, and the disposable cached-image Nginx drill. After rebuilding
the final frontend, all 15 browser scenarios passed across Chromium,
Firefox and Playwright WebKit. See `validation-browser-music-artists.json`.
Hosted Actions were not used. Artist artwork, contributor-level grouping and
large-library performance were outside this validation.

The final Rust source also passed all eight checks in the isolated N100 SSH
matrix, including the CPU-only media suite and separate physical VAAPI SDR
and HDR graph checks with the process-local driver override. See
`validation-n100-music-artists.json`. This run did not benchmark production
performance or rerun the installed-driver rejection, image or device suites.

A local Docker Desktop ARM64 build at `93a6022` stopped at Docker Hub
metadata resolution for `node:22-bookworm-slim` with `DeadlineExceeded`.
A direct pull of that ARM64 base also stalled and was stopped. The build
never reached compilation or image smoke testing. The N100 test account
has free disk space but no container runtime. Current image acceptance
requires access to the base images from a native builder with sufficient
storage. No Docker images, volumes or caches were pruned.

The music regression now also scans an albumless MP3. It verifies that the
artist directory counts the track without increasing its album count, that
omitting the album filter returns the track, and that an explicit empty
album retains the previous HTTP 200 empty-list response. Full local
verification passed again, including 15 browser scenarios and the Nginx
drill. All eight source-matched SSH checks passed on N100, with CPU
media and physical VAAPI SDR/HDR recorded separately in
`validation-n100-albumless.json`. This was a compatibility check, with no
new container, device or production performance acceptance.

## AAC HLS music fallback

The generated music regression requests fallback for a FLAC file, follows
the authenticated master and variant playlists, decodes the MPEG-TS segment
and verifies mono AAC, non-silent samples and the expected shortened output
after a 250 ms seek. Session deletion removes the stream. Chromium forces
fallback through the web player, verifies media time advances and checks
the session URL returns 404 after closing. Firefox and Playwright WebKit
cover the direct music path in the same workflow.

Final local verification passed the frontend checks and build, OpenAPI type
consistency, Rust formatting and Clippy, the parser unit test, 31 API tests,
socket shutdown, nine explicit CPU media/failure tests, ten Python tests,
benchmark crate checks, 15 browser scenarios and the disposable Nginx drill.
See `validation-browser-music-aac.json`. Independent review confirmed the
shared stream guards and verified the added playlist/browser coverage and
lazy HLS loading. No hosted Actions ran.

All eight source-matched N100 checks passed over SSH, including the CPU
AAC regression and separate physical VAAPI SDR and HDR graph checks with
the process-local driver override. See `validation-n100-music-aac.json`.
Music fallback itself uses CPU AAC encoding. Image, device and production
performance acceptance were not part of this run.

### Current ARM64 image retry at `1f4c629`

The Docker Hub metadata lookup succeeded. The local native ARM64 builder
then failed with `no space left on device` while ingesting the Rust base
image into Docker Desktop's internal containerd store, before Rust compilation.
A disposable Dockerfile using the smaller Rust slim base reproduced the
same failure. Neither attempt produced an image to smoke-test. The normal
Dockerfile is unchanged. No unrelated Docker data was pruned.

The SSH preflight found about 281 GiB free on the x86-64 N100 root
filesystem. It found no container CLI in PATH or socket at the Docker or
containerd default locations. Increase Docker Desktop's available image storage or provide a
native ARM64 builder with sufficient space. Native amd64 acceptance also
requires a usable builder, for example a provisioned runtime on the N100.
The build and smoke commands are in `ROLLOUT.md`.

## Audio sample-rate capability ceiling

The optional ceiling gates automatic direct play and selects an AAC output
rate. Regression coverage includes the inclusive request boundaries, values
just outside them, unknown and above-ceiling source rates, and 8 kHz output
arguments. The generated FLAC case offers direct FLAC support with a 25 kHz
ceiling, then follows HLS playlists and decodes 24 kHz mono AAC after a seek.

Final local verification passed frontend checks and build, contract
generation, Rust format and Clippy, one parser unit test, 31 API tests,
socket shutdown, nine explicit CPU media/failure tests, ten Python tests,
benchmark crate checks, 15 browser scenarios and the disposable Nginx drill.
Independent review confirmed compatibility and the added boundary coverage.
The initial N100 software run exposed a fixture requesting stereo while
asserting mono PCM length. The final fixture requests and verifies mono.

The corrected fixture passed all eight source-matched SSH checks on N100.
The CPU media suite and the physical VAAPI SDR/HDR checks are distinct.
See `validation-n100-audio-rate.json`. No new image, real-device or
production performance results were collected, and no Actions ran.

## Transcode quota filesystem failure

Quota monitoring tolerates a segment that disappears during FFmpeg's
temporary-file rename. A filesystem failure on the session directory
takes the same cancellation and reap path as a storage or idle limit.
The fault regression moves the isolated transcode directory while its
simulated encoder is alive, then checks the encoder PID is gone when
the stream has been removed. The source fixture remains unchanged.

Local verification passed frontend checks and build, generated contract
consistency, Rust format and Clippy, the parser unit test, 31 API tests,
socket shutdown, nine explicit CPU media and fault tests, ten Python
tests, benchmark crate checks, 15 browser scenarios and the disposable
Nginx drill. The final fault regression was rerun after making the PID
and stream-removal checks race-safe. Independent review confirmed the
cleanup and test fixes.

The final committed Rust source passed all eight N100 SSH checks. The
CPU media and fault suite included the hardened quota regression.
Physical VAAPI SDR and HDR passed separately with the isolated driver
override. See `validation-n100-quota.json`. The separate low-priority
Jellyfin fixture scan was active during these functional checks. This
run collected no performance measurements.

## Single-library browse pagination

The profiled query selects ordered file rows before reading metadata
and user progress. The regression verifies absent and newly granted access,
per-user progress, duplicate files crossing the page boundary, and
identical results with the original multi-library path. Independent
review confirmed those semantics. Local verification passed frontend
checks/build, contract generation, Rust format/Clippy, parser unit,
31 API and socket shutdown tests, nine explicit CPU media/fault tests,
ten Python tests, benchmark crate checks, 15 browser scenarios, and
the disposable Nginx drill. No Actions ran.

All eight source-matched N100 checks passed over SSH. CPU media and
physical VAAPI SDR/HDR were verified separately. See
`validation-n100-scoped-browse.json`. The matched 100k timing run
started after the functional tests and release build finished.

## Covering library browse order

The additive SQLite projection backfills existing file rows and updates
with file inserts, reassignment, deletion and title changes. Scoped
unfiltered browse pages the projection before loading user progress
and item details, with current grants checked in the same transaction.
The schema version remains 2. The regression covers a legacy database
backfill, backup restore, duplicate file rows, reassignment, title
changes, deletion and current grants. Independent review found no
remaining correctness issue.

Local verification passed frontend checks and build, Rust formatting
and Clippy, the parser unit, 31 API and socket shutdown tests, nine CPU
media/fault tests, ten Python tests, benchmark crate checks, contract
generation, 15 browser scenarios and the disposable Nginx drill. The
final scoped restore assertion and Clippy were rerun locally.

All eight source-matched N100 checks passed over SSH, including the
backup restore assertion, CPU media/fault suite, and separate physical
VAAPI SDR/HDR checks. See `validation-n100-covering.json`. On a disposable
copy of the existing 100k v2 database, the candidate backfilled 100,000
matching file rows, retained schema version 2, passed SQLite quick-check,
and exited zero.

## Retained-binary Nginx snapshot rollback

Over SSH, native Nginx on the N100 proxied the retained page-first release,
the covering-index candidate, and then the retained release again. The
candidate opened a fresh restore of the retained release's live snapshot.
After the candidate's queue edit, another fresh restore recovered the
saved queue cursor under the retained binary. Credentials, session
revocation, range media, proxy limits, and graceful exits passed.
`validation-nginx-n100-retained.json` records both source and binary
identities, the Nginx build and the harness hash. Independent review
confirmed the sequence and the disposable snapshot scope. The exercise
does not cover a live production cutover or a production mount.

The initial full local run had one Playwright WebKit queue scenario
timeout. Its trace showed a click during player startup and no list
write request. The unchanged full rerun passed all 15 browser scenarios,
the default same-binary cached-Nginx drill, frontend and Rust checks,
31 API tests, nine explicit CPU media and fault tests, ten Python tests,
and generated contract and benchmark crate checks. No Actions ran.

## Queue add captures the clicked title — 2026-11-12

`addToList` now captures the selected title and target list before awaiting
the list GET, and clears the previous notice when a new add starts. Switching
titles during the request can no longer add the newly opened title, and a
rejected write no longer leaves an earlier success notice beside its error.
The browser regression delays the first list GET, switches titles, and polls
persisted queue order. A second step returns a mocked 409 for the list PUT and
requires the error alert, no success notice and unchanged persisted entries.
Server-side stale-revision rejection remains covered by the API tests.

The updated scenario passed three repeats in each of Chromium, Firefox and
Playwright WebKit on macOS. A rerun of those repeats is archived in
`validation-browser-queue-repeats.json`. The revert checks are recorded only in
this text. With only the capture fix reverted, it failed in
Chromium on persisted order. With only the notice clear removed, it failed in
Chromium on the leftover success notice. Two independent read-only reviews
found no blocking issue. The first review's stale-notice finding and its two
test-robustness nits were fixed. A display-only title change during an admin
metadata edit was left as cosmetic.

The first full local run failed one API backup test because cached test
binaries embedded the checkout's pre-rename path through `CARGO_MANIFEST_DIR`.
Rebuilding the workspace crate fixed it, with no source change. The final
`make verify-local` then exited zero on the reviewed revision. It covered
frontend check and build, Rust format and Clippy, 31 API tests, nine explicit
CPU media and fault tests, ten Python tests, generated contract and benchmark
crate checks, 15 browser cases (five scenarios in each of three engines),
and the cached-Nginx drill. See `validation-browser-queue-capture.json`. No Actions ran. Linux browser engines
remain unrun because the N100 has no Node or Playwright, and installing them
would need system packages.

All eight N100 checks passed over SSH with the isolated full-driver override.
They included CPU media, and separate physical VAAPI SDR (mean SSIM 0.9738)
and HDR checks. The Rust source hash matches `validation-n100-covering.json`,
because this increment changes only web files. See
`validation-n100-queue-capture.json`.

## Long-file seek timeline and A/V sync — 2026-11-12

A generated corpus encodes the source frame number as sixteen bars. Audio
beeps at each source second with a tone that steps every minute. It found two
seek defects. Remux used stream copy from FFmpeg's input seek, so video began
at an earlier keyframe, up to one GOP before the requested start. The server
still reported the requested start, which the client used as its timeline
offset. Displayed and saved positions and client WebVTT cues were therefore
wrong by up to a GOP, after a silent lead-in. MPEG-TS has no seek index, so
transcodes started at the next keyframe. A 37.3 s request began at 40.0 s, and
600.6 s began at 610.0 s.

Remux now keeps source timestamps with `-copyts` and zero mux delay. The
session response reports where the first published segment begins. A bounded
segment probe measures it, and the value is accepted only between 0 and the
request plus 0.1 s. Otherwise it falls back to the request. The client offsets
its timeline by the reported start and then seeks past any lead-in to the
requested position. For MPEG-TS only, remux and transcode first seek onto the
preceding keyframe. A packet-header lookup finds it: one packet, then 2 s and
8 s windows, with a 3 s deadline. Transcodes then trim the remaining gap after
decoding. MP4 and Matroska transcodes keep the original accurate seek. If a
client disconnects while the start waits for the first segment, the session is
cancelled and FFmpeg is reaped.

The independent review found a blocking flaw in the first design. Predicting
FFmpeg's seek failed for Matroska with B-frames, where FFmpeg's DTS heuristic
lands a GOP early. It also found an extra per-variant filter cost on every
transcode seek. The rework above replaced both. Follow-up reviews found the
orphaned-session window and wrong measurements from wrapped or shifted
timestamps. Both were fixed with regression tests, and the final review found
no blocking issue.

Coverage:

- `real_long_file_seeks_keep_source_timeline_and_av_sync` uses 10-minute
  23.976 fps `-bf 3` MP4, Matroska and MPEG-TS files, plus a 180 s MP4 with a
  45 s GOP. Remux and transcode seek to 37.3, 301.7, 580.4 and 134.0 s. It
  checks that the first frame matches the reported start, that later frames
  match the start plus stream time, and that each beep aligns within two
  frames. Decoding uses closed playlist snapshots and timestamp selection,
  because FFmpeg output seeking skews audio when audio starts after video.
  The test fails on the previous source: a request for 37.3 was reported as
  37.3, but the stream began at frame 720.
- `abandoned_remux_start_cancels_the_session` fails with the guard
  neutralised.
- A unit test covers keyframe parsing, arguments and the measurement clamp.
- A browser scenario reports a start 2 s earlier than requested. It checks
  the lead-in skip and saved progress. With the previous client it fails by
  2.02 s.

The final `make verify-local` passed. It covered frontend check and build,
Rust format and Clippy, 32 API tests, ten explicit CPU media and fault tests,
ten Python tests, contract and benchmark checks, 18 browser cases (six
scenarios in each of three engines) and the cached-Nginx drill. See
`validation-browser-keyframe-seek.json`. An earlier full run failed
one browser case because of a test race: selecting remux and clicking Apply
each send a playback request. The test now acts only on the seek request, and
then passed two full suites and the final run. All eight source-matched N100
checks passed over SSH, including both new media tests and physical VAAPI SDR
and HDR. See `validation-n100-keyframe-seek.json`.

Seek-start cost on the N100 (Ubuntu FFmpeg 6.1.1, 1-minute load average about
4.6 from unrelated host services, n=20 per row, warm cache):

- the MPEG-TS keyframe lookup has a median of 112–171 ms, with p95 up to 205 ms;
- the remux segment-start probe has a median of about 62 ms, with p95 68 ms;
- the first remux segment took a median of 127–203 ms to appear. Clients
  already had to wait for it before playback, so this cost moved into the
  response rather than being added.

These are diagnostics, not LAN first-frame acceptance.

Known limits:

- If the first segment does not appear within 20 s, remux reports the
  requested start.
- Video-only MPEG-TS with a missed keyframe lookup can start up to a GOP late,
  or show a video lead-in.
- Open-GOP sources are untested.
- VAAPI with the MPEG-TS output trim is unrun.
- Real media, SMB/NAS seek I/O, Safari/iOS native HLS and LAN seek latency
  remain unverified.

No Actions ran.

## Surround codecs and legacy containers — 2026-11-12

Two generated corpora extend the video acceptance slice.

**Surround routing.** Each source channel carries its own tone:
AC-3 5.1, E-AC-3 5.1(side), DTS 5.1 and TrueHD 5.1, and FLAC, PCM s24, Opus
and AAC 7.1, all in Matroska. Each plays through automatic remux for an
AAC-only stereo browser, and DTS also for a mono browser. Decoded output keeps
left channels left and right channels right, places centre equally in both,
and drops the LFE. Mono keeps every non-LFE tone. This adds coverage only: it
also passes on the previous source. A deliberately inverted expectation fails,
which shows that the assertions run.

**Legacy containers.** The corpus covers:

- MPEG-1 and MPEG-2 program streams;
- MPEG-2 with AC-3 in MPEG-TS;
- WMV2/WMA in ASF;
- Sorenson FLV with MP3, and H.264/AAC FLV (remux);
- Theora/Vorbis in Ogg;
- SVT-AV1/Opus in Matroska;
- HEVC (`hvc1`) in MP4.

Each case plays in automatic mode for an H.264/AAC browser at 0 s, 4.3 s
(mid-GOP) and 5.02 s (just after a keyframe). The MPEG-2 fixtures use a 2.5 s
GOP. Checks cover the first frame against the reported start, frames and beep
alignment after the start, and audio/video segment starts within 0.1 s.

The corpus found two defects:

- MPEG-2 in a program stream (format `mpeg`) started at the next GOP after a
  seek, as MPEG-TS did. A 4.3 s request began at 4.5 s. The keyframe lookup
  now covers `mpegts`, `mpeg` and `mpegvideo`.
- The server always passed an input `-ss`. At 0 it failed on FLV without a
  keyframe index, and the demuxer then dropped about 1.7 s of leading audio.
  Starts at 0 now omit the input seek. FFmpeg rebases timestamps the same way,
  and hardware options are placed before `-i` when there is no seek. The
  review compared first-segment timestamps and packet counts with and without
  `-ss 0` for B-frame MP4, Matroska, MPEG-TS and open-GOP Matroska, and found
  them identical.

One rounded-up millisecond value now drives the input seek and every subtitle
offset. The legacy test fails on the previous source at the MPEG-2 program
stream seek. Unit coverage asserts that a start at 0 has no `-ss` and that
`-hwaccel` precedes `-i`.

Independent review found nothing blocking. Its rounding nit and its request
for more seek phases were applied.

Known limits:

- ASF, Ogg, FLV and AVI keep FFmpeg's seek; non-zero seeks measured accurate.
- The raw `mpegvideo` lookup is best effort.
- DVD VOB cells with PTS discontinuities, and program-stream bitmap burn-in
  after a seek, are untested.
- About 0.7% of millisecond start values round 1 ms late.
- Surround passthrough to capable clients and real-media audio remain open.

The first full local run stopped at a Clippy type-complexity error in the new
test, which a type alias fixed. The final `make verify-local` passed. It
covered frontend check and build, Rust format and Clippy, 32 API tests, 12
explicit CPU media and fault tests, ten Python tests, contract and benchmark
checks, 18 browser cases and the cached-Nginx drill. See
`validation-browser-media-corpus.json`. The N100 SSH matrix is in
`validation-n100-media-corpus.json`. No Actions ran.

## DVD VOB bitmap subtitles — 2026-11-12

The bitmap subtitle matrix now includes a DVD-style MPEG program stream: MPEG-2
video, AC-3 audio and `dvd_subtitle`, muxed as VOB. Burn-in used a second
program-stream input for the subtitle. That composited the first DVD cue from
the start of the stream: the cue pixel read 254 at 0.25 s, when the cue starts
at 1.0 s. The independent review traced the cause to the unmapped second
input, which supplies no blank sub2video heartbeat before the first cue.
Decoding that input's video corrected it, and Matroska PGS and DVD tracks were
unaffected.

Bitmap burn-in from MPEG program streams now uses the single-input sequential
path that MPEG-TS already used. It composes from the file start and trims the
output to the requested start. That path ignores a keyframe, so `start()` no
longer runs the keyframe lookup for it. The VOB row covers cue selection,
clearing, a 2.3 s seek across an active cue, and single and adaptive scaling.
It fails on the previous source, where the cue was visible at 0.25 s.

The final `make verify-local` passed. It covered frontend check and build,
Rust format and Clippy, 32 API tests, 12 explicit CPU media and fault tests,
ten Python tests, contract and benchmark checks, 18 browser cases and the
cached-Nginx drill. See `validation-browser-dvd-subtitles.json`. All eight
source-matched N100 checks passed with Ubuntu FFmpeg 6.1.1. See
`validation-n100-dvd-subtitles.json`.

Seek cost is the known limit. Sequential burn-in decodes, overlays and scales
every frame before the start. On the N100, decoding 5 minutes of 720x480
MPEG-2 took 3.4 s with 2 threads and 58 MB RSS. A closer approximation
decoded, scaled to 1280x720 and encoded 4 s of x264 after an output seek to
280 s. It took 11.8–12.0 s over n=3 runs with 151 MB RSS, at a 1-minute load
average of about 2. That is about 23x real time, which extrapolates to roughly
65 s for a 25-minute seek and 2.6 minutes at 60 minutes. These figures are
extrapolations, not measured long-file runs. MPEG-TS bitmap burn-in already
had the same cost. A bounded pre-roll would need proof of correctness: the
review showed that a naive input seek on a program stream leaves cue and video
timelines inconsistent.

Other limits:

- A VOB whose first subpicture falls after FFmpeg's default probe window
  lists no subtitle track.
- Cells with PTS discontinuities are untested.
- The fixture covers only one early cue.

No Actions ran.
