# Jellyfin capability and acceptance matrix

Target: Jellyfin **v12.4**, published 26 October 2026. GitHub release and Docker
manifest were checked directly on 9 November 2026; see `jellyfin-baseline.json`.
The immutable image is pinned in `bench/jellyfin.compose.yaml`. It has not been
pulled or benchmarked in this Rust migration.

Statuses describe the Rust application only: **verified** means the named local
automated scenario passed, **implemented** means code exists without that external
acceptance, **partial** means coverage is incomplete, **missing** means not built.
A locally verified slice does not imply parity with every Jellyfin behavior.

| Area | Rust status | Acceptance / remaining gap |
| --- | --- | --- |
| Setup, login, logout, cookies, password hashing, sessions | Verified slice | API tests: CSRF, account migration, session revocation; external review pending |
| Users and per-library access | Verified slice | API tests: viewer isolation and grant revocation; parental controls missing |
| Library scans and incremental updates | Verified slice | API tests: NFO import, existing metadata and unavailable-root preservation; filesystem watching missing |
| Movies/TV, series and seasons | Verified slice | Browse/search API tests and browser workflow; editions/extras/versions missing |
| Search and continue watching | Partial | FTS prefix search and per-user progress; richer discovery/filtering and full-scale acceptance pending |
| Local metadata/artwork, manual edits | Partial | NFO and edits tested; artwork routes implemented; broader corpus pending |
| Online metadata providers | Partial | Explicit TMDB match/cache implemented; provider integration and automated matching pending |
| Direct play and HTTP ranges | Verified slice | Byte-range/permissions tests and pixel-format-aware automatic negotiation; generated H.264 10-bit/4:4:4 conversion slice; profile/level/device corpus and LAN startup acceptance pending |
| Remux and audio track selection | Verified slice | Six-container/AAC/Opus/MP2/MP3/PCM generated software slice; generated AAC 5.1 to mono/stereo channel-tone slice and channel-count and optional sample-rate-ceiling negotiation; surround passthrough, layout/profile negotiation and full audio matrix missing; generated AC-3/E-AC-3/DTS/TrueHD 5.1 and FLAC/PCM/Opus/AAC 7.1 stereo/mono downmix routing and MPEG-1/2 PS/TS, WMV/ASF, FLV, Ogg Theora, AV1 and HEVC conversion corpus (`validation-n100-media-corpus.json`) |
| Single/adaptive SDR HLS and seeking | Verified slice | Real-media master/variants and browser transcoding; generated 10-minute B-frame MP4/Matroska/MPEG-TS and 45 s-GOP seek matrix verifies decoded frame and A/V beep alignment for remux and transcode (`validation-n100-keyframe-seek.json`); long playback/stalls, real-media and LAN seek latency pending |
| Text and burned-in subtitles | Verified slice | Embedded text, external SRT and generated PGS/DVD/DVB/XSUB burn-in (selection, clear, frequent-keyframe seek, single/adaptive; PGS/DVD/DVB SDR/PQ-black-base and XSUB opaque SDR); original embedded-font ASS geometry/style/seek/audio slice; extended XSUB alpha, long-file/SMB seek I/O/latency, SSA, broader fonts/effects/languages and timestamp corpus pending |
| HDR detection and software tone mapping | Verified slice | Tagged synthetic PQ and HLG HEVC converted to BT.709; Dolby Vision and visual quality corpus pending |
| Hardware SDR decode/scale/encode | Verified slice | N100 VAAPI H.264 synthetic graph: decoded SSIM and alternate audio tested; QSV/NVENC, HEVC, broader quality and sustained acceptance pending |
| Hardware HDR tone mapping | Verified slice | Opt-in VAAPI limited-range HDR10; N100 calibrated multi-frame picture/seek/audio/API tests with isolated full driver; installed black-output driver rejected. HLG/Dolby Vision, broader visual corpus, devices and container GPU acceptance pending; see HDR.md |
| Personal collections and ordered playlists | Verified slice | API ordering/ownership/conflicts plus browser persistence; shared/smart collections missing |
| Persistent playback queues | Verified slice | Atomic cursor conflicts, offline/revoked-entry preservation and restart tested on N100; twelve local Chrome/Firefox/WebKit cases passed, plus four full Chromium cases; delayed lifecycle operations covered; queue add keeps the clicked title during in-flight requests (`validation-browser-queue-repeats.json`); broader session coordination missing |
| Backup/restore and rollback | Verified slice | Live WAL snapshot preserves list expansion and revokes restored sessions; deployment drill pending |
| Desktop web | Partial | Automated browser results recorded separately; accessibility and larger codec/device matrix pending |
| Mobile web | Partial | Responsive UI exists; real iOS/Android acceptance pending |
| Native mobile/TV apps | Missing | Later clients; existing Jellyfin apps are explicitly unsupported |
| Music, albums/artists, lyrics, audio queues | Partial | Music scans, bounded embedded tag display, capability-checked direct audio and AAC HLS fallback, shared queues and sibling LRC/TXT and embedded MP3/FLAC/Ogg lyrics, and local plus embedded MP3/FLAC covers. Album and disc/track browsing from bounded tag indexing. A library-scoped album artist directory with album and all-track views. Broader artwork, broader embedded/enhanced lyrics and real-device audio acceptance remain open; see [Jellyfin music](https://jellyfin.org/docs/general/server/media/music/) |
| Books, comics, photos, music videos and other supported libraries | Missing | Inventory per pinned release and add media-specific clients/tests |
| Live TV/IPTV and guide data | Missing | Dedicated later stage: tuners, XMLTV, guide and playback tests |
| DVR, scheduling, conflicts, retention | Missing | Write only to a dedicated recording destination, never read-only library mounts |
| DLNA, casting and remote control | Missing | Discovery/device interoperability and current authorization tests |
| Session/device management, synchronized playback | Missing | Concurrent client lifecycle and permission tests |
| Parental rules, ratings and per-user preferences | Missing | Enforce on every browse, list and media path |
| Tasks, notifications, reporting and administration | Partial | Bounded scans and structured logs; richer scheduling/dashboard/reporting pending |
| Deployment and platform packaging | Implemented | Disposable N100 native Nginx/TLS, live restore and same-binary data rollback pass; Linux amd64/ARM64 CI smoke passed at 97bc011; retained-binary snapshot rollback passed on the N100; current images and production mount/TLS pending |
| Plugin-equivalent integrations | Missing | Explicit selection required before implementation; no .NET binary loading |

## Plugin capability candidates (unselected)

| Candidate | Intended capability | Selection/acceptance requirement |
| --- | --- | --- |
| OpenSubtitles | Optional external subtitle retrieval | Provider terms, bounded requests, no writes to media mounts |
| Intro/credit detection | Skip markers and playback navigation | Corpus accuracy, background CPU budget, UI controls |
| Trakt-style scrobbling | Optional external watched-state sync | Explicit connection, retries, conflict and privacy behavior |
| LDAP/OIDC authentication | External identity integration | Identity mapping, library permissions and session revocation |
| Playback reporting | Optional history/usage reporting | Retention, authorization and overhead budget |
| First-party DLNA capability | Device discovery and streaming | Own integration; protocol/device test matrix |
| Additional metadata providers | Matching/artwork alternatives | Provider selection, attribution and failure preservation |

These are capability candidates, not promises of compatibility with plugin
binaries. Core live TV and DVR are not counted as third-party plugin parity.

## Delivery gates

1. Rust preview: existing API/data preservation plus video/list slices; no parity claim.
2. Video-complete release: HDR/audio/subtitle corpus, queues, real desktop/mobile
   browsers, N100 performance and deployment acceptance.
3. Additional media and account/discovery features with per-feature budgets.
4. Live TV/DVR, DLNA/casting and session coordination.
5. Selected integrations and native device clients.

Do not mark full parity until the release-specific inventory is complete and every
required row has scenario evidence. Re-audit this matrix when the target Jellyfin
version changes. See the [official docs](https://jellyfin.org/docs/),
[live TV docs](https://jellyfin.org/docs/general/server/live-tv/) and
[plugin catalog documentation](https://jellyfin.org/docs/general/server/plugins/).

### Player lifecycle acceptance update (2026-11-10)

Local browser coverage now includes stale probe/progress/Apply/cursor responses
and deletion of obsolete HLS sessions after switching titles. Twelve scenarios
pass across Chrome, Firefox and WebKit. This strengthens personal queue/player
correctness; multi-client session coordination and real-device acceptance remain
open. Physical N100 SDR/HDR evidence is separately recorded in
`validation-n100-player.json`, without expanding the supported hardware scope.
