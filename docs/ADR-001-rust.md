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
HDR tone mapping and subtitle burn-in use software. Hardware HDR requires a
separate tested pipeline before it can be enabled.

Performance targets remain N100/SMB browse/search p95 <150 ms and LAN direct first
frame <2 s. Additional playback budgets follow real measurements, without reducing
picture quality or benchmark semantics to manufacture a win.
