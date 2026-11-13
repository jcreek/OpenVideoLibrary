# OpenVideoLibrary migration handoff

Updated 12 November 2026 (evening session). This file records the handoff
for agents resuming work. Read AGENTS.md, STATUS.md, PARITY.md,
VALIDATION.md, BENCHMARKS.md, ROLLOUT.md, ADR-001-rust.md and openapi.json
before changing code or making acceptance claims.

## Checkout and publication

This repository is public. Keep host addresses, account names, key file
names and machine-specific paths out of committed files, and record them
only in the operator's private notes.

- Use the Rust checkout on the development machine. The older Go reference
  was archived outside the repository as a non-git snapshot with no handoff.
- With the user's authorization, branch `rust-migration` was pushed to `origin`.
  [Draft PR #1](https://github.com/jcreek/OpenVideoLibrary/pull/1) targets
  `main` and carries every commit, including the queue-capture increment.
  Confirm with `git status -sb` and `gh pr view 1`. The only workflow,
  `Verify Rust`, is manual-dispatch and disabled. Pushes spend no Actions
  minutes. Do not re-enable or dispatch it.
- On the N100, use the established unprivileged SSH test account and
  approved key. It runs Ubuntu 24.04 and uses strict host trust.
  `scripts/validate_n100.py` stages source into
  `<workspace>/source`. Pass `--vaapi-driver-path` with the isolated full
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

Four increments from this session are complete and on PR #1. The current
Linux images also pass the container smoke. With the user's approval,
history from the first handoff commit onward was rewritten to remove the
N100's private address and key filename, then force-pushed. PR #1's
commit IDs changed, but no file content did. Two local-only refs keep the
old history on the development machine:
`local-backup-before-history-scrub` and filter-branch's
`refs/original/refs/heads/rust-migration`. The first is a branch, so never
use `git push --all` or `--mirror`, and delete both once the user agrees.

Old commits may stay reachable on GitHub by commit ID, including from PR #1's
force-push event, until GitHub purges them. If the address must be
unrecoverable, the user needs to ask GitHub Support. Committed N100 evidence
still names the host (`plex`), the test account and its workspace paths.
These were outside the authorised scrub; the user decides whether to scrub
them.

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

## Outstanding from the plan

1. **Preview and video acceptance.** The queue-capture regression and the
   generated long-file seek matrix, surround downmix and legacy container
   corpora are done. Expand coverage for surround passthrough, subtitles and
   fonts, real-media and SMB seek latency, HDR/HLG and Dolby Vision. Verify
   decoded picture and audio. Broader Intel hardware paths, AMD/NVIDIA and
   real desktop/mobile device playback still need acceptance.
2. **Production benchmarks.** The user approved the N100 as the bench host,
   including during household use; label noisy results as such. OS-cold
   trials still need root page-cache control on a dedicated host. Never evict
   the shared N100's caches. Next, build a Jellyfin playback client harness
   for paired direct, remux and transcode first-segment and concurrency
   trials. Use the pinned Jellyfin 12.4 baseline and
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
   pruned with authorisation. Native x86 image runs and GPU-in-container
   remain open. Complete isolated production-host Nginx/TLS, read-only mounts
   and cutover/rollback. The N100 test account cannot create a rootless user
   namespace, and no usable container runtime was found. The existing SMB mount is writable. Leave
   it untouched and provision a separate read-only fixture mount. Never
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
