# OpenVideoLibrary migration handoff

Updated 15 November 2026 (review-stack split and CodeRabbit reviews). This file records the handoff
for agents resuming work. Read AGENTS.md, STATUS.md, PARITY.md,
VALIDATION.md, BENCHMARKS.md, ROLLOUT.md, ADR-001-rust.md and openapi.json
before changing code or making acceptance claims.

## Checkout and publication

This repository is public. Keep host addresses, account names, key file
names and machine-specific paths out of committed files, and record them
only in the operator's private notes.

- Use the Rust checkout on the development machine. The older Go reference
  was archived outside the repository as a non-git snapshot with no handoff.
- The migration is on `main`, merged through the six stacked PRs described in
  "Review stack". The original branch, `rust-migration`, and its draft PR #1
  were closed and the remote branch deleted because their tree held
  unredacted local-network details. Work from `main`. The only workflow,
  `Verify Rust`, is manual-dispatch and disabled. Pushes spend no Actions
  minutes. Do not re-enable or dispatch it.
- On the N100, use the established unprivileged SSH test account and
  approved key. It runs Ubuntu 24.04 and uses strict host trust.
  `scripts/validate_n100.py --workspace` stages source into
  `<workspace>/source`, which must be new or hold the
  `.ovl-validate-staging` marker, because the upload prunes it. Pass `--vaapi-driver-path` with the isolated full
  Intel driver directory recorded in `validation-n100-covering.json`.
  The N100 has no Node or Playwright, so Linux browser runs are blocked.
- FFmpeg/FFprobe may be absent from PATH on the development machine. Put
  static binaries on PATH, and set `PLAYWRIGHT_CHROMIUM_CHANNEL=chromium`,
  `PLAYWRIGHT_BROWSERS_PATH` and `OVL_NGINX_IMAGE` (a cached Nginx image) as
  described in LOCAL-VERIFICATION.md.
- After a checkout rename or move, run `cargo clean -p ovl-server` first.
  Cached test binaries embed `CARGO_MANIFEST_DIR`. The backup API test failed
  on a stale pre-rename path until the crate was rebuilt.

## Pick up here

Four increments from this session are complete and now on `main`. The current
Linux images also pass the container smoke. With the user's approval,
history from the first handoff commit onward was rewritten to remove the
N100's private address and key filename, then force-pushed. PR #1's
commit IDs changed, but no file content did. Two local-only refs keep the
old history on the development machine:
`local-backup-before-history-scrub` and filter-branch's
`refs/original/refs/heads/rust-migration`. The first is a branch, so never
use `git push --all` or `--mirror`, and delete both once the user agrees.

Old commits may stay reachable on GitHub by commit ID, including from PR #1's
force-push event and `refs/pull/1/head`, until GitHub purges them. If the address must be
unrecoverable, the user needs to ask GitHub Support. The review-stack split
removes local-network details from the tree (see "Review stack").
Keep any such detail in gitignored local files, such as `.env`, or in the
operator's private notes.

1. **Queue capture.** `addToList` captures the clicked title and list before
   awaiting, and clears stale notices. See "Queue add captures the clicked
   title" in VALIDATION.md.
2. **Long-file seek timeline.** Remux reports the start it actually measures
   (`-copyts` plus a bounded first-segment probe, clamped), and the client
   skips the lead-in. MPEG-TS seeks onto the preceding keyframe, and only
   MPEG-TS transcodes trim after decoding. A start abandoned mid-wait cancels
   its session. A generated B-frame MP4/Matroska/MPEG-TS corpus verifies
   decoded frames and A/V beep alignment. See "Long-file seek timeline and A/V
   sync" in VALIDATION.md for evidence, N100 seek-start costs and known
   limits: the 20 s fallback, video-only MPEG-TS lookup misses, open-GOP and
   VAAPI with trim.

3. **Surround and legacy corpus.** The surround corpus verifies that
   AC-3, E-AC-3, DTS and TrueHD 5.1, and FLAC, PCM, Opus and AAC 7.1, route
   correctly through stereo and mono downmix. The legacy corpus covers
   MPEG-1/2 PS and TS, WMV, FLV, Theora, AV1 and HEVC conversion, with exact
   timelines and A/V sync at three seek phases. Two defects were fixed:
   - MPEG program streams now get the keyframe lookup;
   - starts at 0 no longer pass `-ss 0`, which dropped leading FLV audio.
   See "Surround codecs and legacy containers" in VALIDATION.md.
