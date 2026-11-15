# ADR 001: Rust product backend

Accepted 9 November 2026 by product direction.

Use Rust, Axum/Tokio, a bounded SQLite owner thread and the existing Svelte UI.
Retain the native `/api/v1` contract. Existing Jellyfin client/API and plugin binary
compatibility are not goals. The repository contains no Go source or Go builds.
The prior application remains an external behavioral reference during migration.

This decision does not establish Rust as the benchmark winner. Historical Go/Rust
prototype results are retained as evidence with their original limitations. They
are not production parity, GPU certification or Jellyfin speedup evidence.

SQLite work is serialized on a dedicated OS thread, with 64 pending jobs maximum;
overload returns 503. Password hashing and media inspections each admit two jobs,
metadata providers two, scans one and FFmpeg sessions `OVL_MAX_STREAMS` (default 2).
Synchronous discovery runs outside Tokio; a 16-entry channel bounds backpressure,
16 directories may be open, and one library scan has a million-entry/30-minute
work budget. OS calls blocked inside an unavailable network filesystem cannot be
forcibly interrupted by these cooperative limits; use bounded SMB mount timeouts.

Database v1 migrates transactionally to v2. Collections/playlists/queues use
namespaced additive tables without rewriting the existing v2 schema or bumping
its version. This is deliberately compatible with format-1 SQLite backups and
legacy v2 readers. Queue updates use revision checks to prevent lost updates.

FFmpeg/FFprobe calls have deadlines, bounded stdout and bounded stderr tails.
A tracked owner task kills and reaps on timeout, cancellation or dropped HTTP
requests. Graceful shutdown waits for tracked work. FFmpeg uses two software
encoder/filter threads per rendition. SDR hardware candidates are smoke-tested;
failed hardware attempts retry software only before the master playlist exists.
HDR tone mapping defaults to software; subtitle burn-in remains software. Opt-in
VAAPI HDR10 admits only explicitly described limited-range PQ/BT.2020 HEVC with
successful frame metadata inspection. A bounded decoded-picture probe rejects
silent-black drivers before choosing hardware. The calibrated N100 slice passes
with an isolated full driver; the system free driver is rejected. See `HDR.md`.
This does not establish full HDR visual quality or GPU-in-container acceptance.

Performance targets remain N100/SMB browse/search p95 <150 ms and LAN direct first
frame <2 s. Additional playback budgets follow real measurements, without reducing
picture quality or benchmark semantics to manufacture a win.

Repeated broad unfiltered FTS queries can cache complete match IDs in a private
in-memory SQLite attachment. It admits eight terms, 100,000 matches per query,
200,000 total matches and 4,096 pages of 4,096 bytes (16 MiB database pages, plus
bounded SQLite bookkeeping/journal overhead). Sparse or filtered queries keep
their selective SQL plan. The first broad request records a term; only a repeat
within the same metadata generation fills it. Item insert/delete/title/series
updates invalidate membership; external commits are detected through data_version
within a pinned read snapshot. Grants, progress, ordering and pagination remain
live SQL. Over-cap or allocation-full fills fall back to complete uncached FTS;
there is no partial-match result. The cache is disposable and absent from backups.
