# Implementation status

Rust is the chosen product backend. This checkout is a Rust-only preview with a
Svelte client; it is not full Jellyfin parity or a stable release. See
[architecture decision](ADR-001-rust.md), [parity matrix](PARITY.md) and
[validation](VALIDATION.md). Performance superiority remains unproven.

## Implemented in Rust

- Cargo production workspace separate from the historical benchmark prototype;
  Axum/Tokio HTTP services and a bounded dedicated SQLite owner thread.
- SQLite WAL, transactional v1→v2 migration, existing v2 data preservation,
  exclusive data-directory locking and rejection of future database versions.
- Compatible bcrypt accounts, hashed/expiring sessions, cookie/bearer auth, CSRF
  headers, bounded hashing, login rate limits, administrators, viewers and grants.
- Indexed browse/search, series/seasons, manual metadata, watch/resume state and
  continue watching. Explicit bounded TMDB matching and optional artwork cache.
- Serialized incremental scans with NFO import and read-only media. No deletion
  during reconciliation; missing/empty mounts preserve records and mark offline.
- Permission checks and symlink containment on media access, artwork, subtitles,
  metadata operations, progress, list entries and playback session requests.
- Direct/range playback, remux, single/adaptive HLS, audio selection, text subtitles,
  subtitle burn-in, seek restart and explainable playback negotiation using client
  capability reports. HDR detection and bounded software tone mapping to BT.709.
- QSV/VAAPI/NVENC runtime encoder smoke selection, SDR hardware decode/scale/encode
  arguments and CPU retry before master publication. A short N100 VAAPI SDR graph passes synthetic frame/audio checks; other hardware paths remain unverified.
- Bounded process output, deadlines, kill/reap on cancellation, storage reservations,
  sampled workspace quotas and idle cleanup. Shutdown waits for tracked work.
- Personal collections, ordered playlists, persistent queue cursors, revision
  conflicts, atomic cursor-only updates, account ownership and current-access filtering; Svelte create/add/
  remove/reorder/resume controls and automatic next-item queue playback.
- Additive list tables preserve the v2 schema and existing backup format. Live WAL
  backup/verified fresh-directory restore preserve list data and revoke sessions.
- Structured HTTP response timing, database admission wait logs, scan duration and
  playlist preparation timing, plus untrusted client first-frame/seek/stall events.
- Rust-only Make targets, pinned toolchain, CI and multi-stage Docker packaging.

## Evidence and limits

Current test results are in `VALIDATION.md`. Real generated-media software tests
cover remux, adaptive HLS, alternate audio, embedded/external subtitles, burn-in
and a tagged synthetic HDR fixture converted to BT.709. They do not establish
HDR visual quality, Dolby Vision coverage, GPU performance or every media format.

The short authenticated production API smoke report records 3,600 requests with
zero errors on a shared macOS development host. It is neither N100/SMB acceptance
nor a Jellyfin comparison. Historical N100/VAAPI results describe the earlier
application/prototypes and must not be attributed to this Rust implementation.

The local Rust image build failed because Docker storage is full (`no space
left on device`); no unrelated images or volumes were deleted. GitHub CI later
built and smoke-tested both Linux amd64 and ARM64 images on native runners at
source `97bc011` (see `validation-linux-amd64.json` and
`validation-linux-arm64.json`). Production-host deployment acceptance remains
pending; these smoke checks do not certify full deployment or GPU behavior.

## Remaining acceptance and parity

- Real N100/SMB cold-cache, mixed/sustained load, LAN first-frame/seek/stall and
  concurrency measurements against the pinned Jellyfin baseline. Initial gates:
  browse/search p95 <150 ms and LAN direct first decoded frame <2 s.
- Broader Intel and physical AMD/NVIDIA validation, hardware HDR pipelines, Dolby Vision and broader audio/subtitle/font/container/visual-quality coverage;
  synthetic PQ/HLG and a six-container software slice now pass.
- Real mobile devices, additional desktop/browser codec coverage, accessibility,
  multi-client queue/session coordination and broader security/provider review.
- Production-host Nginx deployment and rollback; native amd64/ARM64 CI image
  smoke and a disposable real Nginx/TLS backup/restore drill pass for their
  recorded source snapshots. No registry publication occurred.
- Music/other media, richer discovery and user controls, live TV/DVR, DLNA/casting,
  selected plugin-equivalent integrations and own native clients remain subsequent
  stages. Recording output will require a dedicated writable destination.

No full-parity, Jellyfin speedup, certified stream-capacity or stable-release claim
has been established. Plugin selection and all later-stage acceptance are explicit
follow-up work, tracked in the parity matrix rather than silently dropped.

The continuation audit fixed filtered-queue entry loss and positional cursor
errors. Queue cursor updates preserve offline/revoked entries and survive restart;
concurrent updates are covered by a Linux N100 regression over SSH. See the
continuation evidence in `VALIDATION.md`; this does not complete the later parity
stages or production performance gates.

The N100 production API diagnostic now includes 80 trials / 80,000 requests and
17,643 errors at high offered concurrency. It is retained with its interference
limitations and is not release acceptance. Query plans motivated order-preserving
browse/broad-search changes. Their isolated N100 profile records 80,000 requests
and 10,148 errors at offered concurrency 100; 100k broad search still misses the
150 ms gate at concurrency 10/50. The bounded exact-match-ID cache's equivalent N100 run has zero errors through
concurrency 50, with 100k search p95 median 7.36 ms at concurrency 10 and 36.92 ms
at concurrency 50. Its concurrency-100 trials still produce 7,523 HTTP 503 errors.
These are warm loopback diagnostics, not complete production acceptance.
Startup now rejects symlinked owned storage and overlapping media/data paths.
Actual TCP cancellation is covered by stalled-upload and stalled-direct-reader
SIGTERM tests, alongside the expanded software container/PQ/HLG corpus.