4. **DVD VOB subtitles.** A two-input burn-in showed DVD cues from the
   stream start. Bitmap burn-in from MPEG program streams now uses the
   sequential single-input path. A long-file seek costs about 23x real time
   on the N100, so a 25-minute DVD seek takes roughly 65 s (extrapolated). See
   "DVD VOB bitmap subtitles" in VALIDATION.md.

Each increment passed a full local `make verify-local`, all eight N100 SSH
checks, and independent adversarial reviews that left no blocking finding.

Next, continue the corpus expansion without new hardware or decisions:

- surround passthrough to capable clients (E-AC-3/AC-3 copy) and
  layout/profile negotiation;
- a proven-correct bounded pre-roll for program-stream and MPEG-TS bitmap
  burn-in seeks, VOB cell PTS discontinuities, and probing late first DVD
  subpictures;
- broader subtitle fonts and styling;
- HLG and HDR10+ software paths.

Each increment needs decoded picture and audio checks. Run it locally and on
the N100 and record it the same way. Real-media and SMB/NAS seek latency
needs a read-only fixture mount.

## What has evidence

- **Rust preview and video:** Authentication, grants, v1/v2 SQLite migration,
  scanning and NFOs, search, progress, range/direct playback, remux, adaptive
  HLS, audio selection, subtitle conversion/burn-in, software HDR, and
  persistent collections, playlists and queues have automated coverage.
  Queue adds keep the clicked title across in-flight requests
  (`validation-browser-queue-repeats.json`, `validation-browser-queue-capture.json`,
  `validation-n100-queue-capture.json`).
  Long-file seeks keep the client timeline and A/V sync for remux and
  transcode (`validation-n100-keyframe-seek.json`,
  `validation-browser-keyframe-seek.json`).
  Queue coverage includes concurrent cursor updates, grant loss, offline
  entries and restart. The latest source-matched Rust SSH matrix is
  `validation-n100-dvd-subtitles.json`. It records CPU media and physical VAAPI
  SDR/HDR checks separately, using an isolated process-local driver override.
- **Music:** Direct audio and AAC HLS fallback, indexed album/artist browse,
  embedded/local artwork and lyrics, shared progress and queues. Check
  PARITY.md for format and real-device gaps.
- **Performance:** Warm local N100 diagnostics passed the 150 ms gate for
  the measured 10k and 100k browse and exact-title top-hit workloads. At
  100k, the covering-index scoped browse p95 was 26.34 ms and exact-title
  Rust top-hit p95 was 3.78 ms. Broader search results differed from Jellyfin.
  Response sizes and tested scopes differ. See BENCHMARKS.md and its raw
  reports and replay patches before using any figures.
- **Deployment:** Native N100 Nginx/TLS, live backup, fresh restore and
  retained → candidate → retained snapshot rollback passed over SSH.
  See `validation-nginx-n100-retained.json`. Older amd64/arm64 image
  smoke evidence applies to its recorded source only.

## Stopping point (13 November 2026)

The user chose to stop here. Every remaining item below needs something
from the user that is not available now:

- a read-only fixture mount on the N100 (production Nginx/TLS cutover and
  NAS seek latency);
- root, or an admin-installed container runtime, on the N100 (container and
  GPU-in-container runs);
- a separate LAN client (LAN and browser first-frame comparisons);
- a dedicated host with page-cache control (OS-cold benchmarks);
- product decisions (later parity).

There is no real HDR content. Self-contained work that needs none of these
is still possible: more generated-corpus coverage and the next
benchmark-harness steps. Ask the user before resuming.

## Review stack (13 November 2026)

PR #1 was too large for automated review, so its final tree (`48753e7`)
was split into six stacked PRs. Each one adds only its own files, taken from
`48753e7` with the redactions listed below:

1. `split/1-server`: the Rust server, API contract and `.coderabbit.yaml`.
2. `split/2-web`: the Svelte client.
3. `split/3-ops`: packaging, operations tooling, verify-local and
   `bench/rust`.
4. `split/4-bench`: benchmark harnesses.
5. `split/5-docs`: these docs.
6. `split/6-evidence`: the recorded JSON evidence.

When the split was made, the only new content was:
- `.coderabbit.yaml`;
- `.env.example`;
- the interim Makefiles in PRs 1–2 (replaced by the final one in PR 3);
- this section, the date in the header and the edited publication paragraph.

Each PR then took fixes from its CodeRabbit and independent reviews, recorded
below. So `main` is `48753e7` with the redactions, these additions and those
review fixes, including new tests, a probe-failure table and contract changes
in `docs/openapi.json`.

PR #1 was closed and the remote `rust-migration` branch deleted after the
final adversarial review (15 November 2026), because that unredacted tree
was still public. GitHub keeps `refs/pull/1/head` and old commits reachable
by ID until GitHub Support purges them; the user is asking Support to do
so.

At the user's request, the split also redacts these details:
- Local checkout paths in recorded evidence are now `<repo>` and
  `<browser-cache>`.
- The remote test home directory is now `<bench-home>`.
- The N100 account name is now `benchuser`.
- The N100 hostname in recorded `uname` output is now `<host>`.
- Other services on that host, and its media mount and permissions, are
  no longer named or described.
- In `scripts/validate_n100.py` and `scripts/remote_bench.py`, `--workspace`
  is now required instead of defaulting to a home path. Both scripts
  reject relative or root paths, and the documented commands pass the flag.
- The NAS mount in `compose.yaml` is now `${OVL_MEDIA_DIR}` from a
  gitignored `.env`, with `.env.example` as the template.

The redacted benchmark scripts differ from the ones that produced their
results only in the user and device names. As a result, six evidence
files pin a `harness_sha256` from before the redaction: `n100-native-10k-browse.json`
pins the plain script, and the other five pin it with their committed patch applied:
- `n100-native-10k-exact-search.json`
- `n100-native-100k-browse.json`
- `n100-native-100k-covering.json`
- `n100-native-100k-exact-search.json`
- `n100-native-100k-rust-before-after.json`

The original scripts are kept only on the development machine, in the local
`rust-migration` branch at `48753e7`; the backup refs hold the same files.
Deleting the backup refs is fine while that branch stays. Once GitHub purges
PR #1, these hashes cannot be checked against the public repository, and
deleting the local branch too makes them unverifiable anywhere. The prepared 10k stage on the N100 was
created with the old device name. Regenerate it before rerunning the
redacted 10k script, or the token lookup returns nothing.

- State on 15 November 2026: all six split PRs have merged (GitHub #6, #2,
  #3, #4, #5 and #7), bottom-up. GitHub #1 is closed. While the stack was
  open, every PR stayed in draft until the PR beneath it had merged.

Split gates:
- PR 1: Rust format, Clippy, 32 API tests, 12 ignored media tests and the
  backup tests.
- PR 2: generated types unchanged and 18 browser cases.
- PR 3: full `make verify-local` and the 8 N100 checks.
- PR 4: all 10 Python tests and syntax checks.

**Intermittent N100 failure, still open:** in the first PR 3 N100 run,
`software-media` failed 2 of 12 tests.
- `abandoned_remux_start_cancels_the_session`: the transcode directory was
  not yet empty after its sessions ended.
- `real_legacy_container_corpus_converts_with_exact_timeline_and_sync`:
  the session DELETE returned 404, so the session was already gone.

Both passed when rerun on their own, and a full 8-check rerun passed. The
split's reviewer found likely causes from reading the code. Neither has been
fixed or confirmed by a regression test yet:
- **Remux leftover directory (likely a small product leak).**
  - `playback::start` awaits `create_dir_all` while holding the `streams`
    lock, and only afterwards inserts the session and arms its drop guard.
  - A client disconnecting during that await leaves an empty
    `transcodes/<id>` that nothing owns until the startup sweep.
  - Holding the lock across that filesystem work also serialises admission
    under load.
- **Legacy-corpus 404 (test timing).**
  - The test config's 5 s `stream_idle` lets the reaper remove a finished
    session while the test probes its files on disk without touching it over
    HTTP.
  - Fix this in the test by lengthening the idle period or touching the
    session between probes.

**CodeRabbit review of PR 1 (13 November 2026).** All 15 comments were
verified against the code, and all were valid. They are fixed in one commit on
`split/1-server`, and the rest of the stack was rebased onto it.
- **Media confinement:**
  - Library media are read only through anchored handles, passed as
    `/dev/fd/N`.
  - Library media are confined to local files and the single-file demuxers in
    `media::MEDIA_DEMUXERS`.
  - Encode inputs fail closed.
- **Authentication:**
  - Setup is rejected once configured, before hashing.
  - Login sessions are created only if the verified hash is still stored.
- **Libraries and lists:**
  - Nested library roots are rejected.
  - List names are limited by characters.
- **Media handling:**
  - ID3 USLT lyrics are read.
  - Music search matches case across Unicode.
  - DELETE interrupts cold hardware detection, with a CPU fallback.
- **Requests:** request bodies have a 15 s total deadline.
- **Tests and contract:**
  - The HDR rejection test reports blocked prerequisites.
  - The OpenAPI contract documents `X-OVL-Request` and byte-length passwords.

Evidence:
- Six new regression tests fail on the old code. With the binding and
  body-deadline tests, the local totals are 38 API tests and 14 of 14
  ignored media tests.
- Two N100 runs passed all 8 checks.
- An independent review and its follow-up found nothing blocking.

Still open:
- The subtitles-filter residual (see STATUS.md).
- There is no deterministic test for the login/reset race or for
  cancelling cold detection.
- Existing overlapping library roots in older databases are not flagged.

**Second CodeRabbit review of PR 1 (13 November 2026).** All five comments were
verified against the code and fixed in one commit on `split/1-server`:
- List views omit entries only when revalidation returns 404; other errors
  reach the client.
- LRC offsets follow the LRC format convention: a positive offset shows
  lyrics sooner. This reverses the earlier Rockbox-style direction; the
  browser test in `split/2-web` was updated.
- After an encode fails, an unfinished playlist reload returns the stored
  failure (502). Complete playlists and published segments stay servable.
- NFO sidecars are read through the anchored, symlink-refusing open, and
  the handle is validated as a bounded regular file.
- Failed music tag probes are recorded in the additive
  `ovl_music_probe_failures` table and skipped for 24 hours (track faults)
  or 1 hour (timeouts and I/O errors), or until the file changes, so the
  per-scan probe budget reaches later tracks.

Evidence: 40 API tests, 14 of 14 ignored media tests, the HDR fallback
test, `make verify-local` (18 of 18 browser cases and the Nginx drill),
and two N100 runs with all 8 checks passing. An independent review and
two follow-ups found nothing blocking. The NFO race itself cannot be
reproduced deterministically; its test covers symlink and FIFO refusal.

**CodeRabbit review of PR 2 (14 November 2026).** PR 1 was merged into
`main` with a merge commit, and PR 2 now targets `main`. All eight comments
were verified and fixed in one commit on `split/2-web`:
- Signing out or logging in resets account-owned client state (lists,
  items, forms, player). Responses requested by the previous account are
  dropped.
- Sign-out sends the logout request even when player cleanup fails.
- Library IDs stay strings, so the series filter and the selector agree.
- "All media" clears the series and season filters, and an empty season
  is sent as an empty value instead of `null`.
- Metadata and TMDB results apply only to the item they were requested for.
- `scripts/summarize_browser.py` fails on run-level Playwright errors and
  keeps only their messages.
- Browser test teardown no longer waits for a server that already exited.

Evidence:
- A new browser test fails when the library, season, All media, reset or
  stale-response fix is reverted. A new Python test fails on the old
  summary script.
- `make verify-local` passed: 21 of 21 browser cases and the Nginx drill.
- The server is unchanged, so the N100 was not rerun.
- An independent review and its follow-up found nothing blocking.

Still open: a session that expires (401) keeps the previous account's
state in memory until the next login resets it.

**Third CodeRabbit review of PR 2 (14 November 2026).** A full review
raised seven comments. Six were valid and fixed in one commit on
`split/2-web`:
- Setup mode ends as soon as setup succeeds, so a failed login retries the
  login and not setup. A second login cannot start while one is pending.
- Only the most recently chosen list is shown.
- Progress saves and Mark watched/unwatched are written one at a time, so a
  late save cannot undo a newer watched mark. The watched label now updates
  after each write; it previously stayed stale (found by the new test).
- A startup load that fails for a reason other than 401 keeps the session
  and offers Retry.
- The browser test waits for the scan to finish before refreshing.

Skipped: installing Playwright browsers in `make browser`. Browser engines
are a documented prerequisite, verify-local downloads nothing, and
Playwright reports the install command when they are missing.

Evidence: new browser checks fail when the setup, list, progress or retry
fix is reverted. The double-submit guard has no test. `make verify-local`
passed on its second run. The first run hit the known intermittent
`abandoned_remux_start_cancels_the_session` failure (server code unchanged
here), which then passed five isolated reruns. An independent review and follow-up found nothing blocking;
a client timeout on progress writes was tried and removed, because an
aborted request cannot cancel a write the server has already queued.

Still open: a progress write that never completes delays later writes, and
the player switch that waits for it, until the browser gives up on the
request. A failed save still stops `open()` from starting the next title.

**Fourth CodeRabbit review of PR 2 (15 November 2026).** An incremental
review raised two comments; both were valid and fixed:
- A failed startup status check no longer leaves the page on "Opening
  your library…"; Retry repeats the check.
- A load that fails straight after signing in keeps the session and
  offers Retry.

Evidence: browser checks fail when either fix is reverted, and
`make verify-local` passed. An independent review found nothing blocking. Not covered by a test: a status check that
fails, then succeeds on Retry while the library load fails.

**CodeRabbit review of PR 3 (15 November 2026).** It raised seven comments;
six were valid and fixed:
- The container check now proves the read-only mount denies writes. Before,
  file permissions alone could pass it.
- The native-binary smoke now stops the server before removing its data on
  failure paths too.
- `validate_n100.py` runs from the repository root, hashes before upload and
  records the hash scope (Cargo files, `crates/`, `scripts/`). Each file is
  hashed as a framed record, so different layouts cannot share a hash. Its
  `rsync --delete` only stages into a new directory or one holding the
  `.ovl-validate-staging` marker; a symlinked destination is refused. It no
  longer uploads `.env` files, `*.db` databases (with WAL/SHM) or
  `bench/rust/target`.
- ROLLOUT takes the cutover backup after the previous server stops.
- `compose.yaml` notes that `./data` must belong to UID 10001.
- Repair tests cover several pages and a failure on a later page.

Skipped: creating the schema in `bench/rust`, because the harnesses seed it
and the benchmarked binary must stay unchanged.

Evidence:
- `make verify-local` passed at fix commit `a4b615a`.
- All eight N100 checks passed at `a68ae34`, which adds the framed hash from
  CodeRabbit's incremental review. Its `source_sha256` (v2 format) is
  `33c5ad114e0314434a97f8dbd41e120e3623c896567183b2979d8e1403e5b8c8`. The
  report was not committed.
- The fixed container check passed on the existing arm64 image
  `sha256:14a9dde3…` (built from tree `acf25de`) and failed with the read-only
  flag removed; the old check passed without it. No new image report was
  recorded, and the amd64 image was not rerun.

An independent review rated the unguarded `rsync --delete` close to blocking;
the marker fixed it. All findings were addressed or explained, and follow-up
reviews, including one of the hash framing, found nothing blocking.

**CodeRabbit review of PR 4 (15 November 2026).** It raised 20 comments on
the benchmark harnesses. The live scripts were fixed:
- `bench_suite.py` starts encoders inside its cleanup scope and unwinds
  SIGTERM through it. It no longer offers `h264_vaapi`, because its
  software-decoded input never reached VAAPI surfaces.
- `bench_suite.py` and `remote_bench.py` reject rows whose title does not
  match the query prefix.
- `remote_bench.py` applies one deadline to each whole worker response line.
  It ends the resource window with the measured load and drains background
  requests afterwards; `remote_worker.py` now keeps its children until stdin
  closes. New mixed-mode CPU and RSS are therefore not strictly comparable
  with the recorded matrices (see BENCHMARKS). A trial with no successful
  request no longer crashes the run.
- `playback_reference.py` rejects `--repetitions` below 1, cleans up on
  SIGTERM and fails when FFmpeg emits no output, for example after a seek
  past the end of the file.
- `probe_hdr.py` decodes with `-xerror`. `probe_hdr_quality.py` records a
  graph timeout and keeps the earlier rows.
- `report_n100.py` prints the optimized-matrix conclusion only when the
  dataset is complete, error-free, has 400 trials covering both candidates and
  every concurrency through 100, and stays under 150 ms. Null percentiles
  render as n/a. Its output for the recorded evidence is unchanged.
- `lan_playback_worker.py` names the missing hls.js bundle and where to copy
  it from. `bench-sustained-playback.mjs` starts HLS at position zero.
- The v2 playback-startup harness was changed as described in BENCHMARKS.
  BENCHMARKS now gives true medians: the recorded summaries took the upper
  middle row of eight.

Skipped, with the archived evidence checked instead:
- The 10k browse harness and exact-search patch are kept byte-for-byte,
  because the search and 100k scripts are rebuilt by patching the browse
  harness. Both recorded runs have five complete, error-free trials per arm,
  and the search preflight matched 100 of 100 queries in both arms. The docs
  already ask for fresh output names and that acceptance check.
- The v1 playback-startup harness is kept as recorded; v2 supersedes it and
  carries the requested fixes.

Evidence:
- 28 Python tests pass, including 11 new ones in
  `scripts/test_bench_harnesses.py`. Against the old scripts, the
  partial-line test hangs and seven of the other nine first-round tests fail.
  The report checks added after the follow-up review fail against the first
  fix commit.
- Locally, FFmpeg 6.0 exits 0 after a seek past the end of the file and
  exits 1 with `-abort_on empty_output`. That was not rechecked on the N100's
  FFmpeg 6.1.1.
- The revised CPU sampler ran on the N100 in a throwaway process. A child
  measured 1.40 s while alive and 1.41 s after being reaped.
- An independent review found nothing blocking. Its should-fix findings were
  fixed: the reaping wording and wait, the report crash on null
  percentiles, the report claim check and the resource-window note.
  Not changed: a signal arriving in the instant between starting a child and
  entering cleanup.
- No benchmark was rerun. The only figures that changed are the v2 medians, recomputed from the recorded rows.

**Second CodeRabbit review of PR 4.** It reviewed the two fix commits and
raised five comments, all valid and fixed:
- `report_n100.py` prints the optimized-matrix conclusion only when every
  candidate covers every concurrency through 100 for each mode, size and
  query, with browse and search both present. A global check had allowed one
  candidate to be missing a concurrency.
- The native SQLite conclusions are now derived from the rows, not from the
  file being present. Every concurrency-200 trial must pair with the other
  candidate's trial for the same mode, size, query and repetition, and Rust
  must be lower in every pair for p95, CPU and RSS. At least one idle Go trial
  must exceed 150 ms, and every Rust trial must stay under 150 ms. The
  recorded data meets all of these: Rust is lower in 20 of 20 pairs on each
  metric, Go exceeds 150 ms in three idle trials, and the highest Rust p95 is
  89 ms.
- The reproduce text in `N100.md` no longer describes building and selecting
  the Go candidate, which the harness rejects. No published revision contains
  the Go prototypes, so the recorded Go rows cannot be rerun from this
  repository. Regenerating `N100.md` changes only that sentence.
- `bench_suite.py` waits for background requests before stopping the
  candidate after a completed load, so shutdown is no longer counted as a
  background error. On failure or cancellation it stops the candidate first,
  and a signal during the wait still stops and reaps every child.
- `lan_playback_worker.py` restarts its client wait after each background
  snapshot. The snapshot cap bounds the phase to at most 11 × 240 s idle or
  16 × 360 s with scans. Without a background workload the client sends only
  `done`, so that single budget is 420 s: the client's readiness, launch and
  per-repetition limits sum to 350 s, plus a 70 s margin for steps with no
  limit of their own.

Evidence: 39 Python tests pass. Each new behavioural test fails against the
commit before its fix. An independent review and two follow-up reviews found
nothing blocking, and their should-fix findings are included above. No
benchmark was rerun.

**CodeRabbit review of PR 5 (15 November 2026).** It raised seven comments
on the docs, all valid and fixed:
- README: `.env` with `OVL_MEDIA_DIR` is set up before the first Compose
  command, which otherwise fails interpolation. `OVL_SECURE_COOKIES` is fixed
  in `compose.yaml`, so local HTTP development now uses a Compose override
  file instead of a shell or `.env` value. `docker compose config` confirmed
  both behaviours.
- BENCHMARKS: the unrun comparison is the pinned-container one; the native
  Jellyfin and Rust diagnostics do not replace it. Four shell continuations
  in fenced blocks used a doubled backslash; both corrected `patch` commands
  were run and their outputs compile.
- HANDOFF gives the current stack state. PARITY limits mono downmix routing
  to DTS 5.1, as VALIDATION records. PLAYBACK no longer lists AAC HLS music
  conversion and artist browsing as missing.

An independent review found nothing blocking. Its should-fix findings were
fixed: the stack-state wording now says which numbering it uses, and this
file's date is current. It also led to a gitignore entry for the HTTP
Compose override.

## Outstanding from the plan

1. **Preview and video acceptance.** The user has no HDR content, so real HDR10, HLG and Dolby Vision acceptance is unavailable; HDR evidence stays synthetic. The queue-capture regression and the
   generated long-file seek matrix, surround downmix and legacy container
   corpora are done. Expand coverage for surround passthrough, subtitles and
   fonts, real-media and SMB seek latency, HDR/HLG and Dolby Vision. Verify
   decoded picture and audio. Broader Intel hardware paths, AMD/NVIDIA and
   real desktop/mobile device playback still need acceptance.
2. **Production benchmarks.** The user approved the N100 as the bench host,
   including during household use; label noisy results as such. OS-cold
   trials still need root page-cache control on a dedicated host. Never evict
   the shared N100's caches. A paired native playback-startup diagnostic
   now exists (`benchmarks/n100-native-playback-startup.py`). It times a
   validated warm-loopback direct range read and the CPU transcode first
   segment at 0 s and 60 s; see BENCHMARKS.md. Next:
   The interleaved v2 run with captured FFmpeg commands and verified segment
   durations is in BENCHMARKS.md. Next:
   - use real or high-entropy media so rate targets are exercised;
   - align rate control (Jellyfin derives its maxrate from the source);
   - split time with FFmpeg spawn time, FFmpeg's own CPU and segment close
     time against the response;
   - extend to remux, concurrency and LAN or browser first frame. Use the pinned Jellyfin 12.4 baseline and
   equivalent read-only media, metadata, output quality and background
   work. Complete broader search comparison, OS-cold trials on a dedicated
   host, sustained saturation, and mixed scanning/streaming. Compare LAN
   first decoded frame, seeks, stalls and direct/remux/transcode concurrency.
   Gates are browse/search p95 under 150 ms and direct first decoded frame
   under 2 s. Rust-only LAN and mixed-load diagnostics already exist.
   Record errors, source/binary identity, repeat uncertainty, server and
   FFmpeg resources separately. Profile before changing query semantics.
3. **Current deployment acceptance.** Current Linux arm64 (native) and
   amd64 (built and run under emulation) images now pass the container
   smoke. See "Current Linux images" in VALIDATION.md; the build cache was
   pruned with authorisation. The amd64 image's binary also passed a
   native smoke on the N100's x86 CPU. Container runs on the N100 need root or an admin-installed runtime,
   because the test account has no runtime and AppArmor blocks rootless user
   namespaces. GPU-in-container remains open. Complete isolated production-host Nginx/TLS, read-only mounts
   and cutover/rollback. The N100 test account cannot create a rootless user
   namespace, and no usable container runtime was found. Never use household media
   mounts; provision a separate read-only fixture mount. Never
   prune unrelated Docker images, caches or volumes.
4. **Subsequent parity.** Audit the inventory against pinned Jellyfin, then
   deliver remaining music and other media libraries, richer discovery and
   metadata, per-user preferences and parental controls, Live TV/IPTV and
   guide, DVR scheduling/conflicts/retention, DLNA/casting, remote and
   synchronized sessions, and first-party native clients with real-device
   acceptance. DVR needs a separate explicitly configured writable output.
   Plugin-equivalent capabilities are unselected. Obtain a product decision
   before committing to integrations that need credentials, accounts,
   provider terms or major scope choices. Continue independent core work.

## Verification and invariants

Use `make verify-local` with FFmpeg/FFprobe, installed Playwright engines
and a cached Nginx image. It includes npm clean install/check/build,
Rust format/Clippy/workspace tests, explicit CPU ignored tests, Python
regressions and browser workflows. For Rust changes use
`scripts/validate_n100.py` over SSH with the established unprivileged test
account, approved key and isolated workspace. See VALIDATION.md and
ROLLOUT.md for commands. Keep GPU, platform and benchmark evidence separate.

Keep media read-only. Missing or empty mounts must preserve records.
Resolve symlinks and enforce current grants on media, lists, artwork,
subtitles and sessions. Bound work and storage, kill and reap subprocesses,
and wait for tracked work on shutdown. Preserve SQLite data, bcrypt
credentials, grants, progress and backups. Change public types in
openapi.json and regenerate frontend types. No Go source or Go build
jobs belong in the Rust repository. Jellyfin API compatibility and loading
Jellyfin plugin binaries are outside scope. Follow AGENTS.md for independent
review and update STATUS, PARITY and VALIDATION when evidence changes.
